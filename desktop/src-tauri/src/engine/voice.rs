//! 语音链路编排（对应 Swift `RemapEngine` 的 setupVoice / handleDictate 全链路）：
//!
//!   按住 dictate 键 → ATVV 开麦推 PCM → ASR 会话（火山/ sherpa）
//!     → 中间结果 LiveTyper 差异上屏（liveTyping）
//!     → 最终结果：去标点 → 谐音纠正 → 上屏 / 回车 / 复制剪贴板
//!
//! 线程模型：
//!   - ATVV / volc / 文本消费者跑在引擎私有 tokio runtime（2 线程）
//!   - sherpa 跑自己的专用线程（见 asr/sherpa.rs）
//!   - 上屏动作走一条专用 "mojo-voice-out" 线程（串行，等价 Swift 的
//!     LiveTyper 串行队列），持有 Emitter

use super::asr::{self, AsrOut, SessionIn, SessionInTx};
use super::atvv::{self, AtvvHandle};
use super::livetype;
use super::log::Log;
use super::macos::emit::Emitter;
use super::termfix;
use crate::config::Config;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

// MARK: - tokio runtime（引擎私有，不占 Tauri 的）

pub fn rt() -> tokio::runtime::Handle {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("mojo-voice-rt")
            .build()
            .expect("创建语音 tokio runtime 失败")
    })
    .handle()
    .clone()
}

// MARK: - VoiceChain

pub struct VoiceChain {
    atvv: AtvvHandle,
    out_jobs: std::sync::mpsc::Sender<OutJob>,
    /// 当前会话的输入通道（也注册在 ATVV 的 audio sink 里）
    session: Mutex<Option<SessionInTx>>,
    dictating: AtomicBool,
}

impl VoiceChain {
    pub fn start(device_name: &str) -> Self {
        let handle = rt();
        Self {
            atvv: atvv::spawn(&handle, device_name.to_string()),
            out_jobs: spawn_out_worker(),
            session: Mutex::new(None),
            dictating: AtomicBool::new(false),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.atvv.is_ready()
    }

    /// 按住 dictate 键：开一次识别会话并开麦
    pub fn dictate_start(&self, cfg: &Config) {
        if self.dictating.swap(true, Ordering::SeqCst) {
            return;
        }
        // 残留会话取消
        if let Some(old) = self.session.lock().unwrap().take() {
            let _ = old.send(SessionIn::Cancel);
        }
        if !self.atvv.is_ready() {
            Log::warn("语音通道未就绪（遥控器可能休眠，先按任意键唤醒）");
            self.dictating.store(false, Ordering::SeqCst);
            return;
        }

        let (in_tx, in_rx) = tokio::sync::mpsc::unbounded_channel::<SessionIn>();
        let (out_tx, out_rx) = tokio::sync::mpsc::unbounded_channel::<AsrOut>();
        self.atvv.set_audio_sink(Some(in_tx.clone()));
        *self.session.lock().unwrap() = Some(in_tx);

        if cfg.voice.uses_sherpa() {
            asr::sherpa::start_session(in_rx, out_tx);
        } else if cfg.voice.uses_volc() {
            asr::volc::start_session(
                &rt(),
                asr::volc::Credentials {
                    app_id: cfg.voice.volc_app_id.clone(),
                    access_token: cfg.voice.volc_access_token.clone(),
                    resource_id: cfg.voice.volc_resource_id.clone(),
                },
                in_rx,
                out_tx,
            );
        } else {
            Log::warn("未选择识别引擎（在面板的语音识别页选 火山 或 sherpa）");
            self.atvv.set_audio_sink(None);
            *self.session.lock().unwrap() = None;
            self.dictating.store(false, Ordering::SeqCst);
            return;
        }

        spawn_out_consumer(out_rx, OutOpts::from_config(cfg), self.out_jobs.clone());
        self.atvv.open_mic();
    }

    /// 松开 dictate 键：关麦（Finish 由 ATVV 在 AUDIO_STOP / 关麦后发给会话）
    pub fn dictate_stop(&self) {
        if !self.dictating.swap(false, Ordering::SeqCst) {
            return;
        }
        self.atvv.close_mic();
    }

    /// 引擎停止：注销音频通道并取消会话
    pub fn shutdown(&self) {
        self.atvv.set_audio_sink(None);
        if let Some(old) = self.session.lock().unwrap().take() {
            let _ = old.send(SessionIn::Cancel);
        }
        self.dictating.store(false, Ordering::SeqCst);
    }
}

// MARK: - 文本输出选项（端口 Swift applyDictationOutputOptions / cleanChinesePunct）

struct OutOpts {
    strip_punct: bool,
    fix_terms: bool,
    live_typing: bool,
    output: String,
}

impl OutOpts {
    fn from_config(cfg: &Config) -> Self {
        Self {
            strip_punct: cfg.voice.strip_punctuation,
            fix_terms: cfg.voice.fix_terms,
            live_typing: cfg.voice.live_typing,
            output: cfg.voice.output.trim().to_lowercase(),
        }
    }

