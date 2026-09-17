//! 火山引擎「流式语音识别大模型」完整会话（对应 Swift `VolcASRClient`）。
//!
//! 协议：WebSocket 自定义二进制帧
//!   [4 字节头][4 字节 sequence（可选）][4 字节 payload 长度][payload]
//! 遥控器 15ms 一包连续推流，边收边往上送（100ms 一帧），
//! 松开按键时结果基本已经算完。

use super::{AsrOut, AsrOutTx, SessionIn};
use crate::engine::log::Log;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async_tls_with_config;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const ENDPOINT: &str = "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel";

/// 攒到 100ms 再发一包（16kHz 16bit mono = 3200B）—— 火山推荐粒度
const CHUNK_BYTES: usize = 3200;

mod msg_type {
    pub const FULL_CLIENT: u8 = 0x1;
    pub const AUDIO_ONLY: u8 = 0x2;
    pub const FULL_SERVER: u8 = 0x9;
    pub const SERVER_ACK: u8 = 0xB;
    pub const ERROR: u8 = 0xF;
}

mod flags {
    pub const POS_SEQ: u8 = 0x1;
    pub const NEG_SEQ: u8 = 0x3; // 最后一包用负数 sequence
}

#[derive(Clone)]
pub struct Credentials {
    pub app_id: String,
    pub access_token: String,
    pub resource_id: String,
}

fn header(t: u8, flags: u8, serialization: u8, compression: u8) -> Vec<u8> {
    vec![
        0x11, // version 1 | header size 1 (=4 字节)
        (t << 4) | flags,
        (serialization << 4) | compression,
        0x00,
    ]
}

/// 启动一次识别会话（在传入的 tokio 运行时上 spawn）
pub fn start_session(
    rt: &tokio::runtime::Handle,
    creds: Credentials,
    mut input: tokio::sync::mpsc::UnboundedReceiver<SessionIn>,
    out: AsrOutTx,
) {
    rt.spawn(async move {
        run(creds, &mut input, &out).await;
    });
}

