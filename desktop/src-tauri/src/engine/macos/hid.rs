//! IOKit HID 两条通道（对应 Swift `DeviceWatcher.swift` / `HIDWatcher.swift`）：
//!
//! - `DeviceWatcher`：维护目标 HID 设备的 IOHIDEventService RegistryEntryID
//!   集合。CGEvent 的 senderID（field 87）就等于这个 ID，用来判断事件
//!   来自哪个物理设备 —— 只拦截遥控器，不影响键盘和其他外设。
//! - `HIDWatcher`：直接监听 HID element 值变化。某些按键的 usage 不被
//!   macOS 键盘驱动识别（小米遥控器返回键发 Keyboard usage 0xF1），永远
//!   不产生 CGEvent，CGEventTap 拦不到，这条通道绕开事件系统直读设备。
//!
//! 全部回调只发生在引擎线程的 CFRunLoop 上，单线程访问，用裸指针上下文。

use super::ffi;
use super::make_timer;
use crate::engine::log::Log;

use core_foundation::base::TCFType;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_foundation::string::CFString;
use core_foundation_sys::runloop as cf_rl_sys;
use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::{c_void, CString};

// MARK: - 工具

/// 读 IORegistry 整数属性
unsafe fn int_prop(svc: ffi::io_object_t, key: &str) -> Option<i64> {
    let key = CFString::new(key);
    let v = ffi::IORegistryEntryCreateCFProperty(
        svc,
        key.as_concrete_TypeRef(),
        std::ptr::null(),
        0,
    );
    if v.is_null() {
        return None;
    }
    let mut out: i64 = 0;
    let ok = ffi::CFNumberGetValue(
        v,
        ffi::K_CF_NUMBER_S_INT64_TYPE,
        &mut out as *mut i64 as *mut c_void,
    );
    ffi::CFRelease(v);
    ok.then_some(out)
}

/// 构造 IOHIDManager 的 VID/PID 匹配字典
/// （kIOHIDVendorIDKey/kIOHIDProductIDKey 是 CFSTR 宏，直接写字符串）
fn matching_dict(vendor_id: i32, product_id: i32) -> CFDictionary<CFString, CFNumber> {
    let k_vendor = CFString::new("VendorID");
    let k_product = CFString::new("ProductID");
    let v_vendor = CFNumber::from(vendor_id);
    let v_product = CFNumber::from(product_id);
    CFDictionary::from_CFType_pairs(&[(k_vendor, v_vendor), (k_product, v_product)])
}

/// 定时器上下文（DeviceWatcher 用）
pub enum WatcherTimer {
    /// 插拔后的延迟刷新（一次性）
    Refresh(*mut DeviceWatcher),
    /// 5s 兜底周期刷新
    Tick(*mut DeviceWatcher),
}

extern "C" fn watcher_timer_cb(_timer: cf_rl_sys::CFRunLoopTimerRef, info: *mut c_void) {
    if info.is_null() {
        return;
    }
    let ctx = unsafe { &*(info as *mut WatcherTimer) };
    match ctx {
        WatcherTimer::Refresh(w) => {
            let w = *w;
            unsafe {
                (*w).pending_refresh.borrow_mut().take();
                (*w).refresh();
            }
        }
        WatcherTimer::Tick(w) => {
            let w = *w;
            unsafe { (*w).refresh() };
        }
    }
}

// MARK: - DeviceWatcher

pub struct DeviceWatcher {
    vendor_id: i32,
    product_id: i32,
    ids: RefCell<HashSet<u64>>,
    manager: ffi::IOHIDManagerRef,
    /// 连接状态变化回调（true = 已连接）
    pub on_change: Option<Box<dyn FnMut(bool)>>,
    /// 插拔回调后的延迟刷新定时器（0.4s，等 IOHIDEventService 建好）
    pending_refresh: RefCell<Option<(cf_rl_sys::CFRunLoopTimerRef, *mut WatcherTimer)>>,
    /// 5s 兜底定时器（蓝牙设备偶尔不触发插拔回调）
    tick: RefCell<Option<(cf_rl_sys::CFRunLoopTimerRef, *mut WatcherTimer)>>,
}

