//! 守护进程服务管理。
//!
//! macOS：通过 launchctl 管理 com.zyk.mojo（LaunchAgent）。
//! 其他平台：后续用系统服务 / 计划任务实现，目前返回未支持。

#![allow(dead_code)]

use std::process::Command;

const LABEL: &str = "com.zyk.mojo";

pub fn gui_target() -> String {
    let uid = current_uid();
    format!("gui/{uid}/{LABEL}")
}

fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        // 用 `id -u` 取当前用户 uid，避免引入额外 libc 依赖
        run("id", &["-u"]).1.trim().parse().unwrap_or(0)
    }
    #[cfg(not(unix))]
    {
        0
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ServiceStatus {
    pub platform_supported: bool,
    pub installed: bool,
    pub running: bool,
    pub label: String,
}

fn run(cmd: &str, args: &[&str]) -> (bool, String) {
    match Command::new(cmd).args(args).output() {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            (o.status.success(), s)
        }
        Err(e) => (false, e.to_string()),
    }
}

#[cfg(target_os = "macos")]
pub fn status() -> ServiceStatus {
    let (ok, out) = run("launchctl", &["print", &gui_target()]);
    let running = ok && out.contains("state = running");
    ServiceStatus {
        platform_supported: true,
        installed: ok,
        running,
        label: LABEL.into(),
    }
}

#[cfg(target_os = "macos")]
pub fn kickstart() -> anyhow::Result<()> {
    let (ok, err) = run("launchctl", &["kickstart", "-k", &gui_target()]);
    if !ok {
        anyhow::bail!("启动失败：{err}");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn stop_service() -> anyhow::Result<()> {
    let _ = run("launchctl", &["kill", "TERM", &gui_target()]);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn status() -> ServiceStatus {
    ServiceStatus {
        platform_supported: false,
        installed: false,
        running: false,
        label: LABEL.into(),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn kickstart() -> anyhow::Result<()> {
    anyhow::bail!("当前平台服务管理尚未实现")
}

#[cfg(not(target_os = "macos"))]
pub fn stop_service() -> anyhow::Result<()> {
    anyhow::bail!("当前平台服务管理尚未实现")
}
