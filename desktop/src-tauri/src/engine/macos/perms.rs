//! macOS 隐私权限状态检测，用于面板提示用户去授权。
//!
//! - 辅助功能（Accessibility）：CGEventTap 拦截/注入按键需要
//! - 输入监控（Input Monitoring）：IOHIDManager 直读遥控器按键需要
//!   （back 键的 usage 0xF1 不产生 CGEvent，只能走这条通道）
//!
//! 注意：检测仅反映 TCC 授权，App 被重新构建/替换（ad-hoc 签名变化）后
//! 系统会把授权作废，列表里开关可能仍显示开着但实际已失效。

use super::ffi;
use serde::Serialize;

/// IOHIDRequestAccess / IOHIDCheckAccess 的设备类型
const K_IO_HID_DEVICE_TYPE_GENERIC: i32 = 0;

// IOHIDAccessStatus
const STATUS_NOT_DETERMINED: i32 = 0;
const STATUS_DENIED: i32 = 1;
const STATUS_ALLOWED: i32 = 2;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> u8;
}

/// 三态：明确有/明确没有/未知（旧系统未决定等），未知时前端不误报
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStatus {
    /// 辅助功能权限
    pub accessibility: Option<bool>,
    /// 输入监控权限
    pub input_monitoring: Option<bool>,
}

pub fn check() -> PermissionStatus {
    PermissionStatus {
        accessibility: Some(check_accessibility()),
        input_monitoring: check_input_monitoring(),
    }
}

fn check_accessibility() -> bool {
    // options 传 NULL：只查询、不触发系统弹窗/引导
    unsafe { AXIsProcessTrustedWithOptions(std::ptr::null()) != 0 }
}

fn check_input_monitoring() -> Option<bool> {
    // 仅查询当前授权状态，不调用 IOHIDRequestAccess（那会弹窗）
    match unsafe { ffi::IOHIDCheckAccess(K_IO_HID_DEVICE_TYPE_GENERIC) } {
        STATUS_ALLOWED => Some(true),
        STATUS_DENIED => Some(false),
        STATUS_NOT_DETERMINED => Some(false), // 还没申请过，等价于不可用
        _ => None,
    }
}

/// 打开对应的系统设置面板
pub fn open_settings(which: &str) {
    let pane = match which {
        "input" => "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent",
        "accessibility" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
        _ => "x-apple.systempreferences:com.apple.preference.security?Privacy",
    };
    let _ = std::process::Command::new("open").arg(pane).spawn();
}
