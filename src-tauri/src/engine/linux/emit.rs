//! 事件注入：uinput 虚拟设备（按键 / 相对轴）+ 剪贴板文本（对照 macos/emit.rs）。
//!
//! - 按键：自建 uinput 虚拟键盘；`VirtualDevice::emit` 每批自动补 SYN_REPORT
//! - 文本：uinput 只能发键码，**无法直接输出 Unicode**。ASCII 可打印字符按
//!   US 布局键码直打（含 Shift 组合）；含任何非 ASCII → 整体走剪贴板粘贴
//!   （arboard 写入 + Ctrl+V）。
//!   取舍：Linux 下语音上屏建议 liveTyping=false（中间结果反复改剪贴板体验差），
//!   首次遇到非 ASCII 文本时日志提醒一次。
//! - 剪贴板在 X11 下由写入方进程托管内容，故 Clipboard 实例随 Emitter 常驻
//!   （drop 后内容可能立即失效）；纯 Wayland 会话 arboard 默认特性（仅 X11 后端）
//!   不可用，会告警退化（后续可考虑开 wayland-data-control 特性）。

use crate::engine::action::Act;
use crate::engine::log::Log;

use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{AttributeSet, EventType, InputEvent, Key, RelativeAxisType};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

// MARK: - 按键名 -> evdev Key（键名口径与 macOS KeyMap.table 对齐）

