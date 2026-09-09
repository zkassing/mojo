//! 火山引擎流式大模型 ASR —— 凭证连通性测试。
//!
//! 打开 WebSocket，发送 fullClientRequest，等待服务端第一个响应（ack 或
//! 错误帧），用来验证 AppID / AccessToken / ResourceID 是否正确。
//! 不发送真实音频，验证通过即主动关闭。

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio_tungstenite::connect_async_tls_with_config;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const ENDPOINT: &str = "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel";

// 消息类型（高 4 位）
#[allow(dead_code)]
mod msg {
    pub const FULL_CLIENT: u8 = 0x1;
    pub const AUDIO_ONLY: u8 = 0x2;
    pub const FULL_SERVER: u8 = 0x9;
    pub const ERROR: u8 = 0xF;
}
#[allow(dead_code)]
mod flag {
    pub const POS_SEQ: u8 = 0x1;
    pub const NEG_SEQ: u8 = 0x2;
}

#[derive(Debug, Serialize)]
pub struct Credentials {
    pub app_id: String,
    pub access_token: String,
    pub resource_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
    pub latency_ms: u128,
}

fn header(t: u8, flags: u8) -> Vec<u8> {
    vec![
        0x11, // version 1, header size 1 (4 字节)
        (t << 4) | flags,
        0x10, // JSON 序列化，无压缩
        0x00,
    ]
}

fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

pub async fn test_connection(cred: Credentials) -> TestResult {
    let started = std::time::Instant::now();
    let result = run(cred).await;
    match result {
        Ok(msg) => TestResult {
            ok: true,
            message: msg,
            latency_ms: started.elapsed().as_millis(),
        },
        Err(e) => TestResult {
            ok: false,
            message: e,
            latency_ms: started.elapsed().as_millis(),
        },
    }
}

async fn run(cred: Credentials) -> Result<String, String> {
    let connect_id = uuid_like();
    let mut req = ENDPOINT
        .into_client_request()
        .map_err(|e| format!("构造请求失败: {e}"))?;
    {
        let h = req.headers_mut();
        h.insert("X-Api-App-Key", cred.app_id.parse().unwrap());
        h.insert("X-Api-Access-Key", cred.access_token.parse().unwrap());
        h.insert("X-Api-Resource-Id", cred.resource_id.parse().unwrap());
        h.insert("X-Api-Connect-Id", connect_id.parse().unwrap());
    }

    let (mut ws, _) = connect_async_tls_with_config(req, None, false, None)
        .await
        .map_err(|e| format!("连接失败（检查凭证 / 网络）: {e}"))?;

    // fullClientRequest
    let body = serde_json::json!({
        "user": { "uid": "miremote-panel" },
        "audio": { "format": "pcm", "codec": "raw", "rate": 16000, "bits": 16, "channel": 1 },
        "request": {
            "model_name": "bigmodel",
            "enable_itn": true,
            "enable_punc": false,
            "enable_ddc": false,
            "show_utterances": false,
            "result_type": "full"
        }
    });
    let json = serde_json::to_vec(&body).map_err(|e| e.to_string())?;

    let mut frame = header(msg::FULL_CLIENT, flag::POS_SEQ);
    frame.extend_from_slice(&be32(1));
    frame.extend_from_slice(&be32(json.len() as u32));
    frame.extend_from_slice(&json);

    ws.send(Message::Binary(frame.into()))
        .await
        .map_err(|e| format!("发送握手失败: {e}"))?;

    // 等服务端第一帧；最多 8 秒
    let reply = tokio::time::timeout(std::time::Duration::from_secs(8), ws.next())
        .await
        .map_err(|_| "等待响应超时".to_string())?
        .ok_or_else(|| "服务端关闭了连接".to_string())?
        .map_err(|e| format!("接收失败: {e}"))?;

    let data = match reply {
        Message::Binary(d) => d,
        Message::Text(t) => return Ok(format!("连接成功（文本响应）: {t}")),
        _ => return Err("收到非预期帧类型".into()),
    };

    if data.len() < 4 {
        return Err("响应帧过短".into());
    }
    let mtype = data[1] >> 4;
    if mtype == msg::ERROR {
        let msg = parse_error(&data).unwrap_or_else(|| "未知错误".into());
        return Err(msg);
    }

    // 收到 fullServerResponse / ack 都说明鉴权通过、会话已建立
    Ok("连接成功，凭证有效".into())
}

fn parse_error(data: &[u8]) -> Option<String> {
    // header(4) + seq(4, 若有) + code(4) + size(4) + payload
    let mut i = 4usize;
    let flags = data[1] & 0x0F;
    if flags & 0x3 != 0 {
        i += 4;
    }
    if data.len() < i + 8 {
        return None;
    }
    let code = u32::from_be_bytes(data[i..i + 4].try_into().ok()?);
    let size = u32::from_be_bytes(data[i + 4..i + 8].try_into().ok()?) as usize;
    i += 8;
    let payload = &data[i..(i + size).min(data.len())];
    let text = String::from_utf8_lossy(payload);
    Some(format!("服务端错误 {code}: {text}"))
}

/// 不引 uuid crate，生成一个够用的随机 connect id
fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("miremote-{nanos:x}")
}
