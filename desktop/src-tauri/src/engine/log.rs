//! 引擎日志：同时写 stderr 和日志文件（面板日志页 tail 同一个文件）。
//! 行为对齐 Swift 的 Log.swift（本地时间 HH:mm:ss.SSS 前缀）。

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub static VERBOSE: AtomicBool = AtomicBool::new(false);

static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

#[cfg(unix)]
fn timestamp() -> String {
    unsafe {
        let mut tv: libc::timeval = std::mem::zeroed();
        libc::gettimeofday(&mut tv, std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&tv.tv_sec, &mut tm);
        format!(
            "{:02}:{:02}:{:02}.{:03}",
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
            tv.tv_usec / 1000
        )
    }
}

#[cfg(not(unix))]
fn timestamp() -> String {
    // 退化格式：自纪元秒数（Windows 面板暂不需要本地时区精度）
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

fn write_line(prefix: &str, msg: &str) {
    let line = format!("{} {}{}", timestamp(), prefix, msg);
    eprintln!("{line}");
    if let Ok(mut g) = FILE.lock() {
        if g.is_none() {
            if let Ok(p) = crate::store::log_path() {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                // 启动轮转：当前日志超过 1MiB 则滚动一份 .old（只保留一份），
                // 防止常驻数月后日志只增不减占满磁盘。
                const MAX_LOG_BYTES: u64 = 1024 * 1024;
                if let Ok(meta) = std::fs::metadata(&p) {
                    if meta.len() > MAX_LOG_BYTES {
                        let old = p.with_extension("log.old");
                        let _ = std::fs::remove_file(&old);
                        let _ = std::fs::rename(&p, &old);
                    }
                }
                *g = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(p)
                    .ok();
            }
        }
        if let Some(f) = g.as_mut() {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }
}

/// 与 Swift 侧 `Log.info(...)` 同款调用风格
pub struct Log;

impl Log {
    pub fn info(msg: &str) {
        info(msg)
    }
    pub fn debug(msg: &str) {
        debug(msg)
    }
    pub fn warn(msg: &str) {
        warn(msg)
    }
    pub fn error(msg: &str) {
        error(msg)
    }
}

pub fn info(msg: &str) {
    write_line("", msg);
}

pub fn debug(msg: &str) {
    if VERBOSE.load(Ordering::Relaxed) {
        write_line("· ", msg);
    }
}

pub fn warn(msg: &str) {
    write_line("警告: ", msg);
}

pub fn error(msg: &str) {
    write_line("错误: ", msg);
}
