//! 配置模型，与 Swift 守护进程的 `config.json` schema 完全兼容。
//!
//! 设计要点：
//! - 设备 ID 既接受整数也接受 `"0x32b8"` 十六进制字符串（serde 自定义）
//! - 动作 `Action` 既接受简写字符串（`"cmd+tab"`、`"return"`）也接受对象
//! - 所有字段尽量带默认值，旧配置能平滑加载

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub device: DeviceConfig,
    #[serde(default)]
    pub options: Options,
    #[serde(default)]
    pub voice: VoiceConfig,
    #[serde(default)]
    pub buttons: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceConfig {
    #[serde(deserialize_with = "de_id")]
    #[serde(serialize_with = "se_hex_id")]
    pub vendor_id: u32,
    #[serde(deserialize_with = "de_id")]
    #[serde(serialize_with = "se_hex_id")]
    pub product_id: u32,
    #[serde(default)]
    pub name: Option<String>,
}

fn de_id<'de, D>(d: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Value::deserialize(d)?;
    match v {
        Value::Number(n) => n
            .as_u64()
            .map(|x| x as u32)
            .ok_or_else(|| serde::de::Error::custom("设备 ID 必须是整数")),
        Value::String(s) => {
            let t = s.trim();
            let body = t
                .strip_prefix("0x")
                .or_else(|| t.strip_prefix("0X"))
                .unwrap_or(t);
            u32::from_str_radix(body, 16).map_err(serde::de::Error::custom)
        }
        _ => Err(serde::de::Error::custom("设备 ID 必须是数字或十六进制字符串")),
    }
}

fn se_hex_id<S>(v: &u32, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_str(&format!("0x{:x}", v))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Options {
    pub swallow_original: bool,
    pub long_press_ms: u32,
    pub double_press_ms: u32,
    pub verbose: bool,
    pub debounce_ms: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            swallow_original: true,
            long_press_ms: 450,
            double_press_ms: 280,
            verbose: false,
            debounce_ms: 150,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VoiceConfig {
    pub locale: String,
    pub output: String,
    pub strip_punctuation: bool,
    pub engine: String,
    pub volc_app_id: String,
    pub volc_access_token: String,
    pub volc_resource_id: String,
    pub live_typing: bool,
    pub fix_terms: bool,
    /// sherpa-onnx 模型目录（空 = 默认路径）
    pub sherpa_model_dir: String,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            locale: "zh-CN".into(),
            output: "type".into(),
            strip_punctuation: true,
            engine: "volc".into(),
            volc_app_id: String::new(),
            volc_access_token: String::new(),
            volc_resource_id: "volc.bigasr.sauc.duration".into(),
            live_typing: true,
            fix_terms: true,
            sherpa_model_dir: String::new(),
        }
    }
}

/// 一个按键的绑定：单击 / 长按 / 双击 / 按住连发
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Binding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap: Option<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long: Option<Action>,
    #[serde(rename = "double", default, skip_serializing_if = "Option::is_none")]
    pub double: Option<Action>,
    #[serde(rename = "repeat", default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<bool>,
}

impl VoiceConfig {
    /// 引擎判断（口径同 Swift）
    pub fn uses_volc(&self) -> bool {
        self.engine.eq_ignore_ascii_case("volc")
            && !self.volc_app_id.is_empty()
            && !self.volc_access_token.is_empty()
    }

    pub fn uses_sherpa(&self) -> bool {
        self.engine.eq_ignore_ascii_case("sherpa")
    }

