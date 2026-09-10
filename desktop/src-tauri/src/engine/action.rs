//! 动作归一化：把配置层的 `Action`（简写字符串 / 完整对象两种形态）
//! 解析成引擎层语义明确的 `Act` 枚举。
//!
//! 解析规则与 Swift 版 `Config.swift` 的 `Action.init(from:)` +
//! `parseShorthand` 一一对应。

use crate::config::{Action, Binding, FullAction};

/// 归一化后的动作
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// 模拟键盘按键
    Key { key: String, mods: Vec<String> },
    /// 系统媒体键，如 playpause / volup
    Media { key: String },
    Shell { command: String },
    /// 打开 App（名字或 bundle id）/ URL / 文件路径
    Open { target: String },
    MouseMove { dx: f64, dy: f64 },
    MouseClick { button: String, count: i64 },
    MouseScroll { dx: i64, dy: i64 },
    Sequence(Vec<Act>),
    /// 语音转文字（按住开麦，松开识别）
    Dictate,
    /// 吞掉按键什么都不做
    None,
    /// 放行原始按键
    Passthrough,
}

impl Act {
    pub fn normalize(a: &Action) -> Act {
        match a {
            Action::Shorthand(s) => Act::from_shorthand(s),
            Action::Full(f) => Act::from_full(f),
        }
    }

    /// 简写："return" / "cmd+tab" / "ctrl+shift+t"，及特殊词 dictate/none/pass
    pub fn from_shorthand(s: &str) -> Act {
        let low = s.trim().to_lowercase();
        match low.as_str() {
            "dictate" | "voice" | "speech" | "asr" => return Act::Dictate,
            "none" | "nothing" | "ignore" => return Act::None,
            "passthrough" | "pass" => return Act::Passthrough,
            _ => {}
        }
        let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
        let key = parts.last().copied().unwrap_or("").to_string();
        let mods: Vec<String> = parts[..parts.len().saturating_sub(1)]
            .iter()
            .map(|m| m.to_lowercase())
            .collect();
        Act::Key { key, mods }
    }

    fn from_full(f: &FullAction) -> Act {
        match f.kind.to_lowercase().as_str() {
            "key" | "keyboard" | "hotkey" => {
                let key = f.key.clone().unwrap_or_default();
                let mods = f.mods.clone().unwrap_or_default();
                // 支持 key 里直接写 "cmd+tab"
                if mods.is_empty() && key.contains('+') {
                    Act::from_shorthand(&key)
                } else {
                    Act::Key { key, mods }
                }
            }
            "media" | "aux" | "system" => Act::Media {
                key: f.key.clone().unwrap_or_default(),
            },
            "shell" | "command" | "exec" => Act::Shell {
                command: f.command.clone().unwrap_or_default(),
            },
            "open" | "app" | "url" | "launch" => Act::Open {
                target: f
                    .target
                    .clone()
                    .or_else(|| f.app.clone())
                    .or_else(|| f.url.clone())
                    .unwrap_or_default(),
            },
            "mousemove" | "move" => Act::MouseMove {
                dx: f.dx.unwrap_or(0.0),
                dy: f.dy.unwrap_or(0.0),
            },
            "mouseclick" | "click" => Act::MouseClick {
                button: f.button.clone().unwrap_or_else(|| "left".into()),
                count: f.count.unwrap_or(1),
            },
            "mousescroll" | "scroll" => Act::MouseScroll {
                dx: f.dx.unwrap_or(0.0) as i64,
                dy: f.dy.unwrap_or(0.0) as i64,
            },
            "sequence" | "seq" | "multi" => Act::Sequence(
                f.actions
                    .clone()
                    .unwrap_or_default()
                    .iter()
                    .map(Act::normalize)
                    .collect(),
            ),
            "none" | "nothing" | "ignore" => Act::None,
            "dictate" | "voice" | "speech" | "asr" => Act::Dictate,
            "passthrough" | "pass" => Act::Passthrough,
            _ => Act::None, // 未知类型：与 Swift 抛错不同，引擎层选择忽略
        }
    }

    pub fn is_dictate(&self) -> bool {
        matches!(self, Act::Dictate)
    }
    pub fn is_none(&self) -> bool {
        matches!(self, Act::None)
    }
    pub fn is_passthrough(&self) -> bool {
        matches!(self, Act::Passthrough)
    }

    /// 自身或序列子动作里是否含 dictate（用于决定是否启用语音链路）
    pub fn contains_dictate(&self) -> bool {
        match self {
            Act::Dictate => true,
            Act::Sequence(list) => list.iter().any(Act::contains_dictate),
            _ => false,
        }
    }

