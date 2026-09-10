//! 事件注入：按键 / 媒体键 / 文本 / 鼠标 / shell / 打开目标（对照 macos/emit.rs）。
//!
//! 全部经 SendInput 注入：
//! - 普通按键：wVk + 按需 KEYEVENTF_EXTENDEDKEY（方向/导航/媒体键带扩展位）
//! - 文本：KEYEVENTF_UNICODE 按 UTF-16 单元直输（CJK/emoji 经代理对天然支持，
//!   对齐 macOS 的 CGEventKeyboardSetUnicodeString）
//! - 注入的事件自带 LLKHF_INJECTED 标志，会被引擎钩子跳过，不会形成死循环

use crate::engine::action::Act;
use crate::engine::log::Log;

use windows::core::{w, PCWSTR};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, MOUSE_EVENT_FLAGS,
    VIRTUAL_KEY,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

// MARK: - 按键名 -> VK 码（键名口径与 macOS KeyMap.table 对齐，值为 Win32 VK_*）

static KEY_TABLE: &[(&str, u16)] = &[
    // 控制键
    ("backspace", 0x08),
    ("delete", 0x08), // 与 macOS 一致：delete = 大退格键
    ("tab", 0x09),
    ("return", 0x0D),
    ("enter", 0x0D),
    ("keypadenter", 0x0D),
    ("shift", 0x10),
    ("ctrl", 0x11),
    ("control", 0x11),
    ("alt", 0x12),
    ("opt", 0x12),
    ("option", 0x12),
    ("capslock", 0x14),
    ("escape", 0x1B),
    ("esc", 0x1B),
    ("space", 0x20),
    ("pageup", 0x21),
    ("pagedown", 0x22),
    ("end", 0x23),
    ("home", 0x24),
    ("left", 0x25),
    ("up", 0x26),
    ("right", 0x27),
    ("down", 0x28),
    ("printscreen", 0x2C),
    ("snapshot", 0x2C),
    ("insert", 0x2D),
    ("forwarddelete", 0x2E),
    ("del", 0x2E), // macOS 的 del(117) = 前向删除
    // 数字 0-9
    ("0", 0x30),
    ("1", 0x31),
    ("2", 0x32),
    ("3", 0x33),
    ("4", 0x34),
    ("5", 0x35),
    ("6", 0x36),
    ("7", 0x37),
    ("8", 0x38),
    ("9", 0x39),
    // 字母 a-z
    ("a", 0x41),
    ("b", 0x42),
    ("c", 0x43),
    ("d", 0x44),
    ("e", 0x45),
    ("f", 0x46),
    ("g", 0x47),
    ("h", 0x48),
    ("i", 0x49),
    ("j", 0x4A),
    ("k", 0x4B),
    ("l", 0x4C),
    ("m", 0x4D),
    ("n", 0x4E),
    ("o", 0x4F),
    ("p", 0x50),
    ("q", 0x51),
    ("r", 0x52),
    ("s", 0x53),
    ("t", 0x54),
    ("u", 0x55),
    ("v", 0x56),
    ("w", 0x57),
    ("x", 0x58),
    ("y", 0x59),
    ("z", 0x5A),
    // Win 键（cmd 口径对齐 macOS：cmd → 系统键）
    ("cmd", 0x5B),
    ("command", 0x5B),
    ("meta", 0x5B),
    ("super", 0x5B),
    ("win", 0x5B),
    ("lwin", 0x5B),
    ("rwin", 0x5C),
    ("apps", 0x5D),
    ("menu", 0x5D), // 上下文菜单键
    // 小键盘
    ("keypad0", 0x60),
    ("keypad1", 0x61),
    ("keypad2", 0x62),
    ("keypad3", 0x63),
    ("keypad4", 0x64),
    ("keypad5", 0x65),
    ("keypad6", 0x66),
    ("keypad7", 0x67),
    ("keypad8", 0x68),
    ("keypad9", 0x69),
    ("keypadmultiply", 0x6A),
    ("keypadplus", 0x6B),
    ("keypadminus", 0x6D),
    ("keypaddecimal", 0x6E),
    ("keypaddivide", 0x6F),
    // F1-F24
    ("f1", 0x70),
    ("f2", 0x71),
    ("f3", 0x72),
    ("f4", 0x73),
    ("f5", 0x74),
    ("f6", 0x75),
    ("f7", 0x76),
    ("f8", 0x77),
    ("f9", 0x78),
    ("f10", 0x79),
    ("f11", 0x7A),
    ("f12", 0x7B),
    ("f13", 0x7C),
    ("f14", 0x7D),
    ("f15", 0x7E),
    ("f16", 0x7F),
    ("f17", 0x80),
    ("f18", 0x81),
    ("f19", 0x82),
    ("f20", 0x83),
    ("f21", 0x84),
    ("f22", 0x85),
    ("f23", 0x86),
    ("f24", 0x87),
    ("numlock", 0x90),
    ("scrolllock", 0x91),
    // 音量 / 媒体（作为普通键也能直接用；Act::Media 走下面的 MEDIA_TABLE）
    ("volumemute", 0xAD),
    ("volumedown", 0xAE),
    ("volumeup", 0xAF),
    ("medianexttrack", 0xB0),
    ("mediaprevioustrack", 0xB1),
    ("mediastop", 0xB2),
    ("mediaplaypause", 0xB3),
    // 常见标点（美式布局 OEM 键）
    (";", 0xBA),
    ("=", 0xBB),
    (",", 0xBC),
    ("-", 0xBD),
    (".", 0xBE),
    ("/", 0xBF),
    ("`", 0xC0),
    ("[", 0xDB),
    ("\\", 0xDC),
    ("]", 0xDD),
    ("'", 0xDE),
];

