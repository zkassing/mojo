//! 事件注入：按键 / 媒体键 / 文本 / 鼠标 / shell / 打开目标。
//!
//! 对应 Swift `Emitter.swift` + `NXPoster.swift`：
//! - 普通按键优先走 IOHIDPostEvent（驱动层、无 PID，与物理键盘不可区分，
//!   能触发过滤 CGEvent 合成事件的全局热键，如微信），CGEvent 兜底
//! - 媒体键经 NSEvent(.systemDefined) → CGEvent
//! - 文本用 CGEventKeyboardSetUnicodeString Unicode 直输

use super::ffi;
use crate::engine::action::Act;
use crate::engine::log::Log;

use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField,
    ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;

// MARK: - 按键名 -> CGKeyCode（与 Swift KeyMap.table 一致）

static KEY_TABLE: &[(&str, u16)] = &[
    ("a", 0), ("s", 1), ("d", 2), ("f", 3), ("h", 4), ("g", 5), ("z", 6), ("x", 7),
    ("c", 8), ("v", 9), ("b", 11), ("q", 12), ("w", 13), ("e", 14), ("r", 15),
    ("y", 16), ("t", 17), ("1", 18), ("2", 19), ("3", 20), ("4", 21), ("6", 22),
    ("5", 23), ("=", 24), ("9", 25), ("7", 26), ("-", 27), ("8", 28), ("0", 29),
    ("]", 30), ("o", 31), ("u", 32), ("[", 33), ("i", 34), ("p", 35),
    ("return", 36), ("enter", 36), ("l", 37), ("j", 38), ("'", 39), ("k", 40),
    (";", 41), ("\\", 42), (",", 43), ("/", 44), ("n", 45), ("m", 46), (".", 47),
    ("tab", 48), ("space", 49), ("`", 50), ("delete", 51), ("backspace", 51),
    ("escape", 53), ("esc", 53), ("cmd", 55), ("command", 55), ("capslock", 57),
    ("shift", 56), ("alt", 58), ("opt", 58), ("option", 58), ("ctrl", 59), ("control", 59),
    ("fn", 63), ("f17", 64),
    ("keypaddecimal", 65), ("keypadmultiply", 67), ("keypadplus", 69), ("keypadclear", 71),
    ("keypaddivide", 75), ("keypadenter", 76), ("keypadminus", 78), ("f18", 79), ("f19", 80),
    ("keypadequals", 81), ("keypad0", 82), ("keypad1", 83), ("keypad2", 84), ("keypad3", 85),
    ("keypad4", 86), ("keypad5", 87), ("keypad6", 88), ("keypad7", 89), ("f20", 90),
    ("keypad8", 91), ("keypad9", 92),
    ("f5", 96), ("f6", 97), ("f7", 98), ("f3", 99), ("f8", 100), ("f9", 101),
    ("f11", 103), ("f13", 105), ("f16", 106), ("f14", 107), ("f10", 109), ("f12", 111),
    ("f15", 113), ("help", 114), ("home", 115), ("pageup", 116), ("forwarddelete", 117),
    ("del", 117), ("f4", 118), ("end", 119), ("f2", 120), ("pagedown", 121),
    ("f1", 122), ("left", 123), ("right", 124), ("down", 125), ("up", 126),
];

pub fn key_code(name: &str) -> Option<u16> {
    let low = name.to_lowercase();
    KEY_TABLE.iter().find(|(n, _)| *n == low).map(|(_, c)| *c)
}

pub fn key_name(code: i64) -> String {
    KEY_TABLE
        .iter()
        .find(|(_, c)| *c as i64 == code)
        .map(|(n, _)| n.to_string())
        .unwrap_or_else(|| format!("kc:{code}"))
}

