//! 电源键守卫（对应 Swift `PowerButtonGuard.swift`）。
//!
//! macOS 睡眠前向注册者广播 `kIOMessageCanSystemSleep`，这个阶段允许否决 ——
//! 调 `IOCancelPowerChange` 就把睡眠拦下来，把电源键还给映射表。
//! （后一个阶段 `kIOMessageSystemWillSleep` 只能延迟，不能拒绝。）
//!
//! 注意这会拦住**所有**来源的空闲/按键睡眠请求，包括 Mac 自带电源键。
//! 合盖睡眠、菜单栏「睡眠」、`pmset sleepnow` 走别的路径，不受影响。

use super::ffi;
use super::make_timer;
use crate::engine::log::Log;

use core_foundation::base::TCFType;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_foundation_sys::runloop as cf_rl_sys;
use std::cell::Cell;
use std::ffi::c_void;

/// IOMessage.h 里的常量
const CAN_SYSTEM_SLEEP: u32 = 0xE000_0270;
const SYSTEM_WILL_SLEEP: u32 = 0xE000_0280;

pub struct PowerButtonGuard {
    root_port: ffi::io_connect_t,
    notify_port: ffi::IONotificationPortRef,
    notifier: ffi::io_object_t,
    enabled: Cell<bool>,
    /// allowNextSleep 的 3s 恢复定时器
    restore_timer: Cell<Option<(cf_rl_sys::CFRunLoopTimerRef, *mut PowerButtonGuard)>>,
}

impl PowerButtonGuard {
    pub fn new() -> Box<Self> {
        Box::new(Self {
            root_port: 0,
            notify_port: std::ptr::null_mut(),
            notifier: 0,
            enabled: Cell::new(false),
            restore_timer: Cell::new(None),
        })
    }

    /// 开始拦截（幂等）
    pub fn start(&mut self) {
        if self.root_port != 0 {
            return;
        }
        let this: *mut PowerButtonGuard = self;
        unsafe {
            let mut port: ffi::IONotificationPortRef = std::ptr::null_mut();
            let mut notifier: ffi::io_object_t = 0;
            let root = ffi::IORegisterForSystemPower(
                this as *mut c_void,
                &mut port,
                power_cb,
                &mut notifier,
            );
            if root == 0 || port.is_null() {
                Log::warn("电源键守卫注册失败，电源键仍会触发系统睡眠");
                return;
            }
            self.root_port = root;
            self.notify_port = port;
            self.notifier = notifier;

            let src = ffi::IONotificationPortGetRunLoopSource(port);
            cf_rl_sys::CFRunLoopAddSource(
                CFRunLoop::get_current().as_concrete_TypeRef(),
                src,
                kCFRunLoopCommonModes,
            );
        }
        self.enabled.set(true);
        Log::info("电源键守卫已启动（拦截按键睡眠）");
    }

    /// 停止拦截，恢复系统默认行为
    pub fn stop(&mut self) {
        if self.root_port == 0 {
            return;
        }
        self.enabled.set(false);
        if let Some((timer, _)) = self.restore_timer.get_mut().take() {
            // ctx 是 guard 自身指针（非 Box），不能 from_raw
            unsafe { ffi::CFRunLoopTimerInvalidate(timer) };
        }
        unsafe {
            cf_rl_sys::CFRunLoopRemoveSource(
                CFRunLoop::get_current().as_concrete_TypeRef(),
                ffi::IONotificationPortGetRunLoopSource(self.notify_port),
                kCFRunLoopCommonModes,
            );
            let mut notifier = self.notifier;
            ffi::IODeregisterForSystemPower(&mut notifier);
            ffi::IOServiceClose(self.root_port);
            ffi::IONotificationPortDestroy(self.notify_port);
        }
        self.root_port = 0;
        self.notify_port = std::ptr::null_mut();
        Log::info("电源键守卫已停止");
    }

    pub fn is_started(&self) -> bool {
        self.root_port != 0
    }

    /// 临时放行一次睡眠（给「真的想睡」的动作用），3s 后自动恢复拦截
    #[allow(dead_code)]
    pub fn allow_next_sleep(&mut self) {
        self.enabled.set(false);
        let this = self as *mut PowerButtonGuard;
        // 清掉旧定时器（ctx 是自身指针，非 Box）
        if let Some((timer, _)) = self.restore_timer.get_mut().take() {
            unsafe { ffi::CFRunLoopTimerInvalidate(timer) };
        }
        let timer = make_timer(3.0, 0.0, restore_cb, this as *mut c_void);
        self.restore_timer.set(Some((timer, this)));
    }

    fn handle(&self, msg_type: u32, arg: *mut c_void) {
        match msg_type {
            CAN_SYSTEM_SLEEP => {
                if self.enabled.get() {
                    unsafe { ffi::IOCancelPowerChange(self.root_port, arg as isize) };
                    Log::debug("已拦截系统睡眠请求");
                } else {
                    unsafe { ffi::IOAllowPowerChange(self.root_port, arg as isize) };
                }
            }
            SYSTEM_WILL_SLEEP => {
                // 这阶段没法拒绝，放行即可
                unsafe { ffi::IOAllowPowerChange(self.root_port, arg as isize) };
            }
            _ => {}
        }
    }
}

extern "C" fn power_cb(
    refcon: *mut c_void,
    _service: ffi::io_service_t,
    msg_type: u32,
    msg_arg: *mut c_void,
) {
    if refcon.is_null() {
        return;
    }
    unsafe { (*(refcon as *mut PowerButtonGuard)).handle(msg_type, msg_arg) };
}

extern "C" fn restore_cb(_timer: cf_rl_sys::CFRunLoopTimerRef, info: *mut c_void) {
    if info.is_null() {
        return;
    }
    unsafe {
        let guard = &mut *(info as *mut PowerButtonGuard);
        guard.restore_timer.get_mut().take();
        guard.enabled.set(true);
    }
}

impl Drop for PowerButtonGuard {
    fn drop(&mut self) {
        self.stop();
    }
}
