//! Windows 引擎驱动（对照 macos/mod.rs 移植）。
//!
//! 结构：
//! - `WindowsEngine`：对外句柄（PlatformEngine impl），管理引擎线程生命周期
//! - 引擎线程：Win32 消息泵线程，安装 WH_KEYBOARD_LL 低级键盘钩子，
//!   钩子回调里跑全部按键逻辑（信号路由 / 方案解析 / 状态机 / 动作执行）
//! - `EngineCore`：引擎线程上的全部可变状态（单线程，经全局原子指针共享给钩子回调）
//!
//! 与 macOS 的差异：
//! - 事件源：CGEventTap/IOHID → WH_KEYBOARD_LL 钩子。钩子只能拿到 vkCode，
//!   无法区分遥控器与物理键盘，同名 vk 的映射对所有键盘生效
//! - 按键定时器：CFRunLoopTimer → 「代次计数 + 睡线程 + mpsc 回执」（见下）
//! - 遥控器在线检测：IOHID 匹配回调 → 3 秒轮询 SetupAPI（devwatch.rs）
//! - 电源键守卫：Windows 无可否决系统睡眠的等效 API，仅打警告
//! - HID 直读通道：未移植（macOS 配置里的 hid: 信号在 Windows 不生效，
//!   按键改用 win:vk:0xXX 信号）

mod devwatch;
pub mod emit;
mod frontmost;

use crate::config::Config;
use crate::engine::action::{self, Act};
use crate::engine::log::{Log, VERBOSE};
use crate::engine::state::{BindingShape, Eff, Machine, Timing};
use crate::engine::{uses_dictate, EngineStatus, PlatformEngine};

use std::collections::HashMap;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HMODULE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{MsgWaitForMultipleObjects, QS_ALLINPUT};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, PeekMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLKHF_LOWER_IL_INJECTED, MSG, PM_REMOVE,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

/// 遥控器在线检测的轮询间隔
const DEV_POLL_INTERVAL: Duration = Duration::from_secs(3);

// MARK: - 对外句柄

/// 引擎线程消息：外部命令（Reload/Stop）与定时器线程回执走同一条 mpsc
enum EngineMsg {
    Reload(Config),
    Stop,
    /// 定时器线程到点回执（gen 用于代次校验，过期即丢）
    TimerFired {
        button: String,
        kind: KeyTimerKind,
        gen: u64,
    },
}

struct Shared {
    cmd_tx: Mutex<Option<mpsc::Sender<EngineMsg>>>,
    running: Mutex<bool>,
    remote_connected: Mutex<bool>,
}

pub struct WindowsEngine {
    shared: Arc<Shared>,
}

impl WindowsEngine {
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

impl PlatformEngine for WindowsEngine {
    fn supported(&self) -> bool {
        true
    }

    fn start(&self, config: &Config) -> anyhow::Result<()> {
        // 幂等：已在运行则发 Reload（引擎线程重建 machine/定时器/按键集）
        let running = *self.shared.running.lock().unwrap();
        {
            let tx_guard = self.shared.cmd_tx.lock().unwrap();
            if running {
                if let Some(tx) = tx_guard.as_ref() {
                    let _ = tx.send(EngineMsg::Reload(config.clone()));
                    return Ok(());
                }
            }
        }
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

// MARK: - 引擎线程

/// 钩子回调与引擎线程的桥梁：LL 钩子回调运行在装钩子的线程（即引擎线程）上，
/// 消息泵抽消息时被系统同步调用，因此对 EngineCore 的访问天然串行
static HOOK_CORE: AtomicPtr<EngineCore> = AtomicPtr::new(std::ptr::null_mut());

fn engine_thread_main(
    config: Config,
    timer_tx: mpsc::Sender<EngineMsg>,
    rx: mpsc::Receiver<EngineMsg>,
    shared: Arc<Shared>,
) {
    Log::info(&format!(
        "Rust 引擎启动（Windows，设备 0x{:04x}/0x{:04x}，{} 个按键，{} 个方案）",
        config.device.vendor_id,
        config.device.product_id,
        config.buttons.len(),
        config.profiles.len()
    ));
    VERBOSE.store(config.options.verbose, Ordering::Relaxed);

    let mut core = Box::new(EngineCore::new(config, timer_tx, shared.clone()));
    core.post_init();
    let core_ptr = Box::into_raw(core);
    HOOK_CORE.store(core_ptr, Ordering::SeqCst);

    // 安装低级键盘钩子（LL 钩子回调在本线程消息泵里被调用）
    // SAFETY: 本线程随后就跑消息泵，满足 LL 钩子的线程模型要求；
    // 指针参数用 .into() 传递，兼容 windows crate 各版本签名差异。
    let hmod = match unsafe { GetModuleHandleW(PCWSTR::null()) } {
        Ok(h) => h,
        Err(e) => {
            Log::warn(&format!("GetModuleHandleW 失败（{e}），仍尝试装钩子"));
            HMODULE::default()
        }
    };
    let hook = match unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook_proc),
            Some(windows::Win32::Foundation::HINSTANCE(hmod.0)),
            0,
        )
    } {
        Ok(h) => h,
        Err(e) => {
            Log::error(&format!(
                "无法安装低级键盘钩子（SetWindowsHookExW: {e}）。按键映射不可用。"
            ));
            HOOK_CORE.store(std::ptr::null_mut(), Ordering::SeqCst);
            // SAFETY: core_ptr 是上方 Box::into_raw 的产物，此刻接管析构
            unsafe { drop(Box::from_raw(core_ptr)) };
            *shared.running.lock().unwrap() = false;
            return;
        }
    };
    Log::info("键盘钩子已安装（WH_KEYBOARD_LL）");
    *shared.running.lock().unwrap() = true;

