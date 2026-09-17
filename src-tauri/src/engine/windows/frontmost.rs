//! 前台 App 识别（对照 macos/frontmost.rs）。
//!
//! Windows 没有 bundle id 概念：取前台窗口进程映像名（exe 文件名去扩展名、
//! 小写）作为 app name，调用方以 `resolve_profile(None, Some(name))` 使用。

use windows::core::PWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// 返回 (bundle_id, app_name)：Windows 侧 bundle_id 恒为 None，
/// app_name = 前台进程 exe 文件名（去 .exe、小写）；任一步失败返回 (None, None)
/// （典型失败：前台是锁屏/UWP 宿主/以管理员运行而本进程未提权）
pub fn frontmost_app() -> (Option<String>, Option<String>) {
    // SAFETY: 全部在本线程栈上操作；进程句柄用完即关，不泄漏
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return (None, None);
        }
        let mut pid: u32 = 0;
        // .into() 兼容 Option<*mut u32> / *mut u32 两种签名
        GetWindowThreadProcessId(hwnd, (&mut pid as *mut u32).into());
        if pid == 0 {
            return (None, None);
        }
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return (None, None);
        };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(process);
        if !ok || len == 0 {
            return (None, None);
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        let file = full
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&full)
            .to_lowercase();
        let name = file
            .strip_suffix(".exe")
            .map(|s| s.to_string())
            .unwrap_or(file);
        if name.is_empty() {
            (None, None)
        } else {
            (None, Some(name))
        }
    }
}
