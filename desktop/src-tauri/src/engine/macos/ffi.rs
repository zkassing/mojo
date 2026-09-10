//! macOS 平台层的裸 FFI 声明。
//!
//! 伺服 core-graphics / core-foundation crate 未导出的部分（tap 创建、
//! 权限检测、IOHID、电源管理、hidsystem 私有符号）在这里手写声明，
//! 与 Swift 侧 `@_silgen_name` / IOKit C API 用法一一对应。

#![allow(non_snake_case, non_camel_case_types, dead_code)]

use core_foundation_sys::base::{CFAllocatorRef, CFTypeRef};
use core_foundation_sys::dictionary::{CFDictionaryRef, CFMutableDictionaryRef};
use core_foundation_sys::runloop::{CFRunLoopRef, CFRunLoopSourceRef, CFRunLoopTimerRef};
use core_foundation_sys::string::CFStringRef;
use std::ffi::{c_char, c_void};

// ---------- IOKit 基础类型 ----------

pub type IOReturn = i32;
pub type IOOptionBits = u32;
pub type io_service_t = u32;
pub type io_iterator_t = u32;
pub type io_connect_t = u32;
pub type io_object_t = u32;
pub type kern_return_t = i32;
pub type IONotificationPortRef = *mut c_void;
pub type IOHIDManagerRef = *mut c_void;
pub type IOHIDDeviceRef = *mut c_void;
pub type IOHIDValueRef = *mut c_void;
pub type IOHIDElementRef = *mut c_void;
pub type CFNumberRef = CFTypeRef;

pub const K_IO_MAIN_PORT_DEFAULT: u32 = 0;
pub const KERN_SUCCESS: kern_return_t = 0;
pub const K_IO_RETURN_SUCCESS: IOReturn = 0;

pub type IOHIDDeviceCallback = extern "C" fn(
    context: *mut c_void,
    result: IOReturn,
    sender: *mut c_void,
    device: IOHIDDeviceRef,
);
pub type IOHIDValueCallback = extern "C" fn(
    context: *mut c_void,
    result: IOReturn,
    sender: *mut c_void,
    value: IOHIDValueRef,
);
pub type IOServiceInterestCallback = extern "C" fn(
    refcon: *mut c_void,
    service: io_service_t,
    message_type: u32,
    message_argument: *mut c_void,
);

/// 与 C 的 IOGPoint 同布局（SInt16 x, y）
#[repr(C)]
pub struct NXIOGPoint {
    pub x: i16,
    pub y: i16,
}

/// NXEventData 中 key 成员的布局（见 IOLLEvent.h；总长 48 字节，尾部补零）
#[repr(C)]
pub struct NXKeyEventData {
    pub orig_char_set: u16,
    pub is_repeat: i16,
    pub char_set: u16,
    pub char_code: u16,
    pub key_code: u16,
    pub orig_char_code: u16,
    pub reserved1: i32,
    pub keyboard_type: u32,
    pub reserved2: i32,
    pub reserved3: i32,
    pub reserved4: [i32; 5],
}

impl NXKeyEventData {
    pub fn zeroed() -> Self {
        Self {
            orig_char_set: 0,
            is_repeat: 0,
            char_set: 0,
            char_code: 0,
            key_code: 0,
            orig_char_code: 0,
            reserved1: 0,
            keyboard_type: 0,
            reserved2: 0,
            reserved3: 0,
            reserved4: [0; 5],
        }
    }
}

pub type CGEventTapCallBack = extern "C" fn(
    proxy: *mut c_void,
    type_: u32,
    event: *mut c_void,
    user_info: *mut c_void,
) -> *mut c_void;