static KEY_TABLE: &[(&str, Key)] = &[
    // 控制键
    ("return", Key::KEY_ENTER),
    ("enter", Key::KEY_ENTER),
    ("tab", Key::KEY_TAB),
    ("esc", Key::KEY_ESC),
    ("escape", Key::KEY_ESC),
    ("space", Key::KEY_SPACE),
    ("backspace", Key::KEY_BACKSPACE),
    // 与 macOS 一致："delete" 指大退格键（macOS vk 51），"forwarddelete"/"del" 才是 DEL
    ("delete", Key::KEY_BACKSPACE),
    ("forwarddelete", Key::KEY_DELETE),
    ("del", Key::KEY_DELETE),
    ("insert", Key::KEY_INSERT),
    ("home", Key::KEY_HOME),
    ("end", Key::KEY_END),
    ("pageup", Key::KEY_PAGEUP),
    ("pagedown", Key::KEY_PAGEDOWN),
    ("up", Key::KEY_UP),
    ("down", Key::KEY_DOWN),
    ("left", Key::KEY_LEFT),
    ("right", Key::KEY_RIGHT),
    ("capslock", Key::KEY_CAPSLOCK),
    ("fn", Key::KEY_FN),
    ("printscreen", Key::KEY_SYSRQ),
    ("snapshot", Key::KEY_SYSRQ),
    ("scrolllock", Key::KEY_SCROLLLOCK),
    ("menu", Key::KEY_MENU),
    ("apps", Key::KEY_MENU),
    ("numlock", Key::KEY_NUMLOCK),
    ("keypadclear", Key::KEY_NUMLOCK),
    // 修饰键也可作为主键单独点按；口径：cmd → 左 Meta（Super/Win 键）
    ("cmd", Key::KEY_LEFTMETA),
    ("command", Key::KEY_LEFTMETA),
    ("meta", Key::KEY_LEFTMETA),
    ("super", Key::KEY_LEFTMETA),
    ("win", Key::KEY_LEFTMETA),
    ("ctrl", Key::KEY_LEFTCTRL),
    ("control", Key::KEY_LEFTCTRL),
    ("alt", Key::KEY_LEFTALT),
    ("opt", Key::KEY_LEFTALT),
    ("option", Key::KEY_LEFTALT),
    ("shift", Key::KEY_LEFTSHIFT),
    // 功能键 f1-f24
    ("f1", Key::KEY_F1),
    ("f2", Key::KEY_F2),
    ("f3", Key::KEY_F3),
    ("f4", Key::KEY_F4),
    ("f5", Key::KEY_F5),
    ("f6", Key::KEY_F6),
    ("f7", Key::KEY_F7),
    ("f8", Key::KEY_F8),
    ("f9", Key::KEY_F9),
    ("f10", Key::KEY_F10),
    ("f11", Key::KEY_F11),
    ("f12", Key::KEY_F12),
    ("f13", Key::KEY_F13),
    ("f14", Key::KEY_F14),
    ("f15", Key::KEY_F15),
    ("f16", Key::KEY_F16),
    ("f17", Key::KEY_F17),
    ("f18", Key::KEY_F18),
    ("f19", Key::KEY_F19),
    ("f20", Key::KEY_F20),
    ("f21", Key::KEY_F21),
    ("f22", Key::KEY_F22),
    ("f23", Key::KEY_F23),
    ("f24", Key::KEY_F24),
    // 数字（主键盘区）
    ("1", Key::KEY_1),
    ("2", Key::KEY_2),
    ("3", Key::KEY_3),
    ("4", Key::KEY_4),
    ("5", Key::KEY_5),
    ("6", Key::KEY_6),
    ("7", Key::KEY_7),
    ("8", Key::KEY_8),
    ("9", Key::KEY_9),
    ("0", Key::KEY_0),
    // 字母
    ("a", Key::KEY_A),
    ("b", Key::KEY_B),
    ("c", Key::KEY_C),
    ("d", Key::KEY_D),
    ("e", Key::KEY_E),
    ("f", Key::KEY_F),
    ("g", Key::KEY_G),
    ("h", Key::KEY_H),
    ("i", Key::KEY_I),
    ("j", Key::KEY_J),
    ("k", Key::KEY_K),
    ("l", Key::KEY_L),
    ("m", Key::KEY_M),
    ("n", Key::KEY_N),
    ("o", Key::KEY_O),
    ("p", Key::KEY_P),
    ("q", Key::KEY_Q),
    ("r", Key::KEY_R),
    ("s", Key::KEY_S),
    ("t", Key::KEY_T),
    ("u", Key::KEY_U),
    ("v", Key::KEY_V),
    ("w", Key::KEY_W),
    ("x", Key::KEY_X),
    ("y", Key::KEY_Y),
    ("z", Key::KEY_Z),
    // 标点（US 布局）
    ("-", Key::KEY_MINUS),
    ("=", Key::KEY_EQUAL),
    ("[", Key::KEY_LEFTBRACE),
    ("]", Key::KEY_RIGHTBRACE),
    ("\\", Key::KEY_BACKSLASH),
    (";", Key::KEY_SEMICOLON),
    ("'", Key::KEY_APOSTROPHE),
    ("`", Key::KEY_GRAVE),
    (",", Key::KEY_COMMA),
    (".", Key::KEY_DOT),
    ("/", Key::KEY_SLASH),
    // 小键盘
    ("keypad0", Key::KEY_KP0),
    ("keypad1", Key::KEY_KP1),
    ("keypad2", Key::KEY_KP2),
    ("keypad3", Key::KEY_KP3),
    ("keypad4", Key::KEY_KP4),
    ("keypad5", Key::KEY_KP5),
    ("keypad6", Key::KEY_KP6),
    ("keypad7", Key::KEY_KP7),
    ("keypad8", Key::KEY_KP8),
    ("keypad9", Key::KEY_KP9),
    ("keypadplus", Key::KEY_KPPLUS),
    ("keypadminus", Key::KEY_KPMINUS),
    ("keypadmultiply", Key::KEY_KPASTERISK),
    ("keypaddivide", Key::KEY_KPSLASH),
    ("keypaddecimal", Key::KEY_KPDOT),
    ("keypadenter", Key::KEY_KPENTER),
];

pub fn key_code(name: &str) -> Option<Key> {
    let low = name.to_lowercase();
    KEY_TABLE.iter().find(|(n, _)| *n == low).map(|(_, c)| *c)
}

// MARK: - 媒体键（Linux 下媒体键就是普通 EV_KEY 键码）

