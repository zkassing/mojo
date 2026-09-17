//! Linux 引擎驱动：evdev 独占抓取 + uinput 回注（对照 macos/mod.rs）。
//!
//! 与 macOS（CGEventTap 全局钩子 + senderID 过滤）不同，evdev 提供真正的按设备
//! 过滤：对遥控器的 event 节点做 EVIOCGRAB 独占抓取后，原始事件只对引擎可见；
//! **未映射的事件必须经 uinput 虚拟设备原样转发**（否则遥控器的普通按键在引擎
//! 运行期间会死掉）。转发虚拟设备的能力集克隆自所有抓取节点的并集。
//!
//! 结构：
//! - `LinuxEngine`：对外句柄（PlatformEngine impl），管理引擎线程生命周期
//! - 引擎线程：单线程事件循环 —— 按键事件 / 定时器 / 命令（Reload、Stop）走同一条 mpsc
//! - 读线程：每个抓取节点一个（非阻塞轮询读 → 事件转发到引擎通道；
//!   Device 随线程退出 drop，EVIOCGRAB 自动解除）
//! - 按键定时器：「生成计数 + 睡线程」——槽位存 generation，调度时 spawn 线程
//!   sleep 后回消息（带 generation），取消 = generation+1，引擎线程校验后执行
//!
//! 语音链路（ATVV/ASR/LiveTyper）在 engine/voice.rs，按住 dictate 键接入。

pub mod emit;

use crate::config::Config;
use crate::engine::action::{self, Act};
use crate::engine::log::{Log, VERBOSE};
use crate::engine::state::{BindingShape, Eff, Machine, Timing};
use crate::engine::{uses_dictate, EngineStatus, PlatformEngine};

use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{
    AttributeSet, Device, EventType, InputEvent, InputEventKind, Key, MiscType, RelativeAxisType,
};

use std::collections::{HashMap, HashSet};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// 设备重扫间隔（遥控器热插拔 / 连接状态检测）
const RESCAN_INTERVAL: Duration = Duration::from_secs(3);

/// 权限不足时的操作指引（启动预检报错与抓取失败告警共用）
const PERMISSION_HINT: &str = "请检查设备节点权限（任选其一）：
  1) 把当前用户加入 input 组，然后重新登录：
       sudo usermod -aG input $USER
  2) 或添加 udev 规则 /etc/udev/rules.d/99-mojo.rules：
       KERNEL==\"uinput\", MODE=\"0660\", GROUP=\"input\"
       SUBSYSTEM==\"input\", ATTRS{idVendor}==\"2717\", ATTRS{idProduct}==\"32b8\", MODE=\"0660\", GROUP=\"input\"
     然后执行：sudo udevadm control --reload && sudo udevadm trigger";

// MARK: - 引擎消息（命令 / 事件 / 定时器同一条通道）

enum EngineMsg {
    /// 配置热重载
    Reload(Config),
    /// 停止引擎
    Stop,
    /// 读线程收到的一个原始事件（path 仅用于日志与节点管理）
    Ev(PathBuf, InputEvent),
    /// 读线程读失败（设备掉线等）；读线程已退出，Device 已 drop（抓取解除）
    NodeGone(PathBuf, String),
    /// 按键定时器触发（generation 与槽位不符则被引擎线程丢弃）
    Timer {
        button: String,
        kind: KeyTimerKind,
        generation: u64,
        action: Option<Act>,
    },
}

// MARK: - 按键定时器

#[derive(Clone, Copy, PartialEq)]
enum KeyTimerKind {
    Long,
    TapFallback,
    Repeat,
}

struct KeyTimerSlot {
    /// 代数：调度一次 +1；陈旧消息据此丢弃
    generation: u64,
    /// 协作式取消标志（连发定时器线程循环里检查，收到即退出）
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct KeyTimers {
    long: Option<KeyTimerSlot>,
    tap_fallback: Option<KeyTimerSlot>,
    repeat: Option<KeyTimerSlot>,
}

impl KeyTimers {
    fn slot_mut(&mut self, kind: KeyTimerKind) -> &mut Option<KeyTimerSlot> {
        match kind {
            KeyTimerKind::Long => &mut self.long,
            KeyTimerKind::TapFallback => &mut self.tap_fallback,
            KeyTimerKind::Repeat => &mut self.repeat,
        }
    }