    /// 日志展示用（对应 Swift 的 describe）
    pub fn describe(&self) -> String {
        match self {
            Act::Key { key, mods } => {
                if mods.is_empty() {
                    format!("按键 {key}")
                } else {
                    format!("按键 {}+{key}", mods.join("+"))
                }
            }
            Act::Media { key } => format!("媒体 {key}"),
            Act::Shell { command } => format!("shell `{}`", &command[..command.len().min(50)]),
            Act::Open { target } => format!("打开 {target}"),
            Act::MouseMove { dx, dy } => format!("鼠标移动 ({dx}, {dy})"),
            Act::MouseClick { button, count } => format!("鼠标{button}键 x{count}"),
            Act::MouseScroll { dx, dy } => format!("滚动 ({dx}, {dy})"),
            Act::Sequence(list) => format!(
                "序列[{}]",
                list.iter().map(Act::describe).collect::<Vec<_>>().join(", ")
            ),
            Act::Dictate => "语音转文字".into(),
            Act::None => "忽略".into(),
            Act::Passthrough => "放行".into(),
        }
    }
}

/// 绑定是否「显式放行」：没有长按/双击，单击是 passthrough
/// （对应 Swift 中 handle/handleHID 里的 passthrough 短路）
pub fn binding_is_passthrough(b: &Binding) -> bool {
    b.long.is_none()
        && b.double.is_none()
        && b.tap
            .as_ref()
            .map(|t| Act::normalize(t).is_passthrough())
            .unwrap_or(false)
}

/// 绑定的任一动作是否含 dictate
pub fn binding_contains_dictate(b: &Binding) -> bool {
    [&b.tap, &b.long, &b.double]
        .into_iter()
        .flatten()
        .any(|a| Act::normalize(a).contains_dictate())
}

/// 纯 dictate 绑定（单击 = dictate 且无长按/双击）走「按住-松开」专用路径，
/// 不进单击/长按状态机
pub fn binding_is_pure_dictate(b: &Binding) -> bool {
    b.long.is_none()
        && b.double.is_none()
        && b.tap
            .as_ref()
            .map(|t| Act::normalize(t).is_dictate())
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shorthand(s: &str) -> Action {
        Action::Shorthand(s.into())
    }

    fn full(kind: &str, key: Option<&str>) -> Action {
        Action::Full(FullAction {
            kind: kind.into(),
            key: key.map(|s| s.into()),
            mods: None,
            command: None,
            target: None,
            app: None,
            url: None,
            dx: None,
            dy: None,
            button: None,
            count: None,
            actions: None,
        })
    }

    #[test]
    fn shorthand_plain_key() {
        assert_eq!(
            Act::normalize(&shorthand("return")),
            Act::Key {
                key: "return".into(),
                mods: vec![]
            }
        );
    }

    #[test]
    fn shorthand_with_mods() {
        assert_eq!(
            Act::normalize(&shorthand("Cmd+Shift+T")),
            Act::Key {
                key: "T".into(),
                mods: vec!["cmd".into(), "shift".into()]
            }
        );
    }

    #[test]
    fn shorthand_special_words() {
        assert_eq!(Act::normalize(&shorthand("dictate")), Act::Dictate);
        assert_eq!(Act::normalize(&shorthand("VOICE")), Act::Dictate);
        assert_eq!(Act::normalize(&shorthand("none")), Act::None);
        assert_eq!(Act::normalize(&shorthand("ignore")), Act::None);
        assert_eq!(Act::normalize(&shorthand("pass")), Act::Passthrough);
    }

    #[test]
    fn full_key_with_plus_in_key_field() {
        // Swift: mods 为空且 key 含 "+" 时按简写解析
        assert_eq!(
            Act::normalize(&full("key", Some("cmd+space"))),
            Act::Key {
                key: "space".into(),
                mods: vec!["cmd".into()]
            }
        );
    }

    #[test]
    fn full_kind_aliases() {
        assert!(matches!(
            Act::normalize(&full("hotkey", Some("a"))),
            Act::Key { .. }
        ));
        assert!(matches!(
            Act::normalize(&full("aux", Some("playpause"))),
            Act::Media { .. }
        ));
        assert_eq!(Act::normalize(&full("voice", None)), Act::Dictate);
        assert_eq!(Act::normalize(&full("pass", None)), Act::Passthrough);
        assert_eq!(Act::normalize(&full("未知类型", None)), Act::None);
    }

    #[test]
    fn sequence_contains_dictate() {
        let a = Action::Full(FullAction {
            kind: "sequence".into(),
            key: None,
            mods: None,
            command: None,
            target: None,
            app: None,
            url: None,
            dx: None,
            dy: None,
            button: None,
            count: None,
            actions: Some(vec![shorthand("return"), shorthand("dictate")]),
        });
        let act = Act::normalize(&a);
        assert!(act.contains_dictate());
        assert!(!Act::normalize(&shorthand("return")).contains_dictate());
    }

    #[test]
    fn binding_predicates() {
        let b: Binding = serde_json::from_str(r#"{"tap":"passthrough"}"#).unwrap();
        assert!(binding_is_passthrough(&b));
        let b: Binding = serde_json::from_str(r#"{"tap":"dictate"}"#).unwrap();
        assert!(binding_is_pure_dictate(&b));
        assert!(binding_contains_dictate(&b));
        // 有长按就不是「纯 dictate」也不是 passthrough 短路
        let b: Binding =
            serde_json::from_str(r#"{"tap":"passthrough","long":"return"}"#).unwrap();
        assert!(!binding_is_passthrough(&b));
    }
}