static MEDIA_TABLE: &[(&str, Key)] = &[
    ("volup", Key::KEY_VOLUMEUP),
    ("volumeup", Key::KEY_VOLUMEUP),
    ("voldown", Key::KEY_VOLUMEDOWN),
    ("volumedown", Key::KEY_VOLUMEDOWN),
    ("mute", Key::KEY_MUTE),
    ("micmute", Key::KEY_MICMUTE),
    // play / pause 与 macOS 口径一致：统一映射到播放暂停切换键
    // （KEY_PLAYCD/KEY_PAUSECD 桌面环境支持度远不如 KEY_PLAYPAUSE）
    ("playpause", Key::KEY_PLAYPAUSE),
    ("play", Key::KEY_PLAYPAUSE),
    ("pause", Key::KEY_PLAYPAUSE),
    ("next", Key::KEY_NEXTSONG),
    ("nexttrack", Key::KEY_NEXTSONG),
    ("previous", Key::KEY_PREVIOUSSONG),
    ("prev", Key::KEY_PREVIOUSSONG),
    ("prevtrack", Key::KEY_PREVIOUSSONG),
    ("fast", Key::KEY_FASTFORWARD),
    ("forward", Key::KEY_FASTFORWARD),
    ("fastforward", Key::KEY_FASTFORWARD),
    ("rewind", Key::KEY_REWIND),
    ("stop", Key::KEY_STOPCD),
    ("eject", Key::KEY_EJECTCD),
    ("brightnessup", Key::KEY_BRIGHTNESSUP),
    ("brightnessdown", Key::KEY_BRIGHTNESSDOWN),
    ("homepage", Key::KEY_HOMEPAGE),
];

pub fn media_code(name: &str) -> Option<Key> {
    let low = name.to_lowercase();
    MEDIA_TABLE.iter().find(|(n, _)| *n == low).map(|(_, c)| *c)
}

/// 修饰键名 → 键码（全部用左键，口径与 macOS flags_of 一致）
fn mod_code(name: &str) -> Option<Key> {
    match name.to_lowercase().as_str() {
        "cmd" | "command" | "meta" | "super" | "win" => Some(Key::KEY_LEFTMETA),
        "ctrl" | "control" => Some(Key::KEY_LEFTCTRL),
        "alt" | "opt" | "option" => Some(Key::KEY_LEFTALT),
        "shift" => Some(Key::KEY_LEFTSHIFT),
        _ => None,
    }
}

// MARK: - Emitter

pub struct Emitter {
    /// uinput 虚拟键盘（含相对轴）。创建失败时退化为全部 no-op + 告警，
    /// 保证引擎/语音链路不崩（启动时的权限预检通常已拦截这种情况）。
    dev: Option<Mutex<VirtualDevice>>,
    /// 常驻剪贴板句柄（见文件头注释：X11 下内容随进程内实例存活）
    clipboard: Mutex<Option<arboard::Clipboard>>,
}

// 编译期断言：Emitter 必须可跨线程移动（语音输出工作线程 "mojo-voice-out" 持有）。
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<Emitter>();
};

impl Emitter {
    pub fn new() -> Self {
        let dev = match build_keyboard_device() {
            Ok(d) => {
                Log::debug("uinput 虚拟键盘已创建（mojo-virtual）");
                Some(Mutex::new(d))
            }
            Err(e) => {
                Log::error(&format!("创建 uinput 虚拟设备失败：{e}（按键注入不可用）"));
                None
            }
        };
        Self {
            dev,
            clipboard: Mutex::new(None),
        }
    }

