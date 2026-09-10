//! 平台按键/蓝牙内核的抽象接缝。
//!
//! UI、配置、语音协议全部跨平台；只有这一层与操作系统强相关：
//! - 读取遥控器原始按键（拦截 HID / 系统钩子 / evdev）
//! - 注入模拟按键（CGEvent / SendInput / uinput）
//! - BLE 连接遥控器 ATVV 语音服务
//!
//! 各平台逐步实现这个 trait，上层代码完全不用改。
#![allow(dead_code)]

pub mod action;
pub mod adpcm;
pub mod asr;
pub mod atvv;
pub mod livetype;
pub mod log;
pub mod state;
pub mod termfix;
pub mod textout;
pub mod voice;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod linux;

/// 配置里是否有任何 dictate 绑定（各平台驱动据此决定是否初始化语音链路）
pub(crate) fn uses_dictate(cfg: &Config) -> bool {
    cfg.profiles
        .iter()
        .flat_map(|p| p.bindings.values())
        .any(action::binding_contains_dictate)
}

use crate::config::Config;

#[derive(Debug, Clone)]
pub struct DeviceButtonEvent {
    /// 逻辑按键名：ok / up / back / voice …
    pub button: String,
    pub is_down: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum EngineStatus {
    Stopped,
    Running {
        daemon_connected: bool,
        remote_connected: bool,
    },
    Unsupported {
        reason: String,
    },
}

pub trait PlatformEngine: Send + Sync {
    /// 内核是否已在当前平台实现
    fn supported(&self) -> bool;

    /// 应用配置并启动事件循环（幂等）
    fn start(&self, config: &Config) -> anyhow::Result<()>;

    /// 停止
    fn stop(&self);

    /// 当前状态
    fn status(&self) -> EngineStatus;
}

pub fn current() -> Box<dyn PlatformEngine> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacEngine::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(stub::StubEngine::new())
    }
}

/// 全局唯一引擎实例（Tauri 命令层共用）
pub fn shared() -> &'static dyn PlatformEngine {
    use std::sync::OnceLock;
    static ENGINE: OnceLock<Box<dyn PlatformEngine>> = OnceLock::new();
    ENGINE.get_or_init(current).as_ref()
}

#[cfg(not(target_os = "macos"))]
mod stub {
    use super::*;

    /// Windows / Linux 原生内核尚在移植：
    /// 计划用 btleplug(BLE) + 平台钩子/SendInput/evdev+uinput 实现。
    pub struct StubEngine;

    impl StubEngine {
        pub fn new() -> Self {
            Self
        }
    }

    impl PlatformEngine for StubEngine {
        fn supported(&self) -> bool {
            false
        }
        fn start(&self, _config: &Config) -> anyhow::Result<()> {
            Ok(())
        }
        fn stop(&self) {}
        fn status(&self) -> EngineStatus {
            let os = if cfg!(target_os = "windows") {
                "Windows"
            } else {
                "Linux"
            };
            EngineStatus::Unsupported {
                reason: format!("{os} 原生按键/蓝牙内核正在移植中，当前版本提供完整配置功能"),
            }
        }
    }
}