fn flags_of(mods: &[String]) -> CGEventFlags {
    let mut f = CGEventFlags::CGEventFlagNull;
    for m in mods {
        match m.to_lowercase().as_str() {
            "cmd" | "command" | "meta" | "super" => f |= CGEventFlags::CGEventFlagCommand,
            "shift" => f |= CGEventFlags::CGEventFlagShift,
            "opt" | "option" | "alt" => f |= CGEventFlags::CGEventFlagAlternate,
            "ctrl" | "control" => f |= CGEventFlags::CGEventFlagControl,
            "fn" | "function" => f |= CGEventFlags::CGEventFlagSecondaryFn,
            _ => {}
        }
    }
    f
}

// MARK: - 媒体键（NX_KEYTYPE_*）

static MEDIA_TABLE: &[(&str, i32)] = &[
    ("volup", 0), ("voldown", 1), ("brightnessup", 2), ("brightnessdown", 3),
    ("capslock", 4), ("mute", 7), ("eject", 14),
    ("playpause", 16), ("play", 16), ("pause", 16),
    ("next", 17), ("nexttrack", 17), ("previous", 18), ("prev", 18), ("prevtrack", 18),
    ("fast", 19), ("forward", 19), ("rewind", 20),
    ("illuminationup", 21), ("illuminationdown", 22),
];

pub fn media_name(code: i32) -> String {
    MEDIA_TABLE
        .iter()
        .find(|(_, c)| *c == code)
        .map(|(n, _)| n.to_string())
        .unwrap_or_else(|| format!("aux:{code}"))
}

// MARK: - NXPoster：驱动层按键注入（IOHIDPostEvent）

mod nx {
    pub const KEY_DOWN: u32 = 10;
    pub const KEY_UP: u32 = 11;
    pub const FLAGS_CHANGED: u32 = 12;

    pub const SHIFT: u32 = 0x0002_0000;
    pub const CONTROL: u32 = 0x0004_0000;
    pub const ALT: u32 = 0x0008_0000;
    pub const CMD: u32 = 0x0010_0000;
    pub const FN: u32 = 0x0080_0000;
}

struct NXPoster {
    handle: ffi::io_connect_t,
    available: bool,
}

impl NXPoster {
    const REQUEST_TYPE_POST_EVENT: i32 = 0;
    const EVENT_DATA_VERSION: u32 = 2;
    const SET_GLOBAL_EVENT_FLAGS: u32 = 0x1;

    fn new() -> Self {
        unsafe {
            let handle = ffi::NXOpenEventStatus();
            if handle == 0 {
                Log::warn("NXOpenEventStatus 失败，按键注入走 CGEvent");
                return Self { handle, available: false };
            }
            if !ffi::IOHIDRequestAccess(Self::REQUEST_TYPE_POST_EVENT) {
                Log::warn("IOHIDPostEvent 访问未授权，按键注入走 CGEvent");
                ffi::NXCloseEventStatus(handle);
                return Self { handle: 0, available: false };
            }
            Log::info("驱动层按键注入已启用（IOHIDPostEvent）");
            Self { handle, available: true }
        }
    }

    fn nx_flags(mods: &[String]) -> u32 {
        let mut f = 0u32;
        for m in mods {
            match m.to_lowercase().as_str() {
                "shift" => f |= nx::SHIFT,
                "ctrl" | "control" => f |= nx::CONTROL,
                "alt" | "opt" | "option" => f |= nx::ALT,
                "cmd" | "command" | "meta" | "super" => f |= nx::CMD,
                "fn" | "function" => f |= nx::FN,
                _ => {}
            }
        }
        f
    }

    /// macOS 虚拟键码 → 修饰键 NX mask（是修饰键则非 None）
    fn modifier_mask(code: u16) -> Option<u32> {
        match code {
            56 | 60 => Some(nx::SHIFT),
            59 | 62 => Some(nx::CONTROL),
            58 | 61 => Some(nx::ALT),
            55 | 54 => Some(nx::CMD),
            63 => Some(nx::FN),
            _ => None,
        }
    }