impl DeviceWatcher {
    pub fn new(vendor_id: i32, product_id: i32) -> Box<Self> {
        Box::new(Self {
            vendor_id,
            product_id,
            ids: RefCell::new(HashSet::new()),
            manager: std::ptr::null_mut(),
            on_change: None,
            pending_refresh: RefCell::new(None),
            tick: RefCell::new(None),
        })
    }

    pub fn contains(&self, id: u64) -> bool {
        self.ids.borrow().contains(&id)
    }

    pub fn has_device(&self) -> bool {
        !self.ids.borrow().is_empty()
    }

    /// 立即扫一遍 IORegistry
    pub fn refresh(&self) {
        let mut found = HashSet::new();
        for class_name in ["IOHIDEventService", "IOHIDDevice", "AppleUserHIDEventService"] {
            let Ok(cname) = CString::new(class_name) else { continue };
            unsafe {
                let matching = ffi::IOServiceMatching(cname.as_ptr());
                if matching.is_null() {
                    continue;
                }
                let mut iter: ffi::io_iterator_t = 0;
                if ffi::IOServiceGetMatchingServices(
                    ffi::K_IO_MAIN_PORT_DEFAULT,
                    matching,
                    &mut iter,
                ) != ffi::KERN_SUCCESS
                {
                    continue;
                }
                let mut svc = ffi::IOIteratorNext(iter);
                while svc != 0 {
                    if int_prop(svc, "VendorID") == Some(self.vendor_id as i64)
                        && int_prop(svc, "ProductID") == Some(self.product_id as i64)
                    {
                        let mut rid: u64 = 0;
                        if ffi::IORegistryEntryGetRegistryEntryID(svc, &mut rid)
                            == ffi::KERN_SUCCESS
                        {
                            found.insert(rid);
                        }
                    }
                    ffi::IOObjectRelease(svc);
                    svc = ffi::IOIteratorNext(iter);
                }
                ffi::IOObjectRelease(iter);
            }
        }
        let changed = {
            let mut ids = self.ids.borrow_mut();
            if found != *ids {
                *ids = found;
                true
            } else {
                false
            }
        };
        if changed {
            let connected = self.has_device();
            Log::debug(&format!(
                "设备 registryID 更新: {:?}",
                self.ids.borrow()
            ));
            if let Some(cb) = &self.on_change {
                // 回调只会从引擎 runloop 触发，单线程安全
                let cb_ptr = cb as *const _ as *mut Box<dyn FnMut(bool)>;
                unsafe { (*cb_ptr)(connected) };
            }
        }
    }

    /// 订阅设备插拔 + 5s 兜底定时刷新
    pub fn start(&mut self) {
        self.refresh();

        unsafe {
            let mgr = ffi::IOHIDManagerCreate(std::ptr::null(), 0);
            self.manager = mgr;
            let this: *mut DeviceWatcher = self;

            let dict = matching_dict(self.vendor_id, self.product_id);
            ffi::IOHIDManagerSetDeviceMatching(mgr, dict.as_concrete_TypeRef());

            let ctx = this as *mut c_void;
            ffi::IOHIDManagerRegisterDeviceMatchingCallback(mgr, device_cb, ctx);
            ffi::IOHIDManagerRegisterDeviceRemovalCallback(mgr, device_cb, ctx);
            ffi::IOHIDManagerScheduleWithRunLoop(
                mgr,
                CFRunLoop::get_current().as_concrete_TypeRef(),
                kCFRunLoopCommonModes,
            );
            ffi::IOHIDManagerOpen(mgr, 0);

            // 5s 兜底周期刷新
            let tick_ctx = Box::into_raw(Box::new(WatcherTimer::Tick(this)));
            let tick_timer = make_timer(5.0, 5.0, watcher_timer_cb, tick_ctx as *mut c_void);
            *self.tick.borrow_mut() = Some((tick_timer, tick_ctx));
        }
    }

    /// 插拔回调：等 IOHIDEventService 建好再刷新（0.4s）
    fn schedule_refresh(&self) {
        if self.pending_refresh.borrow().is_some() {
            return;
        }
        let this = self as *const _ as *mut DeviceWatcher;
        let ctx = Box::into_raw(Box::new(WatcherTimer::Refresh(this)));
        let timer = make_timer(0.4, 0.0, watcher_timer_cb, ctx as *mut c_void);
        *self.pending_refresh.borrow_mut() = Some((timer, ctx));
    }
}