async fn run(
    creds: Credentials,
    input: &mut tokio::sync::mpsc::UnboundedReceiver<SessionIn>,
    out: &AsrOutTx,
) {
    // ---- 建连 ----
    let mut req = match ENDPOINT.into_client_request() {
        Ok(r) => r,
        Err(e) => {
            Log::warn(&format!("火山 ASR 构造请求失败: {e}"));
            let _ = out.send(AsrOut::Final(String::new()));
            return;
        }
    };
    {
        let h = req.headers_mut();
        for (k, v) in [
            ("X-Api-App-Key", creds.app_id.as_str()),
            ("X-Api-Access-Key", creds.access_token.as_str()),
            ("X-Api-Resource-Id", creds.resource_id.as_str()),
            ("X-Api-Connect-Id", uuid_like().as_str()),
        ] {
            if let Ok(v) = v.parse() {
                h.insert(k, v);
            }
        }
    }
    let (mut ws, _) = match tokio::time::timeout(
        std::time::Duration::from_secs(20),
        connect_async_tls_with_config(req, None, false, None),
    )
    .await
    {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => {
            Log::warn(&format!("火山 ASR 连接失败（检查凭证/网络）: {e}"));
            let _ = out.send(AsrOut::Final(String::new()));
            return;
        }
        Err(_) => {
            Log::warn("火山 ASR 连接超时（20s 无响应）");
            let _ = out.send(AsrOut::Final(String::new()));
            return;
        }
    };

    // ---- fullClientRequest ----
    let body = serde_json::json!({
        "user": { "uid": "mojo" },
        "audio": { "format": "pcm", "codec": "raw", "rate": 16000, "bits": 16, "channel": 1 },
        "request": {
            "model_name": "bigmodel",
            "enable_itn": true,
            "enable_punc": false,
            "enable_ddc": false,
            "show_utterances": false,
            "result_type": "full",
        }
    });
    let json = serde_json::to_vec(&body).unwrap_or_default();
    let mut frame = header(msg_type::FULL_CLIENT, flags::POS_SEQ, 0x1, 0x0);
    frame.extend_from_slice(&1i32.to_be_bytes());
    frame.extend_from_slice(&(json.len() as u32).to_be_bytes());
    frame.extend_from_slice(&json);
    if let Err(e) = ws.send(Message::Binary(frame.into())).await {
        Log::warn(&format!("火山 ASR 握手发送失败: {e}"));
        let _ = out.send(AsrOut::Final(String::new()));
        return;
    }

    let mut seq: i32 = 2;
    let mut pending: Vec<u8> = Vec::with_capacity(CHUNK_BYTES * 2);
    let mut last_text = String::new();
    let mut finished = false; // 已发最后一包
    let mut cancelled = false; // 被取消：静默退出，不发 Final
    let mut emitted = false; // Final 只发一次

    loop {
        tokio::select! {
            // 看门狗：录音阶段 60s 无任何音频/返回帧 → 连接假死，放弃会话，
            // 避免 unbounded 通道里 PCM 无限堆积（每轮 select 重新计时，
            // 有消息就重置，天然是空闲超时）。
            _ = tokio::time::sleep(std::time::Duration::from_secs(60)),
                if !finished && !cancelled => {
                Log::warn("火山 ASR 60 秒无活动，判定连接异常，结束本次会话");
                emit_final_once(out, &last_text, &mut emitted);
                return;
            }
            // 已发最后一包后服务端 10s 不回收尾帧 → 用已收到的结果收尾
            _ = tokio::time::sleep(std::time::Duration::from_secs(10)), if finished => {
                emit_final_once(out, &last_text, &mut emitted);
                return;
            }
            msg = input.recv() => {
                let Some(msg) = msg else { break }; // 通道关闭
                match msg {
                    SessionIn::Pcm(pcm) => {
                        if finished { continue; }
                        pending.extend_from_slice(&pcm);
                        while pending.len() >= CHUNK_BYTES {
                            let chunk: Vec<u8> = pending.drain(..CHUNK_BYTES).collect();
                            if !send_audio(&mut ws, &chunk, false, &mut seq).await {
                                emit_final_once(out, &last_text, &mut emitted);
                                return;
                            }
                        }
                    }
                    SessionIn::Finish => {
                        if !finished {
                            finished = true;
                            // 余量连同 last 标记一起发；没余量也发空包收尾
                            if !send_audio(&mut ws, &pending, true, &mut seq).await {
                                emit_final_once(out, &last_text, &mut emitted);
                                return;
                            }
                            pending.clear();
                        }
                    }
                    SessionIn::Cancel => {
                        cancelled = true;
                        break;
                    }
                }
            }
            frame = ws.next() => {
                match frame {
                    Some(Ok(Message::Binary(d))) => {
                        match handle_frame(&d, &mut last_text, out) {
                            FrameVerdict::ServerLast => {
                                emit_final_once(out, &last_text, &mut emitted);
                                return;
                            }
                            FrameVerdict::FatalError => {
                                emit_final_once(out, &last_text, &mut emitted);
                                return;
                            }
                            FrameVerdict::Ok => {}
                        }
                    }
                    Some(Ok(Message::Text(t))) => {
                        Log::debug(&format!("火山 ASR 文本帧: {}", &t[..t.len().min(200)]));
                    }
                    Some(Err(e)) => {
                        Log::warn(&format!("火山 ASR 连接错误: {e}"));
                        emit_final_once(out, &last_text, &mut emitted);
                        return;
                    }
                    Some(_) => {}
                    None => break, // 连接关闭
                }
            }
        }
    }
    if !cancelled {
        emit_final_once(out, &last_text, &mut emitted);
    }
}