pub fn key_code(name: &str) -> Option<u16> {
    let low = name.to_lowercase();
    KEY_TABLE.iter().find(|(n, _)| *n == low).map(|(_, c)| *c)
}

/// 修饰键名 → VK（cmd→VK_LWIN，口径对齐任务约定）
fn modifier_vk(name: &str) -> Option<u16> {
    match name.to_lowercase().as_str() {
        "cmd" | "command" | "meta" | "super" | "win" => Some(0x5B), // VK_LWIN
        "ctrl" | "control" => Some(0x11),                           // VK_CONTROL
        "alt" | "opt" | "option" => Some(0x12),                     // VK_MENU
        "shift" => Some(0x10),                                      // VK_SHIFT
        _ => None,
    }
}

/// 需要 KEYEVENTF_EXTENDEDKEY 标志的键（与真实键盘扫描码的扩展位对齐：
/// 方向/导航/插入/前向删除/Win/菜单键/小键盘除号/NumLock/音量媒体键）
fn is_extended(vk: u16) -> bool {
    matches!(vk,
        0x21..=0x28           // pageup/pagedown/end/home/方向键
        | 0x2D | 0x2E         // insert / 前向删除
        | 0x5B | 0x5C | 0x5D  // LWIN / RWIN / APPS
        | 0x6F                // 小键盘除号
        | 0x90                // numlock
        | 0xAD..=0xB3         // 音量/媒体键
    )
}

// MARK: - 媒体键名 -> VK（与 macOS MEDIA_TABLE 的键名口径对齐）

static MEDIA_TABLE: &[(&str, u16)] = &[
    ("volup", 0xAF),
    ("volumeup", 0xAF),
    ("voldown", 0xAE),
    ("volumedown", 0xAE),
    ("mute", 0xAD),
    ("volumemute", 0xAD),
    ("playpause", 0xB3),
    ("play", 0xB3),
    ("pause", 0xB3),
    ("stop", 0xB2),
    ("next", 0xB0),
    ("nexttrack", 0xB0),
    ("previous", 0xB1),
    ("prev", 0xB1),
    ("prevtrack", 0xB1),
    // macOS 的 brightnessup/down、eject、illumination* 在 Windows 无标准 VK，忽略
];

// MARK: - SendInput 封装

