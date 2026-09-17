//! sherpa-onnx 本地流式识别（对应 Swift `SherpaASRClient` + SherpaBridge）。
//!
//! 直接复用 `Sources/SherpaBridge/bridge.c`（build.rs 用 cc 编译，链接
//! third_party/sherpa-onnx 预编译库），行为与 Swift 完全一致：
//! - 引擎全局只加载一次（prewarm），每个会话 reset 流复用
//! - 端点（句尾静音）确认的文本计入 committed
//!
//! 线程模型：一条专用 "sherpa" 线程串行处理所有命令（等价 Swift 的
//! 串行 DispatchQueue + NSLock）。sherpa_available cfg 不存在时（库未下载
//! 或非 macOS），退化为 stub：会话直接回空结果。

use super::{AsrOut, AsrOutTx, SessionIn};
use crate::engine::log::Log;

use std::sync::OnceLock;

// MARK: - FFI（bridge.c 提供）

#[cfg(sherpa_available)]
mod ffi {
    use std::ffi::{c_char, c_float, c_int, c_void};

    pub type MRSherpa = c_void;

    extern "C" {
        pub fn mr_sherpa_create(model_dir: *const c_char, num_threads: c_int) -> *mut MRSherpa;
        pub fn mr_sherpa_destroy(p: *mut MRSherpa);
        pub fn mr_sherpa_accept(p: *mut MRSherpa, samples: *const c_float, n: c_int);
        pub fn mr_sherpa_text(p: *mut MRSherpa) -> *const c_char;
        pub fn mr_sherpa_is_endpoint(p: *mut MRSherpa) -> c_int;
        pub fn mr_sherpa_reset(p: *mut MRSherpa);
        pub fn mr_sherpa_input_finished(p: *mut MRSherpa);
    }
}

// MARK: - 命令通道

enum SherpaCmd {
    /// 预加载模型（面板启动 / 配置启用 sherpa 时调用一次）
    Load(String),
    /// 新一次识别会话
    Session {
        input: tokio::sync::mpsc::UnboundedReceiver<SessionIn>,
        out: AsrOutTx,
    },
}

fn sherpa_tx() -> &'static std::sync::mpsc::Sender<SherpaCmd> {
    static TX: OnceLock<std::sync::mpsc::Sender<SherpaCmd>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<SherpaCmd>();
        std::thread::Builder::new()
            .name("mojo-sherpa".into())
            .spawn(move || sherpa_thread(rx))
            .expect("启动 sherpa 线程失败");
        tx
    })
}

/// 后台加载模型（引擎启动时调一次，避免首次按键等待）
pub fn prewarm(model_dir: &str) {
    let _ = sherpa_tx().send(SherpaCmd::Load(model_dir.to_string()));
}

/// 启动一次识别会话
pub fn start_session(
    input: tokio::sync::mpsc::UnboundedReceiver<SessionIn>,
    out: AsrOutTx,
) {
    let _ = sherpa_tx().send(SherpaCmd::Session { input, out });
}

// MARK: - sherpa 线程

#[cfg(sherpa_available)]
fn sherpa_thread(rx: std::sync::mpsc::Receiver<SherpaCmd>) {
    use std::ffi::CString;

    let mut engine: *mut ffi::MRSherpa = std::ptr::null_mut();

    for cmd in rx {
        match cmd {
            SherpaCmd::Load(dir) => {
                if !engine.is_null() {
                    continue;
                }
                let t = std::time::Instant::now();
                let c_dir = CString::new(dir.clone()).unwrap_or_default();
                let p = unsafe { ffi::mr_sherpa_create(c_dir.as_ptr(), 2) };
                if p.is_null() {
                    Log::error(&format!("sherpa 模型加载失败: {dir}"));
                } else {
                    engine = p;
                    Log::info(&format!("sherpa 模型已加载（{:.1?}）", t.elapsed()));
                }
            }
            SherpaCmd::Session { mut input, out } => {
                if engine.is_null() {
                    Log::warn("sherpa 模型未就绪，本次忽略");
                    let _ = out.send(AsrOut::Final(String::new()));
                    continue;
                }
                run_session(engine, &mut input, &out);
            }
        }
    }

    if !engine.is_null() {
        unsafe { ffi::mr_sherpa_destroy(engine) };
    }
}

#[cfg(sherpa_available)]
fn run_session(
    eng: *mut ffi::MRSherpa,
    input: &mut tokio::sync::mpsc::UnboundedReceiver<SessionIn>,
    out: &AsrOutTx,
) {
    unsafe { ffi::mr_sherpa_reset(eng) };
    let mut committed = String::new();

    while let Some(msg) = input.blocking_recv() {
        match msg {
            SessionIn::Pcm(pcm) => {
                // Int16 LE → Float / 32768
                let count = pcm.len() / 2;
                if count == 0 {
                    continue;
                }
                let mut floats = Vec::with_capacity(count);
                for c in pcm.chunks_exact(2) {
                    floats.push(i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0);
                }
                unsafe {
                    ffi::mr_sherpa_accept(eng, floats.as_ptr(), count as i32);
                    let text = c_str_to_string(ffi::mr_sherpa_text(eng));
                    if ffi::mr_sherpa_is_endpoint(eng) != 0 {
                        committed.push_str(&text);
                        ffi::mr_sherpa_reset(eng);
                        let _ = out.send(AsrOut::Partial(committed.clone()));
                    } else {
                        let mut full = committed.clone();
                        full.push_str(&text);
                        let _ = out.send(AsrOut::Partial(full));
                    }
                }
            }
            SessionIn::Finish => {
                unsafe {
                    ffi::mr_sherpa_input_finished(eng);
                    let mut full = committed;
                    full.push_str(&c_str_to_string(ffi::mr_sherpa_text(eng)));
                    let _ = out.send(AsrOut::Final(full));
                }
                return;
            }
            SessionIn::Cancel => return,
        }
    }
}

#[cfg(sherpa_available)]
unsafe fn c_str_to_string(p: *const std::ffi::c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { std::ffi::CStr::from_ptr(p) }
        .to_string_lossy()
        .into_owned()
}

// MARK: - stub（库不可用 / 非 macOS）

#[cfg(not(sherpa_available))]
fn sherpa_thread(rx: std::sync::mpsc::Receiver<SherpaCmd>) {
    for cmd in rx {
        match cmd {
            SherpaCmd::Load(dir) => {
                Log::warn(&format!(
                    "sherpa 以 stub 编译（third_party/sherpa-onnx 缺失或非 macOS），模型不会加载: {dir}"
                ));
            }
            SherpaCmd::Session { out, .. } => {
                Log::warn("sherpa 不可用（stub 编译），本次识别忽略");
                let _ = out.send(AsrOut::Final(String::new()));
            }
        }
    }
}