    /// 发一次完整按键（按下+松开）。不可用时返回 false。
    fn send_key(&self, code: u16, mods: &[String]) -> bool {
        if !self.available {
            return false;
        }
        let mut data = ffi::NXKeyEventData::zeroed();
        data.key_code = code;
        let mod_flags = Self::nx_flags(mods);

        if let Some(self_mask) = Self::modifier_mask(code) {
            // 键本身是修饰键：flagsChanged，按下带自己的 mask、松开清零
            self.post(nx::FLAGS_CHANGED, &data, mod_flags | self_mask);
            std::thread::sleep(std::time::Duration::from_millis(10));
            self.post(nx::FLAGS_CHANGED, &data, 0);
        } else {
            self.post(nx::KEY_DOWN, &data, mod_flags);
            std::thread::sleep(std::time::Duration::from_millis(8));
            self.post(nx::KEY_UP, &data, 0);
        }
        true
    }

    fn post(&self, type_: u32, data: &ffi::NXKeyEventData, flags: u32) {
        unsafe {
            ffi::IOHIDPostEvent(
                self.handle,
                type_,
                ffi::NXIOGPoint { x: 0, y: 0 },
                data,
                Self::EVENT_DATA_VERSION,
                flags,
                Self::SET_GLOBAL_EVENT_FLAGS,
            );
        }
    }
}

impl Drop for NXPoster {
    fn drop(&mut self) {
        if self.handle != 0 {
            unsafe { ffi::NXCloseEventStatus(self.handle) };
        }
    }
}

// MARK: - Emitter

pub struct Emitter {
    src: CGEventSource,
    nxp: NXPoster,
}

// CGEventPost / IOHIDPostEvent 本身是线程安全的（Quartz 事件投递无线程亲和）；
// 我们的使用形态也是单线程持有（语音输出工作线程或引擎线程各持一份）。
unsafe impl Send for Emitter {}