// 注意：kIOHIDVendorIDKey / kIOHIDProductIDKey 在 IOHIDKeys.h 里是
// CFSTR("VendorID") / CFSTR("ProductID") 宏，不是导出符号，直接用字符串构造。

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    // ---- IOHIDManager ----
    pub fn IOHIDManagerCreate(alloc: CFAllocatorRef, options: IOOptionBits) -> IOHIDManagerRef;
    pub fn IOHIDManagerSetDeviceMatching(mgr: IOHIDManagerRef, matching: CFDictionaryRef);
    pub fn IOHIDManagerRegisterDeviceMatchingCallback(
        mgr: IOHIDManagerRef,
        callback: IOHIDDeviceCallback,
        context: *mut c_void,
    );
    pub fn IOHIDManagerRegisterDeviceRemovalCallback(
        mgr: IOHIDManagerRef,
        callback: IOHIDDeviceCallback,
        context: *mut c_void,
    );
    pub fn IOHIDManagerRegisterInputValueCallback(
        mgr: IOHIDManagerRef,
        callback: IOHIDValueCallback,
        context: *mut c_void,
    );
    pub fn IOHIDManagerScheduleWithRunLoop(
        mgr: IOHIDManagerRef,
        run_loop: CFRunLoopRef,
        mode: CFStringRef,
    );
    pub fn IOHIDManagerOpen(mgr: IOHIDManagerRef, options: IOOptionBits) -> IOReturn;
    pub fn IOHIDManagerClose(mgr: IOHIDManagerRef, options: IOOptionBits) -> IOReturn;

    // ---- IOHIDValue / IOHIDElement ----
    pub fn IOHIDValueGetElement(value: IOHIDValueRef) -> IOHIDElementRef;
    pub fn IOHIDElementGetUsagePage(element: IOHIDElementRef) -> u32;
    pub fn IOHIDElementGetUsage(element: IOHIDElementRef) -> u32;
    pub fn IOHIDValueGetIntegerValue(value: IOHIDValueRef) -> isize;

    // ---- IORegistry ----
    pub fn IOServiceMatching(name: *const c_char) -> CFMutableDictionaryRef;
    pub fn IOServiceGetMatchingServices(
        main_port: u32,
        matching: CFDictionaryRef,
        existing: *mut io_iterator_t,
    ) -> kern_return_t;
    pub fn IOIteratorNext(iterator: io_iterator_t) -> io_object_t;
    pub fn IOObjectRelease(object: io_object_t) -> kern_return_t;
    pub fn IORegistryEntryGetRegistryEntryID(entry: io_object_t, id: *mut u64) -> kern_return_t;
    pub fn IORegistryEntryCreateCFProperty(
        entry: io_object_t,
        key: CFStringRef,
        allocator: CFAllocatorRef,
        options: IOOptionBits,
    ) -> CFTypeRef;

    // ---- hidsystem 私有符号（同 Swift @_silgen_name，符号在 IOKit.framework）----
    pub fn NXOpenEventStatus() -> io_connect_t;
    pub fn NXCloseEventStatus(handle: io_connect_t);
    pub fn IOHIDPostEvent(
        connect: io_connect_t,
        type_: u32,
        location: NXIOGPoint,
        data: *const NXKeyEventData,
        version: u32,
        flags: IOOptionBits,
        options: IOOptionBits,
    ) -> kern_return_t;
    pub fn IOHIDRequestAccess(request_type: i32) -> bool;

    // ---- pwr_mgt：电源键睡眠拦截 ----
    pub fn IORegisterForSystemPower(
        refcon: *mut c_void,
        the_port_ref: *mut IONotificationPortRef,
        callback: IOServiceInterestCallback,
        notifier: *mut io_object_t,
    ) -> io_connect_t;
    pub fn IONotificationPortGetRunLoopSource(notify: IONotificationPortRef)
        -> CFRunLoopSourceRef;
    pub fn IONotificationPortDestroy(notify: IONotificationPortRef);
    pub fn IODeregisterForSystemPower(notifier: *mut io_object_t) -> IOReturn;
    pub fn IOServiceClose(connect: io_connect_t) -> kern_return_t;
    pub fn IOCancelPowerChange(root_port: io_connect_t, notify_id: isize) -> IOReturn;
    pub fn IOAllowPowerChange(root_port: io_connect_t, notify_id: isize) -> IOReturn;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    // ---- 事件 tap（servo 未导出 SystemDefined 掩码，自建）----
    pub fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: CGEventTapCallBack,
        user_info: *mut c_void,
    ) -> *mut c_void; // CFMachPortRef
    pub fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    pub fn CGEventPost(tap_location: u32, event: *mut c_void);

    // ---- 辅助功能权限（TCC）----
    pub fn CGPreflightListenEventAccess() -> bool;
    pub fn CGRequestListenEventAccess() -> bool;

    // ---- 事件字段读取（tap 回调里用，不涉及所有权）----
    pub fn CGEventGetIntegerValueField(event: *mut c_void, field: u32) -> i64;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub fn CFMachPortCreateRunLoopSource(
        allocator: CFAllocatorRef,
        port: *mut c_void,
        order: isize,
    ) -> CFRunLoopSourceRef;
    pub fn CFMachPortInvalidate(port: *mut c_void);
    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFAbsoluteTimeGetCurrent() -> f64;
    pub fn CFNumberGetValue(
        number: CFNumberRef,
        the_type: i32,
        value_ptr: *mut c_void,
    ) -> bool;
    pub fn CFRunLoopTimerInvalidate(timer: CFRunLoopTimerRef);
}

/// kCFNumberSInt64Type
pub const K_CF_NUMBER_S_INT64_TYPE: i32 = 4;
