//! 配置文件定位与读写。
//!
//! macOS/Linux: ~/.config/mojo/config.json
//! Windows:     %APPDATA%\mojo\config.json

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::config::Config;

pub fn config_dir() -> Result<PathBuf> {
    // 与 Swift 守护进程保持一致：
    //   macOS / Linux: ~/.config/mojo
    //   Windows:       %APPDATA%\mojo
    // 注意 dirs::config_dir() 在 macOS 上是 ~/Library/Application Support，不能用。
    #[cfg(target_os = "windows")]
    {
        let base = dirs::config_dir().context("找不到系统配置目录")?;
        Ok(base.join("mojo"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = dirs::home_dir().context("找不到用户主目录")?;
        Ok(home.join(".config").join("mojo"))
    }
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

pub fn log_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("logs").join("mojo.log"))
}

/// 读取配置。文件不存在时返回内置默认配置（不落盘）。
pub fn load() -> Result<Config> {
    let p = config_path()?;
    if !p.exists() {
        return Ok(Config::builtin_default());
    }
    let raw = std::fs::read_to_string(&p)
        .with_context(|| format!("读取配置失败: {}", p.display()))?;
    let cfg: Config = serde_json::from_str(&raw)
        .with_context(|| format!("解析配置失败: {}", p.display()))?;
    Ok(cfg)
}

/// 保存配置（pretty JSON，权限在 unix 上收紧到 600，因为含 ASR 凭证）。
pub fn save(cfg: &Config) -> Result<()> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir).context("创建配置目录失败")?;
    let p = dir.join("config.json");
    let json = serde_json::to_string_pretty(cfg).context("序列化配置失败")?;
    std::fs::write(&p, json).with_context(|| format!("写入配置失败: {}", p.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// 配置文件是否存在
pub fn exists() -> bool {
    config_path().map(|p| p.exists()).unwrap_or(false)
}
