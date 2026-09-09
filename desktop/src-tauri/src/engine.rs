//! 平台按键/蓝牙内核的抽象接缝。
//!
//! UI、配置、语音协议全部跨平台；只有这一层与操作系统强相关：
//! - 读取遥控器原始按键（拦截 HID / 系统钩子 / evdev）
//! - 注入模拟按键（CGEvent / SendInput / uinput）
//! - BLE 连接遥控器 ATVV 语音服务
//!
//! 各平台逐步实现这个 trait，上层代码完全不用改。
#![allow(dead_code)]

use crate::config::Config;

#[derive(Debug, Clone)]
pub struct DeviceButtonEvent {
    /// 逻辑按键名：ok / up / back / voice …
    pub button: String,
    pub is_down: bool,
}

#[derive(Debug, Clone)]
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
        Box::new(mac::MacEngine::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(stub::StubEngine::new())
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;

    /// macOS 上第一阶段复用现成 Swift 守护进程（sidecar / 已安装 launchd）。
    /// Rust 原生内核是后续移植项，先通过服务管理模块控制 Swift 版本。
    pub struct MacEngine;

    impl MacEngine {
        pub fn new() -> Self {
            Self
        }
    }

    impl PlatformEngine for MacEngine {
        fn supported(&self) -> bool {
            true
        }
        fn start(&self, _config: &Config) -> anyhow::Result<()> {
            // 实际启停由 service.rs 调用 launchctl 完成
            Ok(())
        }
        fn stop(&self) {}
        fn status(&self) -> EngineStatus {
            EngineStatus::Running {
                daemon_connected: true,
                remote_connected: false,
            }
        }
    }
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