    pub fn sherpa_dir(&self) -> String {
        if !self.sherpa_model_dir.is_empty() {
            return self.sherpa_model_dir.clone();
        }
        dirs::home_dir()
            .map(|h| {
                h.join(".config/mojo/models/sherpa-onnx-x-asr-480ms-zh_int8-2025-12-26")
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileMatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_names: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(rename = "match", default, skip_serializing_if = "Option::is_none")]
    pub match_: Option<ProfileMatch>,
    #[serde(default)]
    pub bindings: std::collections::BTreeMap<String, Binding>,
}

/// 一个动作。同时支持简写字符串和对象两种 JSON 形态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Action {
    /// 简写："return"、"cmd+tab"、"ctrl+c"
    Shorthand(String),
    Full(FullAction),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FullAction {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mods: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// open 动作的别名（与 Swift schema 对齐：target ?? app ?? url）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dx: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dy: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<Action>>,
}

impl Config {
    /// 内置默认配置（新用户首次启动用）
    pub fn builtin_default() -> Config {
        serde_json::from_str(include_str!("../default-config.json"))
            .expect("内置默认配置必须合法")
    }

    /// 原始信号 -> 逻辑按键名（反查表），与 Swift `rawToButton()` 一致
    pub fn raw_to_button(&self) -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        for (name, raws) in &self.buttons {
            for r in raws {
                m.insert(r.to_lowercase(), name.clone());
            }
        }
        m
    }

    /// 选出匹配当前前台 App 的方案：有 match 命中的优先，
    /// 其次名为 default 的，再次第一个无 match 的。与 Swift `resolveProfile` 一致。
    pub fn resolve_profile(&self, bundle_id: Option<&str>, app_name: Option<&str>) -> Option<&Profile> {
        for p in &self.profiles {
            if p.match_
                .as_ref()
                .is_some_and(|m| m.matches(bundle_id, app_name))
            {
                return Some(p);
            }
        }
        self.profiles
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case("default"))
            .or_else(|| self.profiles.iter().find(|p| p.match_.is_none()))
    }
}

impl ProfileMatch {
    /// bundleId / appName 任一命中即匹配（不区分大小写）
    pub fn matches(&self, bundle_id: Option<&str>, app_name: Option<&str>) -> bool {
        if let (Some(ids), Some(b)) = (&self.bundle_ids, bundle_id) {
            if ids.iter().any(|x| x.eq_ignore_ascii_case(b)) {
                return true;
            }
        }
        if let (Some(names), Some(n)) = (&self.app_names, app_name) {
            if names.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_default_parses() {
        let cfg = Config::builtin_default();
        assert_eq!(cfg.device.vendor_id, 0x2717);
        assert_eq!(cfg.device.product_id, 0x32b8);
        assert_eq!(cfg.voice.engine, "volc");
        assert!(cfg.profiles.len() >= 1);
        // 内置默认只带一个全局兜底方案
        let dft = cfg.profiles.iter().find(|p| p.name == "默认配置").unwrap();
        assert!(dft.bindings.contains_key("ok"));
    }

    #[test]
    fn shorthand_and_full_actions() {
        let c: Config = serde_json::from_str(
            r#"{
              "device": {"vendorId": "0x2717", "productId": "0x32b8"},
              "profiles": [{
                "name": "t",
                "bindings": {
                  "ok": { "tap": "return", "repeat": true },
                  "menu": { "tap": {"type":"key","key":"c","mods":["ctrl"]} }
                }
              }]
            }"#,
        )
        .unwrap();
        let b = &c.profiles[0].bindings;
        assert!(matches!(b["ok"].tap.as_ref().unwrap(), Action::Shorthand(s) if s == "return"));
        assert!(b["ok"].repeat == Some(true));
        match b["menu"].tap.as_ref().unwrap() {
            Action::Full(f) => {
                assert_eq!(f.kind, "key");
                assert_eq!(f.key.as_deref(), Some("c"));
            }
            _ => panic!("应为 FullAction"),
        }
    }

    #[test]
    fn round_trip_keeps_camel_case() {
        let cfg = Config::builtin_default();
        let json = serde_json::to_string(&cfg).unwrap();
        // 必须产出 camelCase 字段
        assert!(json.contains("vendorId"));
        assert!(json.contains("longPressMs"));
        assert!(json.contains("volcAppId"));
        assert!(json.contains("liveTyping"));
        // 带 match 的方案序列化后保留 bundleIds（camelCase）
        let with_match: Config = serde_json::from_str(
            r#"{"device":{"vendorId":"0x2717","productId":"0x32b8"},
                "profiles":[{"name":"t","match":{"bundleIds":["a.b.c"]},"bindings":{}}]}"#,
        )
        .unwrap();
        assert!(serde_json::to_string(&with_match).unwrap().contains("bundleIds"));
        // 不应泄漏 snake_case
        assert!(!json.contains("long_press_ms"));
        assert!(!json.contains("volc_app_id"));
        // 再解析回来
        let cfg2: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg2.options.long_press_ms, cfg.options.long_press_ms);
    }

    #[test]
    fn parses_real_user_config_if_present() {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = std::path::Path::new(&home)
            .join(".config/mojo/config.json");
        if path.exists() {
            let raw = std::fs::read_to_string(path).unwrap();
            let cfg: Config = serde_json::from_str(&raw).expect("真实配置必须能解析");
            assert!(!cfg.profiles.is_empty());
        }
    }
}