    // 消息泵：等消息（100ms 超时）→ 抽干消息（钩子回调在其中被调用）
    // → 处理命令/定时器回执 → 周期任务
    'pump: loop {
        // SAFETY: 空句柄数组仅等队列输入；返回值无需区分（超时与来消息走同一轮）
        unsafe { MsgWaitForMultipleObjects(None, false, 100, QS_ALLINPUT) };

        let mut msg = MSG::default();
        // SAFETY: msg 是栈上有效缓冲；hwnd=None 收本线程全部消息
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                break 'pump;
            }
            // SAFETY: 消息来自本线程队列
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // 命令与定时器回执（都在引擎线程串行执行）
        while let Ok(m) = rx.try_recv() {
            match m {
                EngineMsg::Stop => break 'pump,
                // SAFETY: 引擎线程独占 core_ptr，且此时没有存活的 &mut 引用
                EngineMsg::Reload(cfg) => unsafe { (*core_ptr).reload(cfg) },
                EngineMsg::TimerFired { button, kind, gen } => unsafe {
                    (*core_ptr).on_timer_fired(&button, kind, gen)
                },
            }
        }

        // 周期任务：遥控器在线检测（内部 3s 节流）
        // SAFETY: 同上，引擎线程独占
        unsafe { (*core_ptr).tick() };
    }

    // 清理
    // SAFETY: hook 由本线程安装且仍有效
    unsafe {
        let _ = UnhookWindowsHookEx(hook);
    }
    HOOK_CORE.store(std::ptr::null_mut(), Ordering::SeqCst);
    // SAFETY: core_ptr 是 Box::into_raw 的产物，钩子已卸、不会再有回调访问
    unsafe { drop(Box::from_raw(core_ptr)) };
    *shared.running.lock().unwrap() = false;
    *shared.remote_connected.lock().unwrap() = false;
    Log::info("Rust 引擎已停止");
}

// MARK: - 键盘钩子回调

/// WH_KEYBOARD_LL 回调（运行在引擎线程上下文，由消息泵驱动）
///
/// SAFETY: 由系统按钩子协议调用；lparam 在调用期间指向有效的 KBDLLHOOKSTRUCT。
unsafe extern "system" fn keyboard_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // code < 0：按文档必须原样下传，不得处理
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let core = HOOK_CORE.load(Ordering::SeqCst);
    if core.is_null() {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let is_down = match wparam.0 as u32 {
        WM_KEYDOWN | WM_SYSKEYDOWN => true,
        WM_KEYUP | WM_SYSKEYUP => false,
        _ => return CallNextHookEx(None, code, wparam, lparam),
    };
    let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    // 跳过自己 SendInput 注入的按键（带 INJECTED 标志），否则映射动作会再触发钩子形成死循环
    if (kb.flags.0 & LLKHF_INJECTED.0) != 0 || (kb.flags.0 & LLKHF_LOWER_IL_INJECTED.0) != 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let vk = kb.vkCode;
    let signal = format!("win:vk:0x{vk:02X}");
    // verbose 模式全量打日志，方便用户在日志页发现信号 id
    Log::debug(&format!(
        "{signal} {}",
        if is_down { "按下" } else { "松开" }
    ));

    let consumed = (*core).route_signal(&signal, is_down);
    // 已消费且配置吞掉原键 → 返回 1 吞掉；否则放行给后续钩子/系统
    if consumed && (*core).config.options.swallow_original {
        LRESULT(1)
    } else {
        CallNextHookEx(None, code, wparam, lparam)
    }
}

// MARK: - 按键定时器（代次计数 + 睡线程 + mpsc 回执）

#[derive(Clone, Copy, PartialEq)]
enum KeyTimerKind {
    Long,
    TapFallback,
    Repeat,
}