    fn apply(&self, text: &str) -> String {
        let mut t = text.to_string();
        if self.strip_punct {
            t = clean_chinese_punct(&t);
        }
        if self.fix_terms {
            t = termfix::fix(&t);
        }
        t
    }
}

/// 去掉中文标点（口径同 Swift cleanChinesePunct；直接输出模式标点已由
/// ASR 关标点，这里只是兜底）
fn clean_chinese_punct(s: &str) -> String {
    let mut out = s
        .chars()
        .filter(|c| !matches!(c, '，' | '。' | '？' | '！' | '、' | '；' | '：'))
        .collect::<String>();
    out = out.replace('…', " ").replace('—', " ");
    // 换行 → 空格
    out = out.replace('\n', " ");
    // 收尾标点与空白
    let trailing = ['。', '，', '！', '？', ',', '.', '、'];
    while out.chars().last().map(|c| trailing.contains(&c)).unwrap_or(false) {
        out.pop();
    }
    out.trim().to_string()
}

// MARK: - ASR 输出消费者（每个会话一个 tokio 任务）

fn spawn_out_consumer(
    mut out_rx: tokio::sync::mpsc::UnboundedReceiver<AsrOut>,
    opts: OutOpts,
    jobs: std::sync::mpsc::Sender<OutJob>,
) {
    rt().spawn(async move {
        // liveTyping：上屏动作只走 live 通道；剪贴板模式 live 没意义
        let live = opts.live_typing && opts.output != "clipboard" && opts.output != "copy";
        if live {
            let _ = jobs.send(OutJob::Reset);
        }

        while let Some(o) = out_rx.recv().await {
            match o {
                AsrOut::Partial(t) => {
                    Log::debug(&format!("中间结果: {}", truncate_chars(&t, 80)));
                    if live {
                        let _ = jobs.send(OutJob::Update(opts.apply(&t)));
                    }
                }
                AsrOut::Final(t) => {
                    let t = opts.apply(&t);
                    if live {
                        if t.is_empty() {
                            let _ = jobs.send(OutJob::Clear);
                            Log::info("没识别到内容");
                        } else {
                            Log::info(&format!("识别结果: {t}"));
                            let enter = matches!(
                                opts.output.as_str(),
                                "typeenter" | "type_enter" | "enter"
                            );
                            let _ = jobs.send(OutJob::Finalize(t, enter));
                        }
                    } else if t.is_empty() {
                        Log::info("没识别到内容");
                    } else {
                        Log::info(&format!("识别结果: {t}"));
                        match opts.output.as_str() {
                            "clipboard" | "copy" => {
                                let _ = jobs.send(OutJob::Copy(t));
                                Log::info("已复制到剪贴板");
                            }
                            "typeenter" | "type_enter" | "enter" => {
                                let _ = jobs.send(OutJob::TypeText(t));
                                let _ = jobs.send(OutJob::PressEnter);
                            }
                            _ => {
                                let _ = jobs.send(OutJob::TypeText(t));
                            }
                        }
                    }
                    return; // 会话结束
                }
            }
        }
    });
}

fn truncate_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

// MARK: - 上屏工作线程（串行队列，持有 Emitter）

enum OutJob {
    /// 新一轮 liveTyping 前重置状态
    Reset,
    /// 中间结果：差异上屏
    Update(String),
    /// 最终结果：差异上屏 + 收尾（可选回车）
    Finalize(String, bool),
    /// 清空已上屏的临时文字（没识别到内容）
    Clear,
    TypeText(String),
    Copy(String),
    PressEnter,
}

fn spawn_out_worker() -> std::sync::mpsc::Sender<OutJob> {
    let (tx, rx) = std::sync::mpsc::channel::<OutJob>();
    std::thread::Builder::new()
        .name("mojo-voice-out".into())
        .spawn(move || {
            let emitter = Emitter::new();
            let mut on_screen = String::new();
            for job in rx {
                match job {
                    OutJob::Reset => on_screen.clear(),
                    OutJob::Update(t) => apply_diff(&emitter, &mut on_screen, &t),
                    OutJob::Finalize(t, enter) => {
                        apply_diff(&emitter, &mut on_screen, &t);
                        on_screen.clear();
                        if enter {
                            // 打完字稍等再回车，避免某些输入框没反应过来
                            std::thread::sleep(std::time::Duration::from_millis(80));
                            emitter.send_key("return", &[]);
                        }
                    }
                    OutJob::Clear => {
                        let n = on_screen.chars().count();
                        emitter.press_backspace(n);
                        on_screen.clear();
                    }
                    OutJob::TypeText(t) => emitter.type_text(&t),
                    OutJob::Copy(t) => copy_to_clipboard(&t),
                    OutJob::PressEnter => {
                        std::thread::sleep(std::time::Duration::from_millis(80));
                        emitter.send_key("return", &[]);
                    }
                }
            }
        })
        .expect("启动语音输出线程失败");
    tx
}

fn apply_diff(emitter: &Emitter, on_screen: &mut String, target: &str) {
    let (backspaces, suffix) = livetype::diff_plan(on_screen, target);
    emitter.press_backspace(backspaces);
    emitter.type_text(&suffix);
    *on_screen = target.to_string();
}

#[cfg(test)]
mod tests {
    use super::clean_chinese_punct;

    #[test]
    fn clean_punct() {
        assert_eq!(clean_chinese_punct("你好，世界。"), "你好世界");
        assert_eq!(clean_chinese_punct("打开微信，然后回车！"), "打开微信然后回车");
        assert_eq!(clean_chinese_punct("没有标点"), "没有标点");
        assert_eq!(clean_chinese_punct("  首尾。 "), "首尾");
        assert_eq!(clean_chinese_punct(""), "");
    }
}

fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    if let Ok(mut child) = std::process::Command::new("/usr/bin/pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}