extern "C" fn device_cb(
    context: *mut c_void,
    _result: ffi::IOReturn,
    _sender: *mut c_void,
    _device: ffi::IOHIDDeviceRef,
) {
    if context.is_null() {
        return;
    }
    unsafe { (*(context as *mut DeviceWatcher)).schedule_refresh() };
}

impl Drop for DeviceWatcher {
    fn drop(&mut self) {
        for slot in [&self.pending_refresh, &self.tick] {
            if let Some((timer, ctx)) = slot.borrow_mut().take() {
                unsafe {
                    ffi::CFRunLoopTimerInvalidate(timer);
                    drop(Box::from_raw(ctx));
                }
            }
        }
        if !self.manager.is_null() {
            unsafe { ffi::IOHIDManagerClose(self.manager, 0) };
        }
    }
}

// MARK: - HIDWatcher

pub struct HIDWatcher {
    manager: ffi::IOHIDManagerRef,
    /// 已按下的 (page<<32|usage)，用于去重和过滤系统自动重复
    down: RefCell<HashSet<u64>>,
    /// 回调：(page, usage, isDown)
    pub on_value: Option<Box<dyn FnMut(u32, u32, bool)>>,
}

impl HIDWatcher {
    pub fn new() -> Box<Self> {
        Box::new(Self {
            manager: std::ptr::null_mut(),
            down: RefCell::new(HashSet::new()),
            on_value: None,
        })
    }

    pub fn start(&mut self, vendor_id: i32, product_id: i32) {
        unsafe {
            let mgr = ffi::IOHIDManagerCreate(std::ptr::null(), 0);
            self.manager = mgr;
            let this: *mut HIDWatcher = self;

            let dict = matching_dict(vendor_id, product_id);
            ffi::IOHIDManagerSetDeviceMatching(mgr, dict.as_concrete_TypeRef());
            ffi::IOHIDManagerRegisterInputValueCallback(mgr, value_cb, this as *mut c_void);
            ffi::IOHIDManagerScheduleWithRunLoop(
                mgr,
                CFRunLoop::get_current().as_concrete_TypeRef(),
                kCFRunLoopCommonModes,
            );
            let r = ffi::IOHIDManagerOpen(mgr, 0);
            if r != ffi::K_IO_RETURN_SUCCESS {
                Log::warn(&format!(
                    "HID element 通道打开失败 (0x{r:08x})，需要 系统设置 › 隐私与安全性 › 输入监控 授权"
                ));
            } else {
                Log::debug("HID element 通道已启动");
            }
        }
    }

    fn handle(&self, value: ffi::IOHIDValueRef) {
        unsafe {
            let el = ffi::IOHIDValueGetElement(value);
            let page = ffi::IOHIDElementGetUsagePage(el);
            let usage = ffi::IOHIDElementGetUsage(el);
            let v = ffi::IOHIDValueGetIntegerValue(value);

            // 过滤噪声：0xffffffff 是聚合/填充 element；
            // Keyboard 0x00(Reserved)/0x01(ErrorRollOver) 是状态位不是真按键
            if usage == 0xFFFF_FFFF {
                return;
            }
            if page == 0x07 && (usage == 0x00 || usage == 0x01) {
                return;
            }

            let key = ((page as u64) << 32) | usage as u64;
            let is_down = v != 0;
            {
                let mut down = self.down.borrow_mut();
                if is_down {
                    if down.contains(&key) {
                        return; // 系统自动重复
                    }
                    down.insert(key);
                } else if !down.remove(&key) {
                    return;
                }
            }
            if let Some(cb) = &self.on_value {
                let cb_ptr = cb as *const _ as *mut Box<dyn FnMut(u32, u32, bool)>;
                (*cb_ptr)(page, usage, is_down);
            }
        }
    }
}

extern "C" fn value_cb(
    context: *mut c_void,
    _result: ffi::IOReturn,
    _sender: *mut c_void,
    value: ffi::IOHIDValueRef,
) {
    if context.is_null() {
        return;
    }
    unsafe { (*(context as *mut HIDWatcher)).handle(value) };
}

impl Drop for HIDWatcher {
    fn drop(&mut self) {
        if !self.manager.is_null() {
            unsafe { ffi::IOHIDManagerClose(self.manager, 0) };
        }
    }
}