impl Emitter {
    pub fn new() -> Self {
        let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .expect("创建 CGEventSource 失败");
        Self {
            src,
            nxp: NXPoster::new(),
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

    pub fn send_key(&self, key: &str, mods: &[String]) {
        let Some(code) = key_code(key) else {
            Log::warn(&format!("未知按键名: {key}"));
            return;
        };
        // 优先驱动层注入
        if self.nxp.send_key(code, mods) {
            return;
        }
        // CGEvent 兜底。键本身是修饰键时，按下事件必须带上自己的 flag
        // （修饰键走 flagsChanged，不带 flag 监听方会认为是空事件）。
        let self_flag = flags_of(&[key.to_string()]);
        let down_flags = flags_of(mods) | self_flag;
        let up_flags = flags_of(mods) - self_flag;
        let (Ok(down), Ok(up)) = (
            CGEvent::new_keyboard_event(self.src.clone(), code, true),
            CGEvent::new_keyboard_event(self.src.clone(), code, false),
        ) else {
            return;
        };
        down.set_flags(down_flags);
        up.set_flags(up_flags);
        down.post(CGEventTapLocation::HID);
        up.post(CGEventTapLocation::HID);
    }

    // MARK: 媒体键

    pub fn send_media(&self, key: &str) {
        let low = key.to_lowercase();
        let Some(code) = MEDIA_TABLE.iter().find(|(n, _)| *n == low).map(|(_, c)| *c) else {
            Log::warn(&format!("未知媒体键: {key}"));
            return;
        };
        post_aux(code, true);
        post_aux(code, false);
    }

    // MARK: 文本输入（LiveTyper / 语音结果用）

    /// 输入一段文本（Unicode 直输，不经键码）
    pub fn type_text(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let (Ok(down), Ok(up)) = (
            CGEvent::new_keyboard_event(self.src.clone(), 0, true),
            CGEvent::new_keyboard_event(self.src.clone(), 0, false),
        ) else {
            return;
        };
        down.set_string(text);
        up.set_string(text);
        down.post(CGEventTapLocation::HID);
        up.post(CGEventTapLocation::HID);
    }

    /// 连按 n 次退格（撤回已上屏的中间结果）
    pub fn press_backspace(&self, times: usize) {
        self.press_backspace_impl(times)
    }

    pub fn copy_clipboard_text(&self, text: &str) {
        crate::engine::textout::system_clipboard_copy(text)
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

impl Emitter {
    fn press_backspace_impl(&self, times: usize) {
        for _ in 0..times {
            self.send_key("delete", &[]);
        }
    }

    // MARK: 鼠标

    fn move_mouse(&self, dx: f64, dy: f64) {
        let cur = self.current_mouse();
        let target = CGPoint::new(cur.x + dx, cur.y + dy);
        if let Ok(ev) = CGEvent::new_mouse_event(
            self.src.clone(),
            CGEventType::MouseMoved,
            target,
            CGMouseButton::Left,
        ) {
            ev.post(CGEventTapLocation::HID);
        }
    }

    fn click_mouse(&self, button: &str, count: i64) {
        let pos = self.current_mouse();
        let (btn, dn, up) = match button.to_lowercase().as_str() {
            "right" => (CGMouseButton::Right, CGEventType::RightMouseDown, CGEventType::RightMouseUp),
            "middle" | "center" => {
                (CGMouseButton::Center, CGEventType::OtherMouseDown, CGEventType::OtherMouseUp)
            }
            _ => (CGMouseButton::Left, CGEventType::LeftMouseDown, CGEventType::LeftMouseUp),
        };
        let n = count.max(1);
        for i in 1..=n {
            let (Ok(d), Ok(u)) = (
                CGEvent::new_mouse_event(self.src.clone(), dn, pos, btn),
                CGEvent::new_mouse_event(self.src.clone(), up, pos, btn),
            ) else {
                return;
            };
            d.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, i);
            u.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, i);
            d.post(CGEventTapLocation::HID);
            u.post(CGEventTapLocation::HID);
            if i < n {
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
        }
    }

    fn scroll(&self, dx: i32, dy: i32) {
        if let Ok(ev) = CGEvent::new_scroll_event(self.src.clone(), ScrollEventUnit::PIXEL, 2, dy, dx, 0) {
            ev.post(CGEventTapLocation::HID);
        }
    }

    fn current_mouse(&self) -> CGPoint {
        // CGEventCreate 出来的新事件位置即当前光标位置（NSEvent.mouseLocation 的 CG 版，
        // 坐标系本来就是左上原点，无需翻转）
        CGEvent::new(self.src.clone())
            .map(|ev| ev.location())
            .unwrap_or(CGPoint::new(0.0, 0.0))
    }
}

// MARK: - 媒体键（经 NSEvent systemDefined）

fn post_aux(key_code: i32, down: bool) {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_foundation::NSPoint;

    let flags = NSEventModifierFlags::from_bits_retain(if down { 0xA00 } else { 0xB00 });
    let data1 = ((key_code as isize) << 16) | ((if down { 0xA } else { 0xB }) << 8);
    let Some(ev) = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
        NSEventType::SystemDefined,
        NSPoint::new(0.0, 0.0),
        flags,
        0.0,
        0,
        None,
        8,
        data1,
        -1,
    ) else {
        return;
    };
    let Some(cg) = ev.CGEvent() else {
        return;
    };
    let ptr = std::ptr::from_ref(&*cg) as *mut std::ffi::c_void;
    unsafe { ffi::CGEventPost(0 /* kCGHIDEventTap */, ptr) };
}

// MARK: - shell / open

fn run_shell(cmd: &str) {
    let r = std::process::Command::new("/bin/zsh")
        .args(["-lc", cmd])
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
    let mut cmd = std::process::Command::new("/usr/bin/open");
    if t.contains("://") {
        cmd.arg(t);
    } else if t.contains('.') && !t.starts_with('/') && !t.ends_with(".app") {
        // 看起来像 bundle id
        cmd.args(["-b", t]);
    } else if t.starts_with('/') {
        cmd.arg(t);
    } else {
        cmd.args(["-a", t]);
    }
    let _ = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
