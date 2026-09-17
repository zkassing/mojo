//! 文本上屏能力抽象 —— 语音链路的输出端。
//!
//! 各平台实现：
//! - macOS：`macos::emit::Emitter`（CGEvent Unicode 直输，支持 LiveTyping）
//! - Windows：`windows::emit::Emitter`（SendInput KEYEVENTF_UNICODE，支持 LiveTyping）
//! - Linux：`linux::emit::Emitter`（uinput 打不了 CJK，退化剪贴板粘贴，无 LiveTyping）

/// 上屏动作接口（语音输出工作线程持有，单线程串行调用）
pub trait TextOut: Send {
    /// 按 N 次退格
    fn press_backspace(&self, times: usize);
    /// 输入一段文本（macOS/Windows 走 Unicode 直输）
    fn type_text(&self, text: &str);
    /// 按一个键（可带修饰键），键名与 Emitter 键码表一致（如 "return"）
    fn send_key(&self, key: &str, mods: &[String]);
    /// 复制到系统剪贴板
    fn copy_clipboard(&self, text: &str);
}

/// 平台剪贴板实现的公共小工具
#[cfg(target_os = "macos")]
pub fn system_clipboard_copy(text: &str) {
    use std::io::Write;
    if let Ok(mut child) = std::process::Command::new("/usr/bin/pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
pub fn system_clipboard_copy(text: &str) {
    match arboard::Clipboard::new() {
        Ok(mut cb) => {
            if let Err(e) = cb.set_text(text) {
                crate::engine::log::Log::warn(&format!("写剪贴板失败: {e}"));
            }
        }
        Err(e) => crate::engine::log::Log::warn(&format!("剪贴板不可用: {e}")),
    }
}
