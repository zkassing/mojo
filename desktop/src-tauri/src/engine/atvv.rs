//! Google ATVV (Android TV Voice over BLE) 客户端 —— btleplug 版
//! （对应 Swift `ATVVClient.swift`，CoreBluetooth → btleplug）。
//!
//! 遥控器的麦克风不走 HID，而走独立 GATT 服务。macOS 上系统 HID 驱动和
//! 这条 GATT 链路可以并存，无需断开配对。
//!
//! 实测协议要点（小米蓝牙语音遥控器，固件 2671）：
//!   - notification 每包 120 字节裸 ADPCM，没有帧头
//!   - 解码器状态跨包连续，不能逐包重置
//!   - nibble 顺序是高 4 位先
//!   - IMA/DVI ADPCM 4-bit, 16 kHz, 16-bit, 单声道

use super::adpcm::AdpcmDecoder;
use super::asr::{SessionIn, SessionInTx};
use super::log::Log;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use btleplug::api::{
    Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, RetrievePeripheralsOptions,
    ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures_util::StreamExt;
use uuid::Uuid;

// MARK: GATT UUID

const SERVICE_UUID: Uuid = Uuid::from_u128(0xAB5E0001_5A21_4F05_BC7D_AF01F617B664);
const TX_UUID: Uuid = Uuid::from_u128(0xAB5E0002_5A21_4F05_BC7D_AF01F617B664);
const AUDIO_UUID: Uuid = Uuid::from_u128(0xAB5E0003_5A21_4F05_BC7D_AF01F617B664);
const CTL_UUID: Uuid = Uuid::from_u128(0xAB5E0004_5A21_4F05_BC7D_AF01F617B664);

/// 电池 / 设备信息服务（按名字找设备的兜底过滤）；16-bit SIG UUID → 蓝牙基址
const FALLBACK_SERVICES: [Uuid; 2] = [
    Uuid::from_u128(0x0000180F_0000_1000_8000_00805F9B34FB),
    Uuid::from_u128(0x0000180A_0000_1000_8000_00805F9B34FB),
];

// MARK: 协议操作码

mod op {
    pub const AUDIO_STOP: u8 = 0x00;
    pub const AUDIO_START: u8 = 0x04;
    pub const START_SEARCH: u8 = 0x08;
    pub const GET_CAPS: u8 = 0x0A;
    pub const CAPS_RESP: u8 = 0x0B;
    pub const MIC_OPEN: u8 = 0x0C;
    pub const MIC_CLOSE: u8 = 0x0D;
}

// MARK: 对外句柄

#[derive(Clone)]
pub enum AtvvCmd {
    OpenMic,
    CloseMic,
}

pub struct AtvvHandle {
    cmd: tokio::sync::mpsc::UnboundedSender<AtvvCmd>,
    /// 语音服务就绪（已连上遥控器且拿到 tx 特征）
    ready: Arc<AtomicBool>,
    /// 正在推流
    streaming: Arc<AtomicBool>,
    /// 当前 ASR 会话的输入通道（由 VoiceChain 注册/注销）
    audio: Arc<Mutex<Option<SessionInTx>>>,
}

impl AtvvHandle {
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    /// 主动开麦（on-request 模式）
    pub fn open_mic(&self) {
        let _ = self.cmd.send(AtvvCmd::OpenMic);
    }

    /// 关麦（遥控器松开按键时通常已自己发 AUDIO_STOP，这里兜底）
    pub fn close_mic(&self) {
        let _ = self.cmd.send(AtvvCmd::CloseMic);
    }

    /// 注册/注销当前会话的音频接收端
    pub fn set_audio_sink(&self, sink: Option<SessionInTx>) {
        *self.audio.lock().unwrap() = sink;
    }
}

/// 在 tokio 运行时上启动 ATVV 后台任务
pub fn spawn(rt: &tokio::runtime::Handle, device_name: String) -> AtvvHandle {
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<AtvvCmd>();
    let ready = Arc::new(AtomicBool::new(false));
    let streaming = Arc::new(AtomicBool::new(false));
    let audio = Arc::new(Mutex::new(None));

    let handle = AtvvHandle {
        cmd: cmd_tx,
        ready: ready.clone(),
        streaming: streaming.clone(),
        audio: audio.clone(),
    };

    rt.spawn(atvv_main(device_name, cmd_rx, ready, streaming, audio));
    handle
}

// MARK: 后台任务

async fn atvv_main(
    device_name: String,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<AtvvCmd>,
    ready: Arc<AtomicBool>,
    streaming: Arc<AtomicBool>,
    audio: Arc<Mutex<Option<SessionInTx>>>,
) {
    // TCC 防护：未授权蓝牙时初始化 CoreBluetooth 是否安全，取决于运行形态：
    // - .app 包：系统弹窗申请 ✓
    // - dev 裸二进制：TCC 归属父进程终端，终端的 Info.plist 没有蓝牙用途描述
    //   → 直接 SIGABRT 杀进程。此时宁可语音链路缺席也不能崩。
    while !bluetooth_usable() {
        Log::error(
            "蓝牙权限未授予，且当前是 dev 裸二进制（TCC 归属终端进程，无法弹窗申请）——\
             语音链路暂停（按键映射不受影响）。解决办法（任选其一，授权后 30 秒内自动恢复）：\
             ① 系统设置 → 隐私与安全性 → 蓝牙，给你的终端/App 授权；\
             ② 改用打包的 Mojo.app（pnpm tauri build），权限只授一次",
        );
        tokio::time::sleep(Duration::from_secs(30)).await;
    }

    let manager = match Manager::new().await {
        Ok(m) => m,
        Err(e) => {
            Log::error(&format!("ATVV 蓝牙管理器创建失败: {e}"));
            return;
        }
    };
    let Some(adapter) = manager.adapters().await.ok().and_then(|a| a.into_iter().next()) else {
        Log::error("ATVV 找不到蓝牙适配器");
        return;
    };
    let mut events = adapter.events().await.ok();

    loop {
        // 断开后持续重试连接，直到遥控器语音服务就绪
        // （遥控器唤醒时 HID 先起来、GATT 语音服务后就绪，单次重试经常扑空）
        use btleplug::api::CentralState;
        match adapter.adapter_state().await {
            Ok(CentralState::PoweredOn) => {}
            _ => {
                ready.store(false, Ordering::Relaxed);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        }

        match connect_session(&adapter, &device_name).await {
            Ok((peripheral, tx_char)) => {
                ready.store(true, Ordering::Relaxed);
                Log::info("语音通道已就绪（ATVV）");

                let mut notifs = match peripheral.notifications().await {
                    Ok(n) => n,
                    Err(e) => {
                        Log::warn(&format!("ATVV 订阅通知失败: {e}"));
                        ready.store(false, Ordering::Relaxed);
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                };

                let mut session = StreamState::new(streaming.clone(), audio.clone());
                // 握手：拿能力
                let caps = [op::GET_CAPS, 0x00, 0x01, 0x00, 0x03];
                if let Err(e) = peripheral
                    .write(&tx_char, &caps, WriteType::WithoutResponse)
                    .await
                {
                    Log::warn(&format!("ATVV 握手发送失败: {e}"));
                }

                loop {
                    tokio::select! {
                        n = notifs.next() => {
                            let Some(n) = n else { break }; // 通知流结束 = 断开
                            session.on_notification(n.uuid, &n.value);
                        }
                        c = cmd_rx.recv() => {
                            let Some(c) = c else { return }; // 句柄全 drop，任务退出
                            match c {
                                AtvvCmd::OpenMic => {
                                    if session.is_streaming() {
                                        Log::debug("ATVV 已在推流，跳过 MIC_OPEN");
                                    } else {
                                        session.reset();
                                        // 实测该固件吃 2 字节变体
                                        let cmd = [op::MIC_OPEN, 0x02];
                                        if let Err(e) = peripheral.write(&tx_char, &cmd, WriteType::WithoutResponse).await {
                                            Log::warn(&format!("ATVV MIC_OPEN 发送失败: {e}"));
                                        } else {
                                            Log::debug("ATVV → MIC_OPEN");
                                        }
                                    }
                                }
                                AtvvCmd::CloseMic => {
                                    if !session.is_streaming() {
                                        Log::debug("ATVV 已停止推流，无需 MIC_CLOSE");
                                    } else {
                                        let cmd = [op::MIC_CLOSE, session.stream_id];
                                        let _ = peripheral.write(&tx_char, &cmd, WriteType::WithoutResponse).await;
                                        Log::debug(&format!("ATVV → MIC_CLOSE (已收 {} 帧)", session.frame_count));
                                        session.flush();
                                    }
                                }
                            }
                        }
                        ev = async {
                            match events.as_mut() {
                                Some(s) => s.next().await,
                                None => std::future::pending().await,
                            }
                        } => {
                            if let Some(CentralEvent::DeviceDisconnected(id)) = ev {
                                if id == peripheral.id() {
                                    Log::info("ATVV 设备已断开");
                                    break;
                                }
                            }
                        }
                    }
                }

                session.flush();
                ready.store(false, Ordering::Relaxed);
                Log::info("ATVV 连接断开，将持续重试直到遥控器就绪");
            }
            Err(e) => {
                // 重试时会频繁走到这里，用 debug 避免刷日志
                Log::debug(&format!("ATVV 暂未找到已连接的「{device_name}」（{e}），稍后重试"));
            }
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// 找已连接的遥控器（已连接设备不广播，不能用扫描）并建立语音会话
async fn connect_session(
    adapter: &Adapter,
    device_name: &str,
) -> Result<(Peripheral, Characteristic), String> {

    // 先按 ATVV 服务过滤找已连接设备
    let mut candidates = adapter
        .retrieve_peripherals(RetrievePeripheralsOptions {
            identifiers: None,
            services: Some(vec![SERVICE_UUID]),
        })
        .await
        .map_err(|e| format!("retrieve_peripherals: {e}"))?;

    // 服务过滤拿不到时，退回按名字找（电池/设备信息服务过滤）
    if candidates.is_empty() {
        let by_name = adapter
            .retrieve_peripherals(RetrievePeripheralsOptions {
                identifiers: None,
                services: Some(FALLBACK_SERVICES.to_vec()),
            })
            .await
            .unwrap_or_default();
        for p in by_name {
            let name = p
                .properties()
                .await
                .ok()
                .flatten()
                .and_then(|props| props.local_name);
            if name.as_deref() == Some(device_name) {
                candidates.push(p);
            }
        }
    }

    // 名字匹配的优先，否则取第一个
    let mut chosen: Option<Peripheral> = None;
    for p in candidates {
        let name = p
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|props| props.local_name);
        if name.as_deref() == Some(device_name) {
            chosen = Some(p);
            break;
        }
        if chosen.is_none() {
            chosen = Some(p);
        }
    }
    let peripheral = chosen.ok_or_else(|| "未发现设备".to_string())?;
    Log::debug("ATVV 找到设备，建立连接…");

    peripheral.connect().await.map_err(|e| format!("连接失败: {e}"))?;
    peripheral
        .discover_services()
        .await
        .map_err(|e| format!("服务发现失败: {e}"))?;

    let chars = peripheral.characteristics();
    let find = |uuid: Uuid| chars.iter().find(|c| c.uuid == uuid).cloned();
    let tx_char = find(TX_UUID).ok_or_else(|| "缺少 tx 特征（该遥控器可能不支持语音）".to_string())?;
    let audio_char = find(AUDIO_UUID).ok_or_else(|| "缺少 audio 特征".to_string())?;
    let ctl_char = find(CTL_UUID).ok_or_else(|| "缺少 ctl 特征".to_string())?;

    peripheral
        .subscribe(&audio_char)
        .await
        .map_err(|e| format!("订阅 audio 失败: {e}"))?;
    peripheral
        .subscribe(&ctl_char)
        .await
        .map_err(|e| format!("订阅 ctl 失败: {e}"))?;

    Ok((peripheral, tx_char))
}

// MARK: 推流状态（一次连接内）

struct StreamState {
    streaming: Arc<AtomicBool>,
    audio: Arc<Mutex<Option<SessionInTx>>>,
    decoder: AdpcmDecoder,
    stream_id: u8,
    frame_count: u32,
}

impl StreamState {
    fn new(streaming: Arc<AtomicBool>, audio: Arc<Mutex<Option<SessionInTx>>>) -> Self {
        Self {
            streaming,
            audio,
            decoder: AdpcmDecoder::new(),
            stream_id: 0,
            frame_count: 0,
        }
    }

    fn is_streaming(&self) -> bool {
        self.streaming.load(Ordering::Relaxed)
    }

    fn reset(&mut self) {
        self.decoder = AdpcmDecoder::new();
        self.frame_count = 0;
    }

    /// 通知入口：audio 特征 → 解码转发；ctl 特征 → 协议状态机
    fn on_notification(&mut self, uuid: Uuid, data: &[u8]) {
        if uuid == AUDIO_UUID {
            if !self.is_streaming() || data.is_empty() {
                return;
            }
            self.frame_count += 1;
            // 整包都是 ADPCM 数据，无帧头；解码器状态跨包连续
            let pcm = self.decoder.decode(data);
            if let Some(tx) = self.audio.lock().unwrap().as_ref() {
                let _ = tx.send(SessionIn::Pcm(pcm));
            }
            return;
        }

        if uuid != CTL_UUID || data.is_empty() {
            return;
        }
        match data[0] {
            op::CAPS_RESP => {
                Log::debug(&format!(
                    "ATVV CAPS_RESP: {}",
                    data.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
                ));
            }
            op::AUDIO_START => {
                // reason 0x03 = 按住语音键（遥控器自发），其余 = MIC_OPEN 触发
                // 已在推流时不要重置，否则会丢掉开头的音频
                if self.is_streaming() {
                    Log::debug(&format!(
                        "ATVV AUDIO_START 重复到达，忽略（保留已录 {} 帧）",
                        self.frame_count
                    ));
                    return;
                }
                self.stream_id = data.get(3).copied().unwrap_or(0);
                self.reset();
                self.streaming.store(true, Ordering::Relaxed);
                let reason = data.get(1).copied().unwrap_or(0);
                Log::debug(&format!(
                    "ATVV AUDIO_START (reason={} stream={})",
                    if reason == 0x03 { "按住语音键" } else { "MIC_OPEN" },
                    self.stream_id
                ));
            }
            op::AUDIO_STOP => {
                Log::debug("ATVV AUDIO_STOP");
                self.flush();
            }
            op::START_SEARCH => Log::debug("ATVV START_SEARCH"),
            _ => {}
        }
    }

    /// 一段语音收尾：通知会话 finish（AUDIO_STOP 和 closeMic 都可能触发，去重）
    fn flush(&mut self) {
        if !self.streaming.swap(false, Ordering::Relaxed) {
            return;
        }
        if let Some(tx) = self.audio.lock().unwrap().as_ref() {
            let _ = tx.send(SessionIn::Finish);
        }
    }
}

/// 扫描过滤器占位（已连接设备不广播，正常用不到扫描）
#[allow(dead_code)]
fn _scan_filter() -> ScanFilter {
    ScanFilter::default()
}

/// 现在初始化 CoreBluetooth 是否安全（不会触发 TCC 崩溃）
///
/// CBManagerAuthorization: 0 未决定 / 1 受限 / 2 拒绝 / 3 已授权
/// - 已授权：直接安全
/// - 未决定：.app 包可以（系统弹窗）；dev 裸二进制会崩（见上）
/// - 拒绝/受限：不崩但也不可用 —— 同样拦下等用户改授权
#[cfg(target_os = "macos")]
fn bluetooth_usable() -> bool {
    use objc2::runtime::AnyClass;

    let Some(cls) = AnyClass::get(c"CBCentralManager") else {
        return true; // CoreBluetooth 尚未加载，拦不住也不该拦
    };
    let auth: isize = unsafe { objc2::msg_send![cls, authorization] };
    if auth == 3 {
        return true;
    }
    // 未决定/拒绝/受限：只有打包应用才有资格（弹窗或引导用户去设置）
    is_bundled_app()
}

#[cfg(not(target_os = "macos"))]
fn bluetooth_usable() -> bool {
    true
}

/// 当前进程是否跑在 .app 包里
#[cfg(target_os = "macos")]
fn is_bundled_app() -> bool {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
        .unwrap_or(false)
}
