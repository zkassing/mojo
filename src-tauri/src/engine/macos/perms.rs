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

/// IOHIDRequestType（IOHIDLib.h）
/// 输入监控对应 ListenEvent（PostEvent=0 是投递事件，不是这里要的）
const K_IO_HID_REQUEST_TYPE_LISTEN_EVENT: i32 = 1;

// IOHIDAccessType：IOHIDCheckAccess 的返回值
const K_IO_HID_ACCESS_TYPE_GRANTED: i32 = 0;
const K_IO_HID_ACCESS_TYPE_DENIED: i32 = 1;
const K_IO_HID_ACCESS_TYPE_UNKNOWN: i32 = 2;

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
    // 仅查询当前授权状态，不调用 IOHIDRequestAccess（那会弹窗）。
    // Granted=有；Denied=明确拒绝（报警告）；Unknown=尚未决定（不误报）。
    match unsafe { ffi::IOHIDCheckAccess(K_IO_HID_REQUEST_TYPE_LISTEN_EVENT) } {
        K_IO_HID_ACCESS_TYPE_GRANTED => Some(true),
        K_IO_HID_ACCESS_TYPE_DENIED => Some(false),
        K_IO_HID_ACCESS_TYPE_UNKNOWN => None,
        _ => None,
    }
}

/// 重置本 App 的 TCC 授权条目（辅助功能 + 输入监控）。
/// 未签名 App 每次构建 cdhash 都会变，更新后旧授权条目失效但设置里开关仍显示开着，
/// 必须删掉旧条目（重置）再重新授权才生效。
pub fn reset(bundle_id: &str) -> Result<(), String> {
    for service in ["Accessibility", "ListenEvent"] {
        let out = std::process::Command::new("tccutil")
            .args(["reset", service, bundle_id])
            .output()
            .map_err(|e| format!("运行 tccutil 失败: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "tccutil reset {service} 失败: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    Ok(())
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