struct KeyTimerSlot {
    /// 本槽位当前接受的代次（引擎级全局分配，跨槽位唯一，见 next_timer_gen）
    gen: u64,
    /// 与定时器线程共享的存活信号：值 == 本线程携带的 gen 才允许发回执；
    /// 取消/重排时改掉它，睡着的线程醒来发现不符就静默退出
    live: Arc<AtomicU64>,
    /// 触发时要执行的动作（Long→long / TapFallback→tap / Repeat→tap）
    action: Option<Act>,
}

#[derive(Default)]
struct KeyTimers {
    long: Option<KeyTimerSlot>,
    tap_fallback: Option<KeyTimerSlot>,
    repeat: Option<KeyTimerSlot>,
}

impl KeyTimers {
    fn slot(&self, kind: KeyTimerKind) -> &Option<KeyTimerSlot> {
        match kind {
            KeyTimerKind::Long => &self.long,
            KeyTimerKind::TapFallback => &self.tap_fallback,
            KeyTimerKind::Repeat => &self.repeat,
        }
    }

    fn slot_mut(&mut self, kind: KeyTimerKind) -> &mut Option<KeyTimerSlot> {
        match kind {
            KeyTimerKind::Long => &mut self.long,
            KeyTimerKind::TapFallback => &mut self.tap_fallback,
            KeyTimerKind::Repeat => &mut self.repeat,
        }
    }
}

/// 定时器线程：睡 delay 后发回执；interval > 0 则按周期连发，
/// 直到代次被作废（取消/重排/引擎停止）或引擎队列断开
fn timer_thread(
    button: String,
    kind: KeyTimerKind,
    delay: Duration,
    interval: Duration,
    gen: u64,
    live: Arc<AtomicU64>,
    tx: mpsc::Sender<EngineMsg>,
) {
    std::thread::sleep(delay);
    loop {
        if live.load(Ordering::SeqCst) != gen {
            return; // 已被取消或重排
        }
        if tx
            .send(EngineMsg::TimerFired {
                button: button.clone(),
                kind,
                gen,
            })
            .is_err()
        {
            return; // 引擎已停
        }
        if interval.is_zero() {
            return; // 一次性定时器
        }
        std::thread::sleep(interval);
    }
}

// MARK: - EngineCore

struct EngineCore {
    config: Config,
    raw_to_button: HashMap<String, String>,
    timing: Timing,
    machine: Machine,
    emitter: emit::Emitter,
    /// 定时器线程的回执通道（与命令通道同一条）
    timer_tx: mpsc::Sender<EngineMsg>,
    /// 定时器代次分配器（全局单调递增，保证跨槽位唯一：
    /// 取消/重排后，旧线程已发出但未处理的回执绝不会撞上新槽位的代次）
    next_timer_gen: u64,
    key_timers: HashMap<String, KeyTimers>,
    shared: Arc<Shared>,
    /// 语音链路（配置含 dictate 绑定时才初始化）
    voice: Option<crate::engine::voice::VoiceChain>,
    /// 当前按住 dictate 的键位
    dictating_button: Option<String>,
    /// 上次遥控器在线检测时间（None = 尚未检测，下个节拍立即检测）
    last_dev_poll: Option<Instant>,
    remote_online: bool,
}