fn key_input(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse_input(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_inputs(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    // SAFETY: inputs 切片在调用期间有效；SendInput 同步消费，不保留指针
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        Log::warn("SendInput 部分事件被系统拦截（目标进程完整性级别可能更高）");
    }
}

// MARK: - Emitter

/// 无状态注入器：SendInput 本身线程安全，语音输出线程与引擎线程各持一份
pub struct Emitter;

impl Emitter {
    pub fn new() -> Self {
        Self
    }

    pub fn perform(&self, action: &Act) {
        match action {
            Act::Key { key, mods } => self.send_key(key, mods),
            Act::Media { key } => self.send_media(key),
            Act::Shell { command } => run_shell(command),
            Act::Open { target } => open_target(target),
            Act::MouseMove { dx, dy } => self.move_mouse(*dx, *dy),
            Act::MouseClick { button, count } => self.click_mouse(button, *count),
            Act::MouseScroll { dx, dy } => self.scroll(*dx as i32, *dy as i32),
            Act::Sequence(list) => {
                for a in list {
                    self.perform(a);
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
            }
            Act::None | Act::Passthrough | Act::Dictate => {
                // dictate 由引擎的按住/松开钩子处理，不在这里发事件
            }
        }
    }

    // MARK: 键盘

    fn send_vk(&self, vk: u16, down: bool) {
        let mut flags = if down {
            KEYBD_EVENT_FLAGS(0)
        } else {
            KEYEVENTF_KEYUP
        };
        if is_extended(vk) {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        send_inputs(&[key_input(vk, 0, flags)]);
    }

    pub fn send_key(&self, key: &str, mods: &[String]) {
        let Some(code) = key_code(key) else {
            Log::warn(&format!("未知按键名: {key}"));
            return;
        };
        let mut mod_vks = Vec::with_capacity(mods.len());
        for m in mods {
            match modifier_vk(m) {
                Some(v) => mod_vks.push(v),
                None => Log::warn(&format!("未知修饰键: {m}")),
            }
        }
        // 修饰键按下 → 主键点按 → 修饰键逆序抬起（顺序与 macOS 一致）
        for &v in &mod_vks {
            self.send_vk(v, true);
        }
        self.send_vk(code, true);
        std::thread::sleep(std::time::Duration::from_millis(8));
        self.send_vk(code, false);
        for &v in mod_vks.iter().rev() {
            self.send_vk(v, false);
        }
    }

    // MARK: 媒体键

    pub fn send_media(&self, key: &str) {
        let low = key.to_lowercase();
        let Some(vk) = MEDIA_TABLE.iter().find(|(n, _)| *n == low).map(|(_, c)| *c) else {
            Log::warn(&format!("未知媒体键: {key}"));
            return;
        };
        self.send_vk(vk, true);
        std::thread::sleep(std::time::Duration::from_millis(8));
        self.send_vk(vk, false);
    }

    // MARK: 文本输入（LiveTyper / 语音结果用）

    /// 输入一段文本（Unicode 直输，不经键码；UTF-16 单元逐一下上，
    /// 代理对自然拼出 CJK/emoji）
    pub fn type_text(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut inputs: Vec<INPUT> = Vec::with_capacity(text.len() * 2);
        for unit in text.encode_utf16() {
            inputs.push(key_input(0, unit, KEYEVENTF_UNICODE));
            inputs.push(key_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
        // 分段注入，避免一次塞爆系统输入队列
        for chunk in inputs.chunks(64) {
            send_inputs(chunk);
        }
    }

    /// 连按 n 次退格（撤回已上屏的中间结果）
    pub fn press_backspace(&self, times: usize) {
        for _ in 0..times {
            self.send_key("delete", &[]);
        }
    }

    pub fn copy_clipboard_text(&self, text: &str) {
        crate::engine::textout::system_clipboard_copy(text)
    }

    // MARK: 鼠标

    fn move_mouse(&self, dx: f64, dy: f64) {
        // 不带 ABSOLUTE 标志的 MOUSEEVENTF_MOVE = 相对位移
        send_inputs(&[mouse_input(
            dx.round() as i32,
            dy.round() as i32,
            0,
            MOUSEEVENTF_MOVE,
        )]);
    }

    fn click_mouse(&self, button: &str, count: i64) {
        let (down, up) = match button.to_lowercase().as_str() {
            "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            "middle" | "center" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
            _ => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        };
        let n = count.max(1);
        for i in 1..=n {
            send_inputs(&[mouse_input(0, 0, 0, down), mouse_input(0, 0, 0, up)]);
            if i < n {
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
        }
    }

    fn scroll(&self, dx: i32, dy: i32) {
        const WHEEL_DELTA: i32 = 120; // 一格滚轮
        let mut inputs = Vec::with_capacity(2);
        if dy != 0 {
            // 正数 = 向上滚（与 macOS 方向一致）
            inputs.push(mouse_input(
                0,
                0,
                (dy * WHEEL_DELTA) as u32,
                MOUSEEVENTF_WHEEL,
            ));
        }
        if dx != 0 {
            inputs.push(mouse_input(
                0,
                0,
                (dx * WHEEL_DELTA) as u32,
                MOUSEEVENTF_HWHEEL,
            ));
        }
        send_inputs(&inputs);
    }
}

impl crate::engine::textout::TextOut for Emitter {
    fn press_backspace(&self, times: usize) {
        Emitter::press_backspace(self, times)
    }

    fn type_text(&self, text: &str) {
        Emitter::type_text(self, text)
    }

    fn send_key(&self, key: &str, mods: &[String]) {
        Emitter::send_key(self, key, mods)
    }

    fn copy_clipboard(&self, text: &str) {
        crate::engine::textout::system_clipboard_copy(text)
    }
}

// MARK: - shell / open

fn run_shell(cmd: &str) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000; // 后台执行不弹控制台窗口
    let r = std::process::Command::new("cmd")
        .args(["/c", cmd])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Err(e) = r {
        Log::warn(&format!("shell 执行失败: {e}"));
    }
}

fn open_target(target: &str) {
    let t = target.trim();
    if t.is_empty() {
        return;
    }
    let wide: Vec<u16> = t.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: wide 以 NUL 结尾且在调用期间存活；ShellExecuteW 同步返回不保留指针。
    // 指针参数用 .into() 传递，兼容 windows crate 各版本 Option<PCWSTR>/PCWSTR 两种签名。
    let r = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // 返回值 <= 32 表示失败（ShellExecute 的历史约定）
    if (r.0 as usize) <= 32 {
        Log::warn(&format!("打开失败: {t}"));
    }
}
