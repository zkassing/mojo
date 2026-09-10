//! Tauri 命令层：前端通过 invoke 调用这些函数。

mod asr;
mod config;
mod engine;
mod store;

use config::Config;
use tauri::{AppHandle, Emitter, Manager};

/// 供测试 / CLI 验证火山凭证连通性
pub async fn test_volc_credentials(
    app_id: String,
    access_token: String,
    resource_id: String,
) -> asr::TestResult {
    asr::test_connection(asr::Credentials {
        app_id,
        access_token,
        resource_id,
    })
    .await
}

#[tauri::command]
fn config_load() -> Result<Config, String> {
    store::load().map_err(|e| e.to_string())
}

#[tauri::command]
fn config_save(cfg: Config) -> Result<(), String> {
    store::save(&cfg).map_err(|e| e.to_string())?;
    // 引擎在跑时热重载（等价 Swift 守护进程的文件监听 reload）
    if matches!(engine::shared().status(), engine::EngineStatus::Running { .. }) {
        let _ = engine::shared().start(&cfg);
    }
    Ok(())
}

#[tauri::command]
fn config_default() -> Config {
    Config::builtin_default()
}

#[tauri::command]
fn config_exists() -> bool {
    store::exists()
}

#[tauri::command]
fn config_path_string() -> Result<String, String> {
    Ok(store::config_path()
        .map_err(|e| e.to_string())?
        .display()
        .to_string())
}

#[tauri::command]
fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    /// macOS 的 CFBundleIdentifier；其它平台为 None
    bundle_id: Option<String>,
    /// 应用显示名
    app_name: Option<String>,
    path: String,
}

/// 解析用户选中的应用：macOS 读 .app/Contents/Info.plist，
/// Windows/Linux 退化为可执行文件名。
#[tauri::command]
fn resolve_app(path: String) -> Result<AppInfo, String> {
    #[cfg(target_os = "macos")]
    {
        let plist = std::path::Path::new(&path).join("Contents/Info.plist");
        if plist.exists() {
            let bid = plist_buddy(&plist, "CFBundleIdentifier");
            let name = plist_buddy(&plist, "CFBundleDisplayName")
                .or_else(|| plist_buddy(&plist, "CFBundleName"));
            return Ok(AppInfo {
                bundle_id: bid,
                app_name: name,
                path,
            });
        }
    }

    // 其它平台 / 选到的不是 .app：用文件名作为应用名
    let fname = std::path::Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string());
    Ok(AppInfo {
        bundle_id: None,
        app_name: fname,
        path,
    })
}

/// 列出本机已安装的应用（macOS 扫描 Applications 目录），
/// 供「方案设置」里按名字点选，不用手填 Bundle ID。
#[tauri::command]
fn list_apps() -> Result<Vec<AppInfo>, String> {
    #[cfg(not(target_os = "macos"))]
    {
        return Ok(Vec::new());
    }
    #[cfg(target_os = "macos")]
    {
        let mut dirs = vec![
            std::path::PathBuf::from("/Applications"),
            std::path::PathBuf::from("/System/Applications"),
        ];
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join("Applications"));
        }
        let mut apps = Vec::new();
        for d in &dirs {
            scan_apps_dir(d, 0, &mut apps);
        }
        apps.sort_by_key(|a| {
            a.app_name.clone().unwrap_or_default().to_lowercase()
        });
        apps.dedup_by(|a, b| a.bundle_id.is_some() && a.bundle_id == b.bundle_id);
        Ok(apps)
    }
}

#[cfg(target_os = "macos")]
fn scan_apps_dir(dir: &std::path::Path, depth: u8, out: &mut Vec<AppInfo>) {
    if depth > 1 {
        return; // 只下钻一层（Utilities 等）
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let is_app = path.extension().is_some_and(|x| x == "app");
        if is_app {
            let plist = path.join("Contents/Info.plist");
            let Some(bid) = plist_buddy(&plist, "CFBundleIdentifier") else {
                continue;
            };
            let name = plist_buddy(&plist, "CFBundleDisplayName")
                .or_else(|| plist_buddy(&plist, "CFBundleName"))
                .or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()));
            out.push(AppInfo {
                bundle_id: Some(bid),
                app_name: name,
                path: path.to_string_lossy().to_string(),
            });
        } else if depth == 0 && path.is_dir() {
            scan_apps_dir(&path, depth + 1, out);
        }
    }
}