/// 发一包音频；isLast 用负数 sequence（协议规定）
async fn send_audio(
    ws: &mut (impl futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    pcm: &[u8],
    is_last: bool,
    seq: &mut i32,
) -> bool {
    let s = if is_last { -*seq } else { *seq };
    let mut frame = header(
        msg_type::AUDIO_ONLY,
        if is_last { flags::NEG_SEQ } else { flags::POS_SEQ },
        0x0, // raw
        0x0,
    );
    frame.extend_from_slice(&s.to_be_bytes());
    frame.extend_from_slice(&(pcm.len() as u32).to_be_bytes());
    frame.extend_from_slice(pcm);
    if let Err(e) = ws.send(Message::Binary(frame.into())).await {
        Log::warn(&format!("火山 ASR 发送失败: {e}"));
        return false;
    }
    if !is_last {
        *seq += 1;
    }
    true
}

enum FrameVerdict {
    Ok,
    ServerLast,
    FatalError,
}

/// 解析服务端帧；更新 last_text，按需发 Partial
fn handle_frame(data: &[u8], last_text: &mut String, out: &AsrOutTx) -> FrameVerdict {
    if data.len() < 4 {
        return FrameVerdict::Ok;
    }
    let b1 = data[1];
    let mtype = b1 >> 4;
    let flg = b1 & 0x0F;
    let compression = data[2] & 0x0F;
    let mut i = 4usize;

    if mtype == msg_type::ERROR {
        if data.len() < i + 8 {
            return FrameVerdict::FatalError;
        }
        let code = be32(&data[i..]);
        i += 4;
        let size = be32(&data[i..]) as usize;
        i += 4;
        let msg = payload_string(data, i, size, compression == 1).unwrap_or_else(|| "?".into());
        Log::error(&format!("火山 ASR 错误 {code}: {msg}"));
        return FrameVerdict::FatalError;
    }

    if mtype != msg_type::FULL_SERVER && mtype != msg_type::SERVER_ACK {
        return FrameVerdict::Ok;
    }

    // flags 带 sequence 位时，payload 前多 4 字节序号
    if flg & 0x01 != 0 || flg & 0x02 != 0 {
        if data.len() < i + 4 {
            return FrameVerdict::Ok;
        }
        i += 4;
    }
    if data.len() < i + 4 {
        return FrameVerdict::Ok;
    }
    let size = be32(&data[i..]) as usize;
    i += 4;
    if size > 0 {
        if let Some(text) = payload_string(data, i, size, compression == 1) {
            // { "result": { "text": "…" } }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(t) = v
                    .get("result")
                    .and_then(|r| r.get("text"))
                    .and_then(|t| t.as_str())
                {
                    if !t.is_empty() && t != last_text {
                        *last_text = t.to_string();
                        let _ = out.send(AsrOut::Partial(last_text.clone()));
                    }
                }
            }
        }
    }

    // flags 第二位 = 服务端最后一包
    if flg & 0x02 != 0 {
        return FrameVerdict::ServerLast;
    }
    FrameVerdict::Ok
}

fn emit_final_once(out: &AsrOutTx, last_text: &str, emitted: &mut bool) {
    if !*emitted {
        *emitted = true;
        let _ = out.send(AsrOut::Final(last_text.to_string()));
    }
}

fn be32(d: &[u8]) -> u32 {
    u32::from_be_bytes(d[..4].try_into().unwrap_or([0; 4]))
}

fn payload_string(d: &[u8], from: usize, size: usize, gzipped: bool) -> Option<String> {
    let end = (from + size).min(d.len());
    if from >= end {
        return None;
    }
    let raw = &d[from..end];
    if gzipped {
        // gzip 解压（火山有时压缩响应体）
        let mut out = Vec::new();
        use std::io::Read;
        let mut dec = flate2::read::GzDecoder::new(raw);
        if dec.read_to_end(&mut out).is_ok() {
            return String::from_utf8(out).ok();
        }
    }
    String::from_utf8(raw.to_vec()).ok()
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("mojo-{nanos:x}")
}