    fn slot(&self, kind: KeyTimerKind) -> &Option<KeyTimerSlot> {
        match kind {
            KeyTimerKind::Long => &self.long,
            KeyTimerKind::TapFallback => &self.tap_fallback,
            KeyTimerKind::Repeat => &self.repeat,
        }
    }
}

// MARK: - 对外句柄

struct Shared {
    cmd_tx: Mutex<Option<mpsc::Sender<EngineMsg>>>,
    running: Mutex<bool>,
    remote_connected: Mutex<bool>,
}

pub struct LinuxEngine {
    shared: Arc<Shared>,
}

impl LinuxEngine {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                cmd_tx: Mutex::new(None),
                running: Mutex::new(false),
                remote_connected: Mutex::new(false),
            }),
        }
    }
}

impl PlatformEngine for LinuxEngine {
    fn supported(&self) -> bool {
        true
    }

    fn start(&self, config: &Config) -> anyhow::Result<()> {
        // 幂等：已在运行 → 走热重载
        {
            let running = *self.shared.running.lock().unwrap();
            let tx_guard = self.shared.cmd_tx.lock().unwrap();
            if running {
                if let Some(tx) = tx_guard.as_ref() {
                    let _ = tx.send(EngineMsg::Reload(config.clone()));
                    return Ok(());
                }
            }
        }
        precheck_permissions(config)?;
        let (tx, rx) = mpsc::channel::<EngineMsg>();
        *self.shared.cmd_tx.lock().unwrap() = Some(tx.clone());
        let shared = self.shared.clone();
        let config = config.clone();
        std::thread::Builder::new()
            .name("mojo-engine".into())
            .spawn(move || engine_thread_main(config, tx, rx, shared))?;
        Ok(())
    }

    fn stop(&self) {
        if let Some(tx) = self.shared.cmd_tx.lock().unwrap().as_ref() {
            let _ = tx.send(EngineMsg::Stop);
        }
    }

    fn status(&self) -> EngineStatus {
        if *self.shared.running.lock().unwrap() {
            EngineStatus::Running {
                daemon_connected: true,
                remote_connected: *self.shared.remote_connected.lock().unwrap(),
            }
        } else {
            EngineStatus::Stopped
        }
    }
}

// MARK: - 权限自检

/// 启动前检查：/dev/uinput 可写 + 已连接的遥控器 event 节点可读写。
/// 不可用时返回带操作指引的 Err（并同步打日志）。
fn precheck_permissions(config: &Config) -> anyhow::Result<()> {
    // 1) /dev/uinput 可写（按键注入与未映射事件转发都靠它）
    if let Err(e) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/uinput")
    {
        let msg = format!("无法打开 /dev/uinput：{e}\n{PERMISSION_HINT}");
        Log::error(&msg);
        return Err(anyhow::anyhow!(msg));
    }
    // 2) 遥控器节点可读写（evdev Device::open 用 O_RDWR）。
    //    设备不在线时节点列表为空：跳过，引擎启动后靠重扫接入。
    for node in find_remote_nodes_sysfs(config.device.vendor_id, config.device.product_id) {
        if let Err(e) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&node)
        {
            let msg = format!(
                "遥控器节点 {} 无法读写：{e}\n{PERMISSION_HINT}",
                node.display()
            );
            Log::error(&msg);
            return Err(anyhow::anyhow!(msg));
        }
    }
    Ok(())
}

/// 经 sysfs 按 VID/PID 找遥控器的 event 节点（读 sysfs 不需要设备节点权限，
/// 专门用于权限预检发现「在线但不可读」的情况）。
/// /sys/class/input/eventN/device/id/{vendor,product} 内容形如 "2717"（无 0x 前缀）。
fn find_remote_nodes_sysfs(vendor: u32, product: u32) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/sys/class/input") else {
        return out;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("event") {
            continue;
        }
        let id_dir = entry.path().join("device/id");
        let read_hex = |file: &str| -> Option<u32> {
            let s = std::fs::read_to_string(id_dir.join(file)).ok()?;
            u32::from_str_radix(s.trim(), 16).ok()
        };
        if read_hex("vendor") == Some(vendor) && read_hex("product") == Some(product) {
            out.push(PathBuf::from(format!("/dev/input/{name}")));
        }
    }
    out
}

// MARK: - 引擎线程