impl EngineCore {
    fn new(config: Config, timer_tx: mpsc::Sender<EngineMsg>, shared: Arc<Shared>) -> Self {
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
            timer_tx,
            next_timer_gen: 1,
            key_timers: HashMap::new(),
            shared,
            voice: None,
            dictating_button: None,
            last_dev_poll: None,
            remote_online: false,
        }
    }

    /// 引擎线程装钩子前调用：电源键警告、语音链路
    fn post_init(&mut self) {
        // 电源键守卫：Windows 没有可否决系统睡眠的等效 API
        // （macOS 用 IORegisterForSystemPower + IOCancelPowerChange），仅提示
        if config_binds_power_key(&self.config) {
            Log::warn(
                "配置给 power 键绑定了动作：Windows 无法拦截电源键的系统默认行为（无等效 API），仅按键映射部分生效",
            );
        }

        // 语音链路：仅当配置了 dictate 绑定时初始化（同 macOS setupVoice）
        if uses_dictate(&self.config) {
            self.init_voice();
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
            Log::warn("power 键绑定已生效，但 Windows 无法拦截电源键的系统默认行为（无等效 API）");
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
        // 全局代次：唯一性保证过期回执永远撞不上新槽位
        let gen = self.next_timer_gen;
        self.next_timer_gen = self.next_timer_gen.wrapping_add(1);

        let timers = self.key_timers.entry(button.to_string()).or_default();
        let slot = timers.slot_mut(kind);
        if slot.is_none() {
            *slot = Some(KeyTimerSlot {
                gen: 0,
                live: Arc::new(AtomicU64::new(0)),
                action: None,
            });
        }
        // 重排 = 改写共享代次：旧线程（如果还睡着）醒来发现代次不符自行退出
        let s = slot.as_mut().expect("刚补过槽位");
        s.live.store(gen, Ordering::SeqCst);
        s.gen = gen;
        s.action = action;
        let live = s.live.clone();
        let tx = self.timer_tx.clone();
        let btn = button.to_string();
        if let Err(e) = std::thread::Builder::new()
            .name("mojo-key-timer".into())
            .spawn(move || timer_thread(btn, kind, delay, interval, gen, live, tx))
        {
            Log::warn(&format!("按键定时器线程启动失败: {e}"));
        }
    }

    fn cancel_key_timer(&mut self, button: &str, kind: KeyTimerKind) {
        if let Some(timers) = self.key_timers.get_mut(button) {
            if let Some(slot) = timers.slot_mut(kind).take() {
                // 作废共享代次：睡着的线程醒来后自行退出，
                // 已发出的回执也会被引擎侧的代次校验挡掉
                slot.live.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    /// 摘出槽位（定时器已触发，等价 macOS take_timer_slot：动作随之取出）
    fn take_timer_slot(&mut self, button: &str, kind: KeyTimerKind) -> Option<KeyTimerSlot> {
        self.key_timers
            .get_mut(button)
            .and_then(|t| t.slot_mut(kind).take())
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

    /// 定时器回执（引擎线程串行执行，对照 macOS key_timer_cb）
    fn on_timer_fired(&mut self, button: &str, kind: KeyTimerKind, gen: u64) {
        // 代次校验：过期回执直接丢
        let valid = self
            .key_timers
            .get(button)
            .and_then(|t| t.slot(kind).as_ref())
            .is_some_and(|s| s.gen == gen);
        if !valid {
            return;
        }

        match kind {
            KeyTimerKind::Long => {
                // 一次性：先摘出槽位（定时器线程已退出）
                let Some(slot) = self.take_timer_slot(button, kind) else {
                    return;
                };
                let effs = self.machine.long_timeout(button);
                for eff in effs {
                    match eff {
                        Eff::CancelTapFallback => {
                            self.cancel_key_timer(button, KeyTimerKind::TapFallback)
                        }
                        Eff::PerformLong => {
                            if let Some(a) = &slot.action {
                                self.emitter.perform(a);
                            }
                        }
                        _ => {}
                    }
                }
            }
            KeyTimerKind::TapFallback => {
                let Some(slot) = self.take_timer_slot(button, kind) else {
                    return;
                };
                let effs = self.machine.tap_timeout(button, slot.action.is_some());
                for eff in effs {
                    if let (Eff::PerformTap, Some(a)) = (eff, &slot.action) {
                        self.emitter.perform(a);
                    }
                }
            }
            KeyTimerKind::Repeat => {
                // 连发：松开按键时 StopRepeat 会作废本定时器；槽位保留
                let action = self
                    .key_timers
                    .get(button)
                    .and_then(|t| t.slot(KeyTimerKind::Repeat).as_ref())
                    .and_then(|s| s.action.clone());
                if let Some(a) = &action {
                    self.emitter.perform(a);
                }
            }
        }
    }

    // MARK: 周期任务（消息泵每个节拍调用）

    fn tick(&mut self) {
        // 遥控器在线检测：3s 节流（首轮立即检测）
        let now = Instant::now();
        if self
            .last_dev_poll
            .is_some_and(|t| now.duration_since(t) < DEV_POLL_INTERVAL)
        {
            return;
        }
        self.last_dev_poll = Some(now);
        let online =
            devwatch::remote_online(self.config.device.vendor_id, self.config.device.product_id);
        if online != self.remote_online {
            self.remote_online = online;
            *self.shared.remote_connected.lock().unwrap() = online;
            if online {
                Log::info("遥控器已连接");
            } else {
                Log::info("遥控器已断开");
            }
        }
    }

    // MARK: 信号路由（钩子通道）

    /// 返回 true 表示事件已被映射消费（钩子据此决定是否吞掉原键）
    fn route_signal(&mut self, signal_id: &str, is_down: bool) -> bool {
        let Some(button) = self.raw_to_button.get(&signal_id.to_lowercase()).cloned() else {
            return false;
        };
        let (bid, bname) = frontmost::frontmost_app();
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
                Eff::PerformLong => { /* 由长按定时器回执执行（动作在槽位里） */ }
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

/// 配置里是否把 power 键绑了真实动作（与 macOS 同口径；
/// Windows 仅用于打「不支持拦截」的警告）
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

impl Drop for EngineCore {
    fn drop(&mut self) {
        self.shutdown();
    }
}