#[cfg(target_os = "macos")]
fn plist_buddy(plist: &std::path::Path, key: &str) -> Option<String> {
    let out = std::process::Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Print :{key}"), plist.to_str()?])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

#[tauri::command]
fn engine_supported() -> bool {
    engine::shared().supported()
}

/// 启动内置引擎（幂等；已在跑则热重载配置）
#[tauri::command]
fn engine_start() -> Result<(), String> {
    let cfg = store::load().map_err(|e| e.to_string())?;
    engine::shared().start(&cfg).map_err(|e| e.to_string())
}

#[tauri::command]
fn engine_stop() {
    engine::shared().stop()
}

#[tauri::command]
fn engine_runtime_status() -> engine::EngineStatus {
    engine::shared().status()
}

/// 读取日志尾部（用于“实时日志”页初始内容）
#[tauri::command]
fn log_tail(lines: usize) -> Result<String, String> {
    let path = store::log_path().map_err(|e| e.to_string())?;
    if !path.exists() {
        return Ok(String::new());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let n = lines.max(1);
    let v: Vec<&str> = raw.lines().collect();
    let start = v.len().saturating_sub(n);
    Ok(v[start..].join("\n"))
}

/// 开始跟踪日志，新增行通过事件 `log-line` 推给前端
#[tauri::command]
/// 开始跟踪日志，新增行通过事件 `log-line` 推给前端。
/// 全局只跑一个线程：前端每次进日志页都会调本命令，
/// 不守卫的话多个 tail 线程会把同一行推 N 遍。
fn log_follow(app: AppHandle) {
    use std::io::{BufRead, BufReader, Seek};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    static FOLLOWING: AtomicBool = AtomicBool::new(false);
    if FOLLOWING.swap(true, Ordering::SeqCst) {
        return;
    }

    thread::spawn(move || {
        // 线程退出时复位，允许下次重新跟随
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                FOLLOWING.store(false, Ordering::SeqCst);
            }
        }
        let _reset = Reset;

        let Ok(path) = store::log_path() else { return };
        if !path.exists() {
            let _ = app.emit("log-follow-ended", "日志文件尚不存在");
            return;
        }
        let Ok(file) = std::fs::File::open(&path) else {
            return;
        };
        let mut reader = BufReader::new(file);
        // 从文件末尾开始，只推新增内容
        let _ = reader.seek(std::io::SeekFrom::End(0));

        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => thread::sleep(Duration::from_millis(400)),
                Ok(_) => {
                    let trimmed = line.trim_end_matches('\n');
                    if app.emit("log-line", trimmed).is_err() {
                        break; // 窗口关闭
                    }
                }
                Err(_) => break,
            }
        }
    });
}

#[tauri::command(rename_all = "camelCase")]
async fn volc_test(
    app_id: String,
    access_token: String,
    resource_id: String,
) -> Result<asr::TestResult, String> {
    let r = asr::test_connection(asr::Credentials {
        app_id,
        access_token,
        resource_id,
    })
    .await;
    Ok(r)
}

#[tauri::command]
fn sherpa_model_status(custom_dir: Option<String>) -> serde_json::Value {
    let dir = sherpa_dir(custom_dir);
    // 模型目录下需有 tokens.txt 和一个 encoder*.onnx（兼容新旧两套命名）
    let base = std::path::Path::new(&dir);
    let has_encoder = base.join("encoder.int8.onnx").exists()
        || base.join("encoder-epoch-99-avg-1.int8.onnx").exists()
        || base.join("encoder.onnx").exists();
    let installed = base.join("tokens.txt").exists() && has_encoder;
    serde_json::json!({ "installed": installed, "dir": dir })
}