fn engine_thread_main(
    config: Config,
    tx: mpsc::Sender<EngineMsg>,
    rx: mpsc::Receiver<EngineMsg>,
    shared: Arc<Shared>,
) {
    Log::info(&format!(
        "Rust 引擎启动（设备 0x{:04x}/0x{:04x}，{} 个按键，{} 个方案）",
        config.device.vendor_id,
        config.device.product_id,
        config.buttons.len(),
        config.profiles.len()
    ));
    VERBOSE.store(config.options.verbose, Ordering::Relaxed);

    let mut core = EngineCore::new(config, tx, shared.clone());
    core.post_init();
    *shared.running.lock().unwrap() = true;

    // 主循环：收消息；RESCAN_INTERVAL 无消息则重扫设备（热插拔 / 连接状态）
    loop {
        match rx.recv_timeout(RESCAN_INTERVAL) {
            Ok(EngineMsg::Stop) => break,
            Ok(EngineMsg::Reload(cfg)) => core.reload(cfg),
            Ok(EngineMsg::Ev(path, ev)) => core.handle_ev(&path, ev),
            Ok(EngineMsg::NodeGone(path, err)) => core.handle_node_gone(&path, &err),
            Ok(EngineMsg::Timer {
                button,
                kind,
                generation,
                action,
            }) => core.handle_timer(&button, kind, generation, action),
            Err(mpsc::RecvTimeoutError::Timeout) => core.scan_devices(),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    core.shutdown();
    *shared.running.lock().unwrap() = false;
    *shared.remote_connected.lock().unwrap() = false;
    Log::info("Rust 引擎已停止");
}

// MARK: - 读线程

struct ReaderHandle {
    path: PathBuf,
    shutdown: Arc<AtomicBool>,
    join: std::thread::JoinHandle<()>,
}

/// 读线程主循环：非阻塞轮询读 → 事件转发到引擎通道。
/// 退出时 Device drop，EVIOCGRAB 随之解除。
fn reader_loop(
    dev: &mut Device,
    path: &Path,
    tx: mpsc::Sender<EngineMsg>,
    shutdown: Arc<AtomicBool>,
) {
    // 切非阻塞：配合停止标志轮询，引擎停止时无需等按键也能及时退出（并解除抓取）。
    // SAFETY: fd 属于本线程独占的 Device（引擎线程只持有路径，不碰 fd）；
    // F_GETFL/F_SETFL 仅在原标志上追加 O_NONBLOCK，失败也只是退化为阻塞读。
    let fd = dev.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        match dev.fetch_events() {
            Ok(events) => {
                for ev in events {
                    if tx.send(EngineMsg::Ev(path.to_path_buf(), ev)).is_err() {
                        return; // 引擎线程已退出
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(30));
            }
            Err(e) => {
                // ENODEV（设备拔掉）等：上报引擎线程后退出
                let _ = tx.send(EngineMsg::NodeGone(path.to_path_buf(), e.to_string()));
                return;
            }
        }
    }
}

// MARK: - EngineCore

struct EngineCore {
    config: Config,
    raw_to_button: HashMap<String, String>,
    timing: Timing,
    machine: Machine,
    emitter: emit::Emitter,
    /// 回引擎通道的克隆端（读线程 / 定时器线程用）
    tx: mpsc::Sender<EngineMsg>,
    shared: Arc<Shared>,
    /// 已独占抓取的节点路径（去重；Device 本体在各自读线程里）
    grabbed: HashSet<PathBuf>,
    readers: Vec<ReaderHandle>,
    /// 未映射事件的转发虚拟设备（能力 = 所有抓取节点的并集，按需重建）
    forward: Option<VirtualDevice>,
    fwd_keys: AttributeSet<Key>,
    fwd_rels: AttributeSet<RelativeAxisType>,
    fwd_miscs: AttributeSet<MiscType>,
    key_timers: HashMap<String, KeyTimers>,
    timer_seq: u64,
    /// 语音链路（配置含 dictate 绑定时才初始化）
    voice: Option<crate::engine::voice::VoiceChain>,
    /// 当前按住 dictate 的键位
    dictating_button: Option<String>,
    remote_connected: bool,
    /// 抓取权限告警每段运行期只发一次（重扫每 3 秒一次，避免刷屏）
    grab_warn_logged: bool,
}

impl EngineCore {
    fn new(config: Config, tx: mpsc::Sender<EngineMsg>, shared: Arc<Shared>) -> Self {
        let timing = Timing {
            long_press: Duration::from_millis(config.options.long_press_ms as u64),
            double_press: Duration::from_millis(config.options.double_press_ms as u64),
            debounce: Duration::from_millis(config.options.debounce_ms as u64),
        };
        let raw_to_button = config.raw_to_button();
        Self {
            config,
            raw_to_button,
            timing,
            machine: Machine::new(),
            emitter: emit::Emitter::new(),
            tx,
            shared,
            grabbed: HashSet::new(),
            readers: Vec::new(),
            forward: None,
            fwd_keys: AttributeSet::new(),
            fwd_rels: AttributeSet::new(),
            fwd_miscs: AttributeSet::new(),
            key_timers: HashMap::new(),
            timer_seq: 0,
            voice: None,
            dictating_button: None,
            remote_connected: false,
            grab_warn_logged: false,
        }
    }

    /// 引擎线程起跑前的初始化（对应 macOS post_init）
    fn post_init(&mut self) {
        // 电源键：Linux v1 不支持（logind inhibitor 是另一套机制），配置绑了 power 给警告
        if config_binds_power_key(&self.config) {
            Log::warn("Linux 驱动暂不支持电源键拦截，power 绑定不会生效");
        }
        // 语音链路：仅当配置了 dictate 绑定时初始化（同 macOS post_init）
        if uses_dictate(&self.config) {
            self.init_voice();
        }
        self.scan_devices();
        if !self.remote_connected {
            Log::info(&format!(
                "暂未发现遥控器（VID 0x{:04x} / PID 0x{:04x}），连接后将自动接入",
                self.config.device.vendor_id, self.config.device.product_id
            ));
        }
    }

    fn init_voice(&mut self) {
        let name = self
            .config
            .device
            .name
            .clone()
            .unwrap_or_else(|| "小米蓝牙语音遥控器".to_string());
        if self.config.voice.uses_sherpa() {
            Log::info("语音引擎: sherpa-onnx 本地流式（免费离线）");
            crate::engine::asr::sherpa::prewarm(&self.config.voice.sherpa_dir());
        } else if self.config.voice.uses_volc() {
            Log::info("语音引擎: 火山引擎流式大模型");
        } else {
            Log::warn("未选择识别引擎（在面板的语音识别页选 火山 或 sherpa）");
        }
        self.voice = Some(crate::engine::voice::VoiceChain::start(&name, || {
            Box::new(emit::Emitter::new())
        }));
    }

    /// 按住开始录音，松开识别上屏（端口 macOS handleDictate）
    fn handle_dictate(&mut self, button: &str, is_down: bool, profile: &str) {
        let Some(voice) = &self.voice else {
            Log::warn("语音通道未初始化");
            return;
        };
        if is_down {
            if self.dictating_button.is_some() {
                return; // 防重入
            }
            if !voice.is_ready() {
                Log::warn("语音通道未就绪（遥控器可能休眠，先按任意键唤醒）");
                return;
            }
            self.dictating_button = Some(button.to_string());
            Log::info(&format!("[{profile}] {button} 按住 → 开始录音"));
            voice.dictate_start(&self.config);
        } else {
            if self.dictating_button.as_deref() != Some(button) {
                return;
            }
            self.dictating_button = None;
            Log::info(&format!("[{profile}] {button} 松开 → 识别中"));
            voice.dictate_stop();
        }
    }

    fn shutdown(&mut self) {
        if let Some(v) = &self.voice {
            v.shutdown();
        }
        self.voice = None;
        self.dictating_button = None;
        self.cancel_all_key_timers();
        // 停读线程：置标志 + join（非阻塞循环 30ms 内退出）；
        // Device 随线程函数作用域 drop，EVIOCGRAB 自动解除
        for r in self.readers.drain(..) {
            r.shutdown.store(true, Ordering::Relaxed);
            let _ = r.join.join();
        }
        self.grabbed.clear();
        // 转发虚拟设备随引擎退出销毁
        self.forward = None;
    }

    // MARK: 设备发现 / 独占抓取

    /// 扫 /dev/input/event*，匹配 VID/PID 且具备 EV_KEY 能力的节点全部抓取。
    /// 同时据「是否有匹配节点」维护遥控器连接状态。每 RESCAN_INTERVAL 跑一次。
    fn scan_devices(&mut self) {
        let vendor = self.config.device.vendor_id;
        let product = self.config.device.product_id;
        let mut present = false;
        for (path, mut dev) in evdev::enumerate() {
            let id = dev.input_id();
            if id.vendor() as u32 != vendor || id.product() as u32 != product {
                continue;
            }
            present = true;
            if self.grabbed.contains(&path) {
                continue;
            }
            // BLE HID 常拆成多个节点（键盘/消费控制/鼠标），只抓有按键能力的；
            // 无按键能力的节点（如纯鼠标）事件仍走系统，不影响映射
            if !dev.supported_events().contains(EventType::KEY) {
                continue;
            }
            let name = dev.name().unwrap_or("未命名设备").to_string();
            match dev.grab() {
                Ok(()) => {
                    Log::info(&format!("已独占抓取 {}（{}）", path.display(), name));
                    self.grab_warn_logged = false;
                    self.merge_forward_caps(&dev);
                    self.spawn_reader(path, dev);
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::PermissionDenied {
                        if !self.grab_warn_logged {
                            self.grab_warn_logged = true;
                            Log::warn(&format!(
                                "抓取 {} 权限不足：{e}\n{PERMISSION_HINT}",
                                path.display()
                            ));
                        }
                    } else {
                        Log::warn(&format!("抓取 {} 失败：{e}", path.display()));
                    }
                }
            }
        }
        self.set_remote_connected(present);
    }

    /// 把新抓取节点的能力并入转发虚拟设备（keys / 相对轴 / MSC），有新增则重建
    fn merge_forward_caps(&mut self, dev: &Device) {
        let mut grew = false;
        if let Some(keys) = dev.supported_keys() {
            for k in keys.iter() {
                if !self.fwd_keys.contains(k) {
                    self.fwd_keys.insert(k);
                    grew = true;
                }
            }
        }
        if let Some(rels) = dev.supported_relative_axes() {
            for r in rels.iter() {
                if !self.fwd_rels.contains(r) {
                    self.fwd_rels.insert(r);
                    grew = true;
                }
            }
        }
        if let Some(miscs) = dev.misc_properties() {
            for m in miscs.iter() {
                if !self.fwd_miscs.contains(m) {
                    self.fwd_miscs.insert(m);
                    grew = true;
                }
            }
        }
        if grew || self.forward.is_none() {
            self.rebuild_forward();
        }
    }

    /// 用当前能力并集重建转发虚拟设备（uinput 能力集只能在创建时设定）
    fn rebuild_forward(&mut self) {
        let keys = std::mem::take(&mut self.fwd_keys);
        let rels = std::mem::take(&mut self.fwd_rels);
        let miscs = std::mem::take(&mut self.fwd_miscs);
        // 注意：虚拟设备不设 input_id（vendor/product 默认 0），
        // 避免重扫时被自己误判成遥控器形成回环
        let result = (|| -> std::io::Result<VirtualDevice> {
            let mut b = VirtualDeviceBuilder::new()?.name("mojo-forward");
            if keys.iter().next().is_some() {
                b = b.with_keys(&keys)?;
            }
            if rels.iter().next().is_some() {
                b = b.with_relative_axes(&rels)?;
            }
            if miscs.iter().next().is_some() {
                b = b.with_msc(&miscs)?;
            }
            b.build()
        })();
        self.fwd_keys = keys;
        self.fwd_rels = rels;
        self.fwd_miscs = miscs;
        match result {
            Ok(dev) => self.forward = Some(dev),
            Err(e) => {
                self.forward = None;
                Log::error(&format!(
                    "创建转发虚拟设备失败：{e}（未映射按键将无法透传）"
                ));
            }
        }
    }

    /// 未映射/放行的事件原样转发回系统
    fn forward(&mut self, ev: &InputEvent) {
        let Some(dev) = &mut self.forward else {
            return;
        };
        if let Err(e) = dev.emit(std::slice::from_ref(ev)) {
            Log::warn(&format!("转发事件失败: {e}"));
        }
    }

    fn spawn_reader(&mut self, path: PathBuf, dev: Device) {
        let mut dev = dev;
        let tx = self.tx.clone();
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = shutdown.clone();
        let thread_name = path
            .file_name()
            .map(|s| format!("mojo-evdev-{}", s.to_string_lossy()))
            .unwrap_or_else(|| "mojo-evdev".into());
        let path_in_thread = path.clone();
        let join = std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || reader_loop(&mut dev, &path_in_thread, tx, flag));
        match join {
            Ok(join) => {
                self.grabbed.insert(path.clone());
                self.readers.push(ReaderHandle {
                    path,
                    shutdown,
                    join,
                });
            }
            Err(e) => {
                // spawn 失败：dev 在此 drop，抓取随之解除
                Log::error(&format!("启动读取线程失败：{e}"));
            }
        }
    }

    fn handle_node_gone(&mut self, path: &Path, err: &str) {
        Log::warn(&format!(
            "{} 读取失败：{err}（遥控器可能已断开）",
            path.display()
        ));
        self.grabbed.remove(path);
        if let Some(i) = self.readers.iter().position(|r| r.path.as_path() == path) {
            let r = self.readers.remove(i);
            r.shutdown.store(true, Ordering::Relaxed);
            // 读线程已退出（它是上报方），join 立即返回
            let _ = r.join.join();
        }
        // 所有抓取节点都读失败 → 视为断开（重扫会继续确认/重连）
        if self.readers.is_empty() {
            self.set_remote_connected(false);
        }
    }

    fn set_remote_connected(&mut self, connected: bool) {
        if self.remote_connected != connected {
            self.remote_connected = connected;
            *self.shared.remote_connected.lock().unwrap() = connected;
            if connected {
                Log::info("遥控器已连接");
            } else {
                Log::info("遥控器已断开");
            }
        }
    }

    // MARK: 配置热重载

    fn reload(&mut self, cfg: Config) {
        self.machine.reset();
        self.cancel_all_key_timers();
        VERBOSE.store(cfg.options.verbose, Ordering::Relaxed);
        self.timing = Timing {
            long_press: Duration::from_millis(cfg.options.long_press_ms as u64),
            double_press: Duration::from_millis(cfg.options.double_press_ms as u64),
            debounce: Duration::from_millis(cfg.options.debounce_ms as u64),
        };

        if config_binds_power_key(&cfg) {
            Log::warn("Linux 驱动暂不支持电源键拦截，power 绑定不会生效");
        }

        Log::info(&format!(
            "配置已重新加载（{} 个按键，{} 个方案）",
            cfg.buttons.len(),
            cfg.profiles.len()
        ));
        // 语音链路：之前没有、现在配置了 dictate 绑定 → 初始化
        if self.voice.is_none() && uses_dictate(&cfg) {
            self.init_voice();
        }

        self.raw_to_button = cfg.raw_to_button();
        self.config = cfg;
    }

    // MARK: 按键定时器管理

    fn start_key_timer(
        &mut self,
        button: &str,
        kind: KeyTimerKind,
        delay: Duration,
        interval: Duration,
        action: Option<Act>,
    ) {
        self.cancel_key_timer(button, kind);
        self.timer_seq = self.timer_seq.wrapping_add(1);
        let generation = self.timer_seq;
        let cancel = Arc::new(AtomicBool::new(false));
        self.key_timers
            .entry(button.to_string())
            .or_default()
            .slot_mut(kind)
            .replace(KeyTimerSlot {
                generation,
                cancel: cancel.clone(),
            });
        let tx = self.tx.clone();
        let button_owned = button.to_string();
        let repeating = interval > Duration::ZERO;
        let _ = std::thread::Builder::new()
            .name(format!("mojo-keytimer-{button}"))
            .spawn(move || {
                std::thread::sleep(delay);
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    // 引擎线程退出后发送失败，直接结束（代数校验双保险在引擎侧）
                    if tx
                        .send(EngineMsg::Timer {
                            button: button_owned.clone(),
                            kind,
                            generation,
                            action: action.clone(),
                        })
                        .is_err()
                    {
                        return;
                    }
                    if !repeating {
                        return;
                    }
                    std::thread::sleep(interval);
                }
            });
    }

    fn cancel_key_timer(&mut self, button: &str, kind: KeyTimerKind) {
        if let Some(timers) = self.key_timers.get_mut(button) {
            if let Some(slot) = timers.slot_mut(kind).take() {
                slot.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    /// 一次性定时器触发时摘出槽位（不动 cancel —— 线程发完消息已自行退出）
    fn take_timer_slot(&mut self, button: &str, kind: KeyTimerKind) {
        if let Some(timers) = self.key_timers.get_mut(button) {
            timers.slot_mut(kind).take();
        }
    }

    fn cancel_all_key_timers(&mut self) {
        let buttons: Vec<String> = self.key_timers.keys().cloned().collect();
        for b in buttons {
            for kind in [
                KeyTimerKind::Long,
                KeyTimerKind::TapFallback,
                KeyTimerKind::Repeat,
            ] {
                self.cancel_key_timer(&b, kind);
            }
        }
        self.key_timers.clear();
    }

    /// 定时器消息入口：先校验代数，再按种类驱动状态机（对应 macOS key_timer_cb）
    fn handle_timer(
        &mut self,
        button: &str,
        kind: KeyTimerKind,
        generation: u64,
        action: Option<Act>,
    ) {
        let current = self
            .key_timers
            .get(button)
            .and_then(|t| t.slot(kind).as_ref())
            .map(|s| s.generation);
        if current != Some(generation) {
            return; // 已换代的陈旧定时器
        }
        match kind {
            KeyTimerKind::Long => {
                self.take_timer_slot(button, KeyTimerKind::Long);
                for eff in self.machine.long_timeout(button) {
                    match eff {
                        Eff::CancelTapFallback => {
                            self.cancel_key_timer(button, KeyTimerKind::TapFallback)
                        }
                        Eff::PerformLong => {
                            if let Some(a) = &action {
                                self.emitter.perform(a);
                            }
                        }
                        _ => {}
                    }
                }
            }
            KeyTimerKind::TapFallback => {
                self.take_timer_slot(button, KeyTimerKind::TapFallback);
                for eff in self.machine.tap_timeout(button, action.is_some()) {
                    if let (Eff::PerformTap, Some(a)) = (eff, &action) {
                        self.emitter.perform(a);
                    }
                }
            }
            KeyTimerKind::Repeat => {
                // 连发：松开按键时 StopRepeat 会取消本定时器
                if let Some(a) = &action {
                    self.emitter.perform(a);
                }
            }
        }
    }

    // MARK: evdev 事件入口

    fn handle_ev(&mut self, path: &Path, ev: InputEvent) {
        match ev.kind() {
            InputEventKind::Key(key) => {
                let value = ev.value(); // 0=松开 1=按下 2=按住自动重复
                let (signal, pretty) = signal_of_key(key, ev.code());
                let phase = match value {
                    0 => "松开",
                    1 => "按下",
                    _ => "重复",
                };
                Log::debug(&format!("{pretty} {phase} [{}]", path.display()));
                if !self.raw_to_button.contains_key(&signal) {
                    // 未映射：原样转发（SYN 由 emit 自动补）
                    self.forward(&ev);
                    return;
                }
                let consumed = self.route_signal(&signal, value != 0);
                if !(consumed && self.config.options.swallow_original) {
                    self.forward(&ev);
                }
            }
            // SYN 不转发：VirtualDevice::emit 每批自动补 SYN_REPORT
            InputEventKind::Synchronization(_) => {}
            // 相对轴 / 绝对轴 / MSC 等其余事件一律原样转发
            other => {
                Log::debug(&format!("转发非按键事件 {other:?} [{}]", path.display()));
                self.forward(&ev);
            }
        }
    }

    // MARK: 信号路由

    /// 返回 true 表示事件已被映射消费（据此决定是否吞掉原键）
    fn route_signal(&mut self, signal_id: &str, is_down: bool) -> bool {
        let Some(button) = self.raw_to_button.get(&signal_id.to_lowercase()).cloned() else {
            return false;
        };
        let (bid, bname) = frontmost_app();
        let Some(profile) = self
            .config
            .resolve_profile(bid.as_deref(), bname.as_deref())
        else {
            return false;
        };
        let Some(binding) = profile.bindings.get(&button) else {
            if is_down {
                Log::info(&format!("[{}] {button} 无绑定，放行原键", profile.name));
            }
            return false;
        };
        let profile_name = profile.name.clone();
        let binding = binding.clone();

        // 显式 passthrough：不做任何映射，原样放行（不受 swallowOriginal 影响）
        if action::binding_is_passthrough(&binding) {
            if is_down {
                Log::info(&format!(
                    "[{profile_name}] {button} → 放行原键（passthrough）"
                ));
            }
            return false;
        }

        // 纯 dictate：按住-松开语义，不进单击/长按状态机
        if action::binding_is_pure_dictate(&binding) {
            self.handle_dictate(&button, is_down, &profile_name);
            return true;
        }

        let shape = BindingShape {
            has_tap: binding.tap.is_some(),
            has_long: binding.long.is_some(),
            has_double: binding.double.is_some(),
            repeat: binding.repeat.unwrap_or(false),
        };
        let now = Instant::now();
        let effs = if is_down {
            self.machine.press(&button, shape, self.timing, now)
        } else {
            self.machine.release(&button, shape, self.timing, now)
        };
        let verbose_actions = self.config.options.verbose;
        for eff in effs {
            match eff {
                Eff::PerformTap => {
                    if let Some(a) = &binding.tap {
                        let act = Act::normalize(a);
                        Log::info(&format!("[{profile_name}] {button} → {}", act.describe()));
                        self.emitter.perform(&act);
                    }
                }
                Eff::PerformLong => { /* 由长按定时器消息执行（动作在消息里） */ }
                Eff::PerformDouble => {
                    if let Some(a) = &binding.double {
                        let act = Act::normalize(a);
                        Log::info(&format!(
                            "[{profile_name}] {button} 双击 → {}",
                            act.describe()
                        ));
                        self.emitter.perform(&act);
                    }
                }
                Eff::ScheduleLong(d) => {
                    let act = binding.long.as_ref().map(Act::normalize);
                    if let Some(a) = &act {
                        Log::info(&format!(
                            "[{profile_name}] {button} 长按 → {}",
                            a.describe()
                        ));
                    }
                    self.start_key_timer(&button, KeyTimerKind::Long, d, Duration::ZERO, act);
                }
                Eff::ScheduleTapFallback(d) => {
                    let act = binding.tap.as_ref().map(Act::normalize);
                    self.start_key_timer(
                        &button,
                        KeyTimerKind::TapFallback,
                        d,
                        Duration::ZERO,
                        act,
                    );
                }
                Eff::CancelLong => self.cancel_key_timer(&button, KeyTimerKind::Long),
                Eff::CancelTapFallback => self.cancel_key_timer(&button, KeyTimerKind::TapFallback),
                Eff::StartRepeat { initial, interval } => {
                    let act = binding.tap.as_ref().map(Act::normalize);
                    if verbose_actions {
                        Log::debug(&format!("{button} 按住连发"));
                    }
                    self.start_key_timer(&button, KeyTimerKind::Repeat, initial, interval, act);
                }
                Eff::StopRepeat => self.cancel_key_timer(&button, KeyTimerKind::Repeat),
            }
        }
        true
    }
}

impl Drop for EngineCore {
    fn drop(&mut self) {
        // 兜底：正常路径 engine_thread_main 已显式 shutdown（此处二次调用无副作用）
        self.shutdown();
    }
}

// MARK: - 信号解码

/// evdev Key → (信号 id, 展示名)。
/// 调试名形如 "KEY_UP" / "BTN_LEFT" / "unknown key: 342"：
/// 去 KEY_ 前缀小写得 `linux:key:up`；BTN 保留整名得 `linux:key:btn_left`；
/// 无法命名时退回数字码 `linux:key:342`。
fn signal_of_key(key: Key, code: u16) -> (String, String) {
    let dbg = format!("{key:?}");
    let name: Option<String> = if let Some(rest) = dbg.strip_prefix("KEY_") {
        Some(rest.to_string())
    } else if dbg.starts_with("BTN_") {
        Some(dbg.clone())
    } else {
        None
    };
    let name =
        name.filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    match name {
        Some(n) => {
            let low = n.to_lowercase();
            (format!("linux:key:{low}"), format!("{n} (linux:key:{low})"))
        }
        None => (
            format!("linux:key:{code}"),
            format!("{dbg} (linux:key:{code})"),
        ),
    }
}

// MARK: - 前台 App

/// 前台 App 识别：v1 恒返回 (None, None)，即总是命中默认方案。
/// Wayland 协议层面没有稳定的「当前焦点应用」查询；
/// TODO: X11 会话可走 _NET_ACTIVE_WINDOW + _NET_WM_PID（需引入 x11rb 依赖）。
fn frontmost_app() -> (Option<String>, Option<String>) {
    (None, None)
}

// MARK: - 电源键

/// 配置里是否把 power 键绑了真实动作（绑了才值得警告；与 macOS 口径一致）
fn config_binds_power_key(cfg: &Config) -> bool {
    cfg.profiles.iter().any(|p| {
        p.bindings.get("power").is_some_and(|b| {
            [&b.tap, &b.long, &b.double].into_iter().flatten().any(|a| {
                let act = Act::normalize(a);
                !act.is_passthrough() && !act.is_none()
            })
        })
    })
}