    pub fn perform(&self, action: &Act) {
        match action {
            Act::Key { key, mods } => self.send_key(key, mods),
            Act::Media { key } => self.send_media(key),
            Act::Shell { command } => run_shell(command),
            Act::Open { target } => open_target(target),
            Act::MouseMove { dx, dy } => self.move_mouse(*dx, *dy),
            Act::MouseClick { button, count } => self.click_mouse(button, *count),
            Act::MouseScroll { dx, dy } => self.scroll(*dx, *dy),
            Act::Sequence(list) => {
                for a in list {
                    self.perform(a);
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
            Act::None | Act::Passthrough | Act::Dictate => {
                // dictate 由引擎的按住/松开钩子处理，不在这里发事件
            }
        }
    }

    // MARK: 键盘

    pub fn send_key(&self, key: &str, mods: &[String]) {
        // 主键查不到时兜底媒体键表（配置写 {type:key, key:"volup"} 也能工作）
        let Some(code) = key_code(key).or_else(|| media_code(key)) else {
            Log::warn(&format!("未知按键名: {key}"));
            return;
        };
        let mod_codes: Vec<Key> = mods
            .iter()
            .filter_map(|m| {
                let c = mod_code(m);
                if c.is_none() {
                    Log::warn(&format!("未知修饰键: {m}"));
                }
                c
            })
            .collect();
        // 修饰按下 → 主键点按 → 修饰抬起（倒序），每步自带 SYN_REPORT
        for m in &mod_codes {
            self.press_code(*m, 1);
        }
        self.tap_code(code);
        for m in mod_codes.iter().rev() {
            self.press_code(*m, 0);
        }
    }

    // MARK: 媒体键

    pub fn send_media(&self, key: &str) {
        let Some(code) = media_code(key).or_else(|| key_code(key)) else {
            Log::warn(&format!("未知媒体键: {key}"));
            return;
        };
        self.tap_code(code);
    }

    // MARK: 文本输入（LiveTyper / 语音结果用）

    /// 输入一段文本。
    /// ASCII 可打印 → 键码直打；含任何非 ASCII → 整体走剪贴板粘贴。
    pub fn type_text(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        if text.chars().all(|c| ascii_char_code(c).is_some()) {
            for c in text.chars() {
                let Some((code, shift)) = ascii_char_code(c) else {
                    continue;
                };
                if shift {
                    self.press_code(Key::KEY_LEFTSHIFT, 1);
                }
                self.tap_code(code);
                if shift {
                    self.press_code(Key::KEY_LEFTSHIFT, 0);
                }
            }
            return;
        }
        // 非 ASCII：uinput 无法表达 Unicode，整体经剪贴板粘贴上屏
        static WARNED_NON_ASCII: AtomicBool = AtomicBool::new(false);
        if !WARNED_NON_ASCII.swap(true, Ordering::Relaxed) {
            Log::warn(
                "Linux 下非 ASCII 文本经剪贴板粘贴上屏（uinput 不支持 Unicode 直输）；\
                 建议在面板的语音识别页关闭 liveTyping（实时上屏），且 v1 会覆盖剪贴板现有内容",
            );
        }
        self.set_clipboard(text);
        // 等 X11/Wayland 剪贴板所有权生效再发 Ctrl+V
        std::thread::sleep(Duration::from_millis(60));
        self.send_key("v", &["ctrl".to_string()]);
    }

    /// 连按 n 次退格（撤回已上屏的中间结果）
    pub fn press_backspace(&self, times: usize) {
        for _ in 0..times {
            self.tap_code(Key::KEY_BACKSPACE);
        }
    }

    pub fn copy_clipboard_text(&self, text: &str) {
        crate::engine::textout::system_clipboard_copy(text)
    }

    // MARK: 底层注入

    /// 发一批事件（VirtualDevice::emit 自动补 SYN_REPORT）
    fn emit(&self, events: &[InputEvent]) {
        let Some(dev) = &self.dev else {
            return;
        };
        if let Err(e) = dev.lock().unwrap().emit(events) {
            Log::warn(&format!("uinput 注入失败: {e}"));
        }
    }

    /// 按下/松开单个键码（value: 1=按下 0=松开），后留小间隔保证事件有序
    fn press_code(&self, code: Key, value: i32) {
        self.emit(&[InputEvent::new(EventType::KEY, code.code(), value)]);
        std::thread::sleep(Duration::from_millis(4));
    }

    /// 点按单个键码（按下 → 松开）。按下与松开之间留 8ms：
    /// 部分应用对过快的合成按键不敏感（与 macOS 侧间隔口径一致）。
    fn tap_code(&self, code: Key) {
        self.press_code(code, 1);
        std::thread::sleep(Duration::from_millis(8));
        self.press_code(code, 0);
    }

    fn set_clipboard(&self, text: &str) {
        let mut guard = self.clipboard.lock().unwrap();
        if guard.is_none() {
            match arboard::Clipboard::new() {
                Ok(cb) => *guard = Some(cb),
                Err(e) => {
                    Log::warn(&format!("剪贴板不可用: {e}（纯 Wayland 会话暂不支持）"));
                    return;
                }
            }
        }
        if let Some(cb) = guard.as_mut() {
            if let Err(e) = cb.set_text(text) {
                Log::warn(&format!("写剪贴板失败: {e}"));
            }
        }
    }

    // MARK: 鼠标

    fn move_mouse(&self, dx: f64, dy: f64) {
        let (ix, iy) = (dx.round() as i32, dy.round() as i32);
        let mut evs = Vec::new();
        if ix != 0 {
            evs.push(InputEvent::new(
                EventType::RELATIVE,
                RelativeAxisType::REL_X.0,
                ix,
            ));
        }
        if iy != 0 {
            evs.push(InputEvent::new(
                EventType::RELATIVE,
                RelativeAxisType::REL_Y.0,
                iy,
            ));
        }
        if !evs.is_empty() {
            self.emit(&evs);
        }
    }

    fn click_mouse(&self, button: &str, count: i64) {
        let code = match button.to_lowercase().as_str() {
            "right" => Key::BTN_RIGHT,
            "middle" | "center" => Key::BTN_MIDDLE,
            _ => Key::BTN_LEFT,
        };
        let n = count.max(1);
        for i in 0..n {
            self.tap_code(code);
            if i + 1 < n {
                std::thread::sleep(Duration::from_millis(40));
            }
        }
    }

    fn scroll(&self, dx: i64, dy: i64) {
        // REL_WHEEL 正值 = 向上（远离用户），REL_HWHEEL 正值 = 向右
        let mut evs = Vec::new();
        if dy != 0 {
            evs.push(InputEvent::new(
                EventType::RELATIVE,
                RelativeAxisType::REL_WHEEL.0,
                dy as i32,
            ));
        }
        if dx != 0 {
            evs.push(InputEvent::new(
                EventType::RELATIVE,
                RelativeAxisType::REL_HWHEEL.0,
                dx as i32,
            ));
        }
        if !evs.is_empty() {
            self.emit(&evs);
        }
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

// MARK: - 虚拟设备构建

/// 注入用虚拟键盘：能力 = 键表/媒体表全集 + 修饰键 + 鼠标键 + 相对轴。
/// 宁多勿少（uinput 按能力集过滤写入的事件）。
fn build_keyboard_device() -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::new();
    for (_, k) in KEY_TABLE {
        keys.insert(*k);
    }
    for (_, k) in MEDIA_TABLE {
        keys.insert(*k);
    }
    for k in [
        Key::KEY_LEFTMETA,
        Key::KEY_LEFTCTRL,
        Key::KEY_LEFTALT,
        Key::KEY_LEFTSHIFT,
        Key::BTN_LEFT,
        Key::BTN_RIGHT,
        Key::BTN_MIDDLE,
    ] {
        keys.insert(k);
    }
    let mut rels = AttributeSet::new();
    rels.insert(RelativeAxisType::REL_X);
    rels.insert(RelativeAxisType::REL_Y);
    rels.insert(RelativeAxisType::REL_WHEEL);
    rels.insert(RelativeAxisType::REL_HWHEEL);

    VirtualDeviceBuilder::new()?
        .name("mojo-virtual")
        .with_keys(&keys)?
        .with_relative_axes(&rels)?
        .build()
}

// MARK: - ASCII 字符 → 键码（US 布局假设；type_text 直打用）

fn letter_key(c: char) -> Option<Key> {
    Some(match c {
        'a' => Key::KEY_A,
        'b' => Key::KEY_B,
        'c' => Key::KEY_C,
        'd' => Key::KEY_D,
        'e' => Key::KEY_E,
        'f' => Key::KEY_F,
        'g' => Key::KEY_G,
        'h' => Key::KEY_H,
        'i' => Key::KEY_I,
        'j' => Key::KEY_J,
        'k' => Key::KEY_K,
        'l' => Key::KEY_L,
        'm' => Key::KEY_M,
        'n' => Key::KEY_N,
        'o' => Key::KEY_O,
        'p' => Key::KEY_P,
        'q' => Key::KEY_Q,
        'r' => Key::KEY_R,
        's' => Key::KEY_S,
        't' => Key::KEY_T,
        'u' => Key::KEY_U,
        'v' => Key::KEY_V,
        'w' => Key::KEY_W,
        'x' => Key::KEY_X,
        'y' => Key::KEY_Y,
        'z' => Key::KEY_Z,
        _ => return None,
    })
}

fn digit_key(c: char) -> Option<Key> {
    Some(match c {
        '1' => Key::KEY_1,
        '2' => Key::KEY_2,
        '3' => Key::KEY_3,
        '4' => Key::KEY_4,
        '5' => Key::KEY_5,
        '6' => Key::KEY_6,
        '7' => Key::KEY_7,
        '8' => Key::KEY_8,
        '9' => Key::KEY_9,
        '0' => Key::KEY_0,
        _ => return None,
    })
}

/// ASCII 可打印字符 → (键码, 是否需要 Shift)。控制字符返回 None。
fn ascii_char_code(c: char) -> Option<(Key, bool)> {
    match c {
        'a'..='z' => letter_key(c).map(|k| (k, false)),
        'A'..='Z' => letter_key(c.to_ascii_lowercase()).map(|k| (k, true)),
        '0'..='9' => digit_key(c).map(|k| (k, false)),
        ' ' => Some((Key::KEY_SPACE, false)),
        '\n' | '\r' => Some((Key::KEY_ENTER, false)),
        '\t' => Some((Key::KEY_TAB, false)),
        '!' => Some((Key::KEY_1, true)),
        '@' => Some((Key::KEY_2, true)),
        '#' => Some((Key::KEY_3, true)),
        '$' => Some((Key::KEY_4, true)),
        '%' => Some((Key::KEY_5, true)),
        '^' => Some((Key::KEY_6, true)),
        '&' => Some((Key::KEY_7, true)),
        '*' => Some((Key::KEY_8, true)),
        '(' => Some((Key::KEY_9, true)),
        ')' => Some((Key::KEY_0, true)),
        '-' => Some((Key::KEY_MINUS, false)),
        '_' => Some((Key::KEY_MINUS, true)),
        '=' => Some((Key::KEY_EQUAL, false)),
        '+' => Some((Key::KEY_EQUAL, true)),
        '[' => Some((Key::KEY_LEFTBRACE, false)),
        '{' => Some((Key::KEY_LEFTBRACE, true)),
        ']' => Some((Key::KEY_RIGHTBRACE, false)),
        '}' => Some((Key::KEY_RIGHTBRACE, true)),
        '\\' => Some((Key::KEY_BACKSLASH, false)),
        '|' => Some((Key::KEY_BACKSLASH, true)),
        ';' => Some((Key::KEY_SEMICOLON, false)),
        ':' => Some((Key::KEY_SEMICOLON, true)),
        '\'' => Some((Key::KEY_APOSTROPHE, false)),
        '"' => Some((Key::KEY_APOSTROPHE, true)),
        '`' => Some((Key::KEY_GRAVE, false)),
        '~' => Some((Key::KEY_GRAVE, true)),
        ',' => Some((Key::KEY_COMMA, false)),
        '<' => Some((Key::KEY_COMMA, true)),
        '.' => Some((Key::KEY_DOT, false)),
        '>' => Some((Key::KEY_DOT, true)),
        '/' => Some((Key::KEY_SLASH, false)),
        '?' => Some((Key::KEY_SLASH, true)),
        _ => None,
    }
}

// MARK: - shell / open

fn run_shell(cmd: &str) {
    let r = std::process::Command::new("sh")
        .args(["-c", cmd])
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
    // xdg-open 统一处理 URL / 文件路径 / 桌面应用名
    let r = std::process::Command::new("xdg-open")
        .arg(t)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Err(e) = r {
        Log::warn(&format!("xdg-open 执行失败: {e}"));
    }
}
