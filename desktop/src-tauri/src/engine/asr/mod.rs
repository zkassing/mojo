//! ASR 会话抽象：火山云端 / sherpa 本地，同一套输入输出形状
//! （对应 Swift 的 `ASRSession` 协议）。
//!
//! 会话是单工的：ATVV 推 PCM 进来（`SessionIn`），识别文本出去（`AsrOut`）。
//! 输入通道用 tokio unbounded —— tokio 任务（volc）`.recv().await`，
//! sherpa 专用线程 `.blocking_recv()`，两端都顺手。

pub mod sherpa;
pub mod volc;

/// 会话输入（ATVV → 会话）
pub enum SessionIn {
    /// 一帧 PCM（16-bit LE, 16 kHz 单声道）
    Pcm(Vec<u8>),
    /// 推流结束（ATVV 收到 AUDIO_STOP 或主动关麦）→ 会话给出最终结果
    Finish,
    /// 放弃本次识别（新会话顶替 / 引擎停止）
    Cancel,
}

/// 会话输出（会话 → 文本消费者）
#[derive(Debug)]
pub enum AsrOut {
    /// 中间结果（累计全文，边说边出字）
    Partial(String),
    /// 最终结果（空串 = 没识别到内容）
    Final(String),
}

pub type SessionInTx = tokio::sync::mpsc::UnboundedSender<SessionIn>;
pub type AsrOutTx = tokio::sync::mpsc::UnboundedSender<AsrOut>;