fn sherpa_dir(custom_dir: Option<String>) -> String {
    custom_dir.filter(|s| !s.is_empty()).unwrap_or_else(|| {
        dirs::home_dir()
            .map(|h| {
                h.join(".config/mojo/models/sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05")
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_default()
    })
}

/// 一键下载 sherpa 模型（GitHub Releases tar.bz2，curl + tar 均为系统自带）
#[tauri::command]
async fn sherpa_model_download(app: tauri::AppHandle) -> Result<String, String> {
    use tauri::Emitter;
    const URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05.tar.bz2";
    const DIR_NAME: &str = "sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05";

    let home = dirs::home_dir().ok_or("找不到用户主目录")?;
    let models = home.join(".config/mojo/models");
    std::fs::create_dir_all(&models).map_err(|e| e.to_string())?;
    let tar_path = models.join("sherpa-model.tar.bz2");

    // 总大小（跟随跳转，从响应头取 Content-Length）
    let total: u64 = std::process::Command::new("curl")
        .args(["-sIL", URL])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|h| {
            h.lines()
                .filter(|l| l.to_lowercase().starts_with("content-length:"))
                .last()
                .and_then(|l| l.split(':').nth(1)?.trim().parse().ok())
        })
        .unwrap_or(0);

    // 后台下载，轮询文件大小发进度事件
    let tar_str = tar_path.to_string_lossy().into_owned();
    let mut child = std::process::Command::new("curl")
        .args(["-sL", "--fail", "-o", &tar_str, URL])
        .spawn()
        .map_err(|e| format!("启动 curl 失败: {e}"))?;
    let app2 = app.clone();
    let tar2 = tar_path.clone();
    let progress = std::thread::spawn(move || {
        loop {
            let size = std::fs::metadata(&tar2).map(|m| m.len()).unwrap_or(0);
            let _ = app2.emit("sherpa-download-progress", serde_json::json!({
                "downloaded": size, "total": total
            }));
            std::thread::sleep(std::time::Duration::from_millis(300));
            if size >= total && total > 0 { break; }
        }
    });
    let status = tokio::task::spawn_blocking(move || child.wait())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let _ = progress.join();
    if !status.success() {
        let _ = std::fs::remove_file(&tar_path);
        return Err("下载失败（网络错误或链接失效）".into());
    }

    // 解压（bsdtar 自动识别 bz2）
    let out = std::process::Command::new("tar")
        .args(["xjf", &tar_str, "-C", &models.to_string_lossy()])
        .output()
        .map_err(|e| format!("解压失败: {e}"))?;
    let _ = std::fs::remove_file(&tar_path);
    if !out.status.success() {
        return Err(format!("解压失败: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(models.join(DIR_NAME).to_string_lossy().into_owned())
}

// MARK: - 托盘

/// 显示并聚焦主窗口
fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 切换内置引擎启停（托盘菜单用；启动失败只记日志不弹窗）
fn toggle_engine() {
    match engine::shared().status() {
        engine::EngineStatus::Running { .. } => engine::shared().stop(),
        engine::EngineStatus::Unsupported { .. } => {}
        engine::EngineStatus::Stopped => {
            if let Ok(cfg) = store::load() {
                if let Err(e) = engine::shared().start(&cfg) {
                    engine::log::Log::error(&format!("托盘启动引擎失败: {e}"));
                }
            }
        }
    }
}

/// 构建系统托盘：状态行 + 显示面板 + 引擎启停 + 退出
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
    use tauri::tray::TrayIconBuilder;
    use tauri_plugin_autostart::ManagerExt;

    let status = MenuItemBuilder::with_id("engine_status", "引擎：已停止")
        .enabled(false)
        .build(app)?;
    let show = MenuItemBuilder::with_id("show", "显示面板").build(app)?;
    let toggle = MenuItemBuilder::with_id("engine_toggle", "启动引擎").build(app)?;
    let autostart = CheckMenuItemBuilder::with_id("autostart", "开机自启")
        .checked(false)
        .build(app)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItemBuilder::with_id("quit", "退出 Mojo").build(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&status, &show, &toggle, &autostart, &sep, &quit])
        .build()?;

    // macOS 用单色模板图标（随深浅色菜单栏自适应），其他平台用彩色图标
    let icon_bytes: &[u8] = if cfg!(target_os = "macos") {
        include_bytes!("../icons/tray-icon.png")
    } else {
        include_bytes!("../icons/32x32.png")
    };
    let icon = tauri::image::Image::from_bytes(icon_bytes)?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .menu(&menu)
        .tooltip("Mojo 小米蓝牙语音遥控器")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "quit" => app.exit(0),
            "show" => show_main_window(app),
            "engine_toggle" => toggle_engine(),
            "autostart" => {
                let m = app.autolaunch();
                let r = match m.is_enabled() {
                    Ok(true) => m.disable(),
                    _ => m.enable(),
                };
                if let Err(e) = r {
                    engine::log::Log::error(&format!("切换开机自启失败: {e}"));
                }
            }
            _ => {}
        })
        .build(app)?;

    // 周期刷新状态行 / 启停项文案 / 自启勾选（引擎状态和遥控器连接都可能异步变化）
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let _ = autostart.set_checked(handle.autolaunch().is_enabled().unwrap_or(false));
            let (s, t) = match engine::shared().status() {
                engine::EngineStatus::Running {
                    remote_connected, ..
                } => (
                    if remote_connected {
                        "引擎：运行中 · 遥控器已连接".to_string()
                    } else {
                        "引擎：运行中".to_string()
                    },
                    "停止引擎",
                ),
                engine::EngineStatus::Stopped => ("引擎：已停止".to_string(), "启动引擎"),
                engine::EngineStatus::Unsupported { .. } => {
                    ("引擎：此平台暂不支持".to_string(), "启动引擎")
                }
            };
            let _ = status.set_text(s);
            let _ = toggle.set_text(t);
        }
    });

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例：重复启动时聚焦已有窗口并退出新进程，
        // 避免多个实例各起一个日志跟随线程导致日志重复。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri::Manager;
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        // 开机自启：macOS 走 LaunchAgent（~/Library/LaunchAgents/com.zyk.mojo.plist）
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            #[cfg(debug_assertions)]
            if let Some(win) = app.get_webview_window("main") {
                win.open_devtools();
            }
            setup_tray(app)?;
            // 常驻后台形态：macOS 上隐藏 Dock 图标，只留菜单栏托盘
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            // 即开即用：已有配置文件就自动拉起引擎（开机自启才有意义）
            if engine::shared().supported()
                && store::config_path().map(|p| p.exists()).unwrap_or(false)
            {
                if let Ok(cfg) = store::load() {
                    if let Err(e) = engine::shared().start(&cfg) {
                        engine::log::Log::error(&format!("启动时自动拉起引擎失败: {e}"));
                    }
                }
            }
            Ok(())
        })
        // 关窗不退 App：隐藏到托盘（「退出 Mojo」菜单才真正退出）
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            config_load,
            config_save,
            config_default,
            config_exists,
            config_path_string,
            platform_name,
            resolve_app,
            list_apps,
            engine_supported,
            engine_start,
            engine_stop,
            engine_runtime_status,
            log_tail,
            log_follow,
            volc_test,
            sherpa_model_status,
            sherpa_model_download,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
