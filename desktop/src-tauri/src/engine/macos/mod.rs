//! macOS 引擎驱动（对应 Swift `RemapEngine.swift` + `main.swift` 的 run 路径）。
//!
//! 结构：
//! - `MacEngine`：对外句柄（PlatformEngine impl），管理引擎线程生命周期
//! - 引擎线程：独立 CFRunLoop，跑事件 tap、IOHID 回调、按键定时器、命令轮询
//! - `EngineCore`：引擎线程上的全部可变状态（单线程，裸指针上下文共享）
//!
//! 语音链路（ATVV/ASR/LiveTyper）在 `engine/voice.rs`，按住 dictate 键接入。

pub mod emit;
mod ffi;
mod frontmost;
mod hid;
pub(crate) mod perms;
mod power;

use crate::config::Config;
use crate::engine::action::{self, Act};
use crate::engine::log::{Log, VERBOSE};
use crate::engine::state::{BindingShape, Eff, Machine, Timing};
use crate::engine::{uses_dictate, EngineStatus, PlatformEngine};

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use core_foundation::base::TCFType;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_foundation_sys::runloop as cf_rl_sys;

// MARK: - 定时器基建

/// 创建 CFRunLoopTimer 并加到当前线程 runloop（common modes）。
/// interval_secs == 0 表示一次性。返回的引用由 runloop 持有；
/// 提前取消用 ffi::CFRunLoopTimerInvalidate。
pub(super) fn make_timer(
    delay_secs: f64,
    interval_secs: f64,
    cb: cf_rl_sys::CFRunLoopTimerCallBack,
    ctx: *mut c_void,
) -> cf_rl_sys::CFRunLoopTimerRef {
    unsafe {
        let mut context: cf_rl_sys::CFRunLoopTimerContext = std::mem::zeroed();
        context.info = ctx;
        let timer = cf_rl_sys::CFRunLoopTimerCreate(
            std::ptr::null(),
            ffi::CFAbsoluteTimeGetCurrent() + delay_secs,
            interval_secs,
            0,
            0,
            cb,
            &mut context,
        );
        cf_rl_sys::CFRunLoopAddTimer(
            CFRunLoop::get_current().as_concrete_TypeRef(),
            timer,
            kCFRunLoopCommonModes,
        );
        // Create Rule：CFRunLoopTimerCreate 返回 +1，AddTimer 只是 runloop 再
        // retain 一份。这里立即平衡掉 Create 的引用，交给 runloop 持有；
        // 之后 invalidate 时 runloop 释放最后一份，对象才真正销毁。
        // 不这么做，每个长按/双击/连发定时器都会永久泄漏一个 timer 对象。
        ffi::CFRelease(timer as _);
        timer
    }
}

// MARK: - 对外句柄

pub enum EngineCmd {
    Reload(Config),
    Stop,
}

struct Shared {
    cmd_tx: Mutex<Option<mpsc::Sender<EngineCmd>>>,
    running: Mutex<bool>,
    remote_connected: Mutex<bool>,
}

pub struct MacEngine {
    shared: Arc<Shared>,
}

impl MacEngine {
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

impl PlatformEngine for MacEngine {
    fn supported(&self) -> bool {
        true
    }

    fn start(&self, config: &Config) -> anyhow::Result<()> {
        let running = *self.shared.running.lock().unwrap();
        {
            let tx_guard = self.shared.cmd_tx.lock().unwrap();
            if running {
                if let Some(tx) = tx_guard.as_ref() {
                    let _ = tx.send(EngineCmd::Reload(config.clone()));
                    return Ok(());
                }
            }
        }
        let (tx, rx) = mpsc::channel::<EngineCmd>();
        *self.shared.cmd_tx.lock().unwrap() = Some(tx);
        let shared = self.shared.clone();
        let config = config.clone();
        std::thread::Builder::new()
            .name("mojo-engine".into())
            .spawn(move || engine_thread_main(config, rx, shared))?;
        Ok(())
    }

    fn stop(&self) {
        if let Some(tx) = self.shared.cmd_tx.lock().unwrap().as_ref() {
            let _ = tx.send(EngineCmd::Stop);
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

fn engine_thread_main(config: Config, rx: mpsc::Receiver<EngineCmd>, shared: Arc<Shared>) {
    Log::info(&format!(
        "Rust 引擎启动（设备 0x{:04x}/0x{:04x}，{} 个按键，{} 个方案）",
        config.device.vendor_id,
        config.device.product_id,
        config.buttons.len(),
        config.profiles.len()
    ));
    VERBOSE.store(config.options.verbose, Ordering::Relaxed);

    let core = Box::new(EngineCore::new(config, shared.clone()));
    let core_ptr = Box::into_raw(core);
    unsafe { (*core_ptr).post_init(core_ptr) };

    if !unsafe { (*core_ptr).start_tap() } {
        Log::error(
            "无法创建事件拦截（CGEventTap）。\n请到 系统设置 › 隐私与安全性 › 辅助功能 中授权 Mojo，然后重试。\n若已勾选过旧版本，先移除再重新添加。",
        );
        unsafe { drop(Box::from_raw(core_ptr)) };
        *shared.running.lock().unwrap() = false;
        return;
    }

    // 命令轮询定时器（0.2s）：Reload / Stop 都从这条通道进引擎线程
    let drain = Box::new(DrainCtx { rx, core: core_ptr });
    let drain_ptr = Box::into_raw(drain);
    let drain_timer = make_timer(0.2, 0.2, drain_cb, drain_ptr as *mut c_void);

    *shared.running.lock().unwrap() = true;
    CFRunLoop::run_current();

    // 清理
    unsafe {
        ffi::CFRunLoopTimerInvalidate(drain_timer);
        drop(Box::from_raw(drain_ptr));
        (*core_ptr).shutdown();
        drop(Box::from_raw(core_ptr));
    }
    *shared.running.lock().unwrap() = false;
    *shared.remote_connected.lock().unwrap() = false;
    Log::info("Rust 引擎已停止");
}

struct DrainCtx {
    rx: mpsc::Receiver<EngineCmd>,
    core: *mut EngineCore,
}

extern "C" fn drain_cb(_timer: cf_rl_sys::CFRunLoopTimerRef, info: *mut c_void) {
    if info.is_null() {
        return;
    }
    let ctx = unsafe { &mut *(info as *mut DrainCtx) };
    while let Ok(cmd) = ctx.rx.try_recv() {
        match cmd {
            EngineCmd::Reload(cfg) => unsafe { (*ctx.core).reload(cfg) },
            EngineCmd::Stop => CFRunLoop::get_current().stop(),
        }
    }
}

// MARK: - 按键定时器

#[derive(Clone, Copy, PartialEq)]
enum KeyTimerKind {
    Long,
    TapFallback,
    Repeat,
}

struct KeyTimerCtx {
    core: *mut EngineCore,
    button: String,
    kind: KeyTimerKind,
    /// 触发时要执行的动作（Long→long / TapFallback→tap / Repeat→tap）
    action: Option<Act>,
}

struct KeyTimerSlot {
    timer: cf_rl_sys::CFRunLoopTimerRef,
    ctx: *mut KeyTimerCtx,
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
}

extern "C" fn key_timer_cb(_timer: cf_rl_sys::CFRunLoopTimerRef, info: *mut c_void) {
    if info.is_null() {
        return;
    }
    let ctx = unsafe { &mut *(info as *mut KeyTimerCtx) };
    let core = unsafe { &mut *ctx.core };
    let button = ctx.button.clone();

    match ctx.kind {
        KeyTimerKind::Long => {
            // 一次性：先从 map 摘出（定时器已自动失效），最后再释放 ctx 自身
            core.take_timer_slot(&button, KeyTimerKind::Long);
            let effs = core.machine.long_timeout(&button);
            for eff in effs {
                match eff {
                    Eff::CancelTapFallback => {
                        core.cancel_key_timer(&button, KeyTimerKind::TapFallback)
                    }
                    Eff::PerformLong => {
                        if let Some(a) = &ctx.action {
                            core.emitter.perform(a);
                        }
                    }
                    _ => {}
                }
            }
            unsafe { drop(Box::from_raw(info as *mut KeyTimerCtx)) };
        }
        KeyTimerKind::TapFallback => {
            core.take_timer_slot(&button, KeyTimerKind::TapFallback);
            let effs = core.machine.tap_timeout(&button, ctx.action.is_some());
            for eff in effs {
                if let (Eff::PerformTap, Some(a)) = (eff, &ctx.action) {
                    core.emitter.perform(a);
                }
            }
            unsafe { drop(Box::from_raw(info as *mut KeyTimerCtx)) };
        }
        KeyTimerKind::Repeat => {
            // 连发：松开按键时 StopRepeat 会取消本定时器
            if let Some(a) = &ctx.action {
                core.emitter.perform(a);
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
    watcher: Box<hid::DeviceWatcher>,
    hid: Option<Box<hid::HIDWatcher>>,
    power: Box<power::PowerButtonGuard>,
    tap: *mut c_void,
    tap_source: cf_rl_sys::CFRunLoopSourceRef,
    key_timers: HashMap<String, KeyTimers>,
    /// 语音链路（配置含 dictate 绑定时才初始化）
    voice: Option<crate::engine::voice::VoiceChain>,
    /// 当前按住 dictate 的键位
    dictating_button: Option<String>,
}

impl EngineCore {
    fn new(config: Config, shared: Arc<Shared>) -> Self {
        let timing = Timing {
            long_press: Duration::from_millis(config.options.long_press_ms as u64),
            double_press: Duration::from_millis(config.options.double_press_ms as u64),
            debounce: Duration::from_millis(config.options.debounce_ms as u64),
        };
        let raw_to_button = config.raw_to_button();

        let mut watcher = hid::DeviceWatcher::new(
            config.device.vendor_id as i32,
            config.device.product_id as i32,
        );
        watcher.on_change = Some(Box::new(move |connected| {
            *shared.remote_connected.lock().unwrap() = connected;
            if connected {
                Log::info("遥控器已连接");
            } else {
                Log::info("遥控器已断开");
            }
        }));

        Self {
            config,
            raw_to_button,
            timing,
            machine: Machine::new(),
            emitter: emit::Emitter::new(),
            watcher,
            hid: None,
            power: power::PowerButtonGuard::new(),
            tap: std::ptr::null_mut(),
            tap_source: std::ptr::null_mut(),
            key_timers: HashMap::new(),
            voice: None,
            dictating_button: None,
        }
    }

    /// 在 Box::into_raw 之后调用：启动需要回指 EngineCore 的组件
    fn post_init(&mut self, self_ptr: *mut EngineCore) {
        self.watcher.start();

        // 电源键守卫：配置里给 power 绑了真实动作才启用
        if config_binds_power_key(&self.config) {
            self.power.start();
        }

        // HID element 直读通道：配置里有 hid: 信号才启用
        if self.needs_hid_channel() {
            self.start_hid_channel(self_ptr);
        }

        // 语音链路：仅当配置了 dictate 绑定时初始化（同 Swift setupVoice）
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

    /// 按住开始录音，松开识别上屏（端口 Swift handleDictate）
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

    fn needs_hid_channel(&self) -> bool {
        self.raw_to_button.keys().any(|k| k.starts_with("hid:"))
    }

    fn start_hid_channel(&mut self, self_ptr: *mut EngineCore) {
        let mut hw = hid::HIDWatcher::new();
        let core_addr = self_ptr as usize;
        hw.on_value = Some(Box::new(move |page, usage, is_down| {
            let core = unsafe { &mut *(core_addr as *mut EngineCore) };
            core.handle_hid(page, usage, is_down);
        }));
        hw.start(
            self.config.device.vendor_id as i32,
            self.config.device.product_id as i32,
        );
        self.hid = Some(hw);
    }

    // MARK: 事件 tap

    fn start_tap(&mut self) -> bool {
        // keyDown(10) | keyUp(11) | systemDefined(14)
        let mask: u64 = (1 << 10) | (1 << 11) | (1 << 14);
        let ctx = self as *mut EngineCore as *mut c_void;
        unsafe {
            let tap = ffi::CGEventTapCreate(
                0, // kCGHIDEventTap
                0, // kCGHeadInsertEventTap
                0, // kCGEventTapOptionDefault
                mask,
                tap_cb,
                ctx,
            );
            if tap.is_null() {
                return false;
            }
            let src = ffi::CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
            cf_rl_sys::CFRunLoopAddSource(
                CFRunLoop::get_current().as_concrete_TypeRef(),
                src,
                kCFRunLoopCommonModes,
            );
            ffi::CGEventTapEnable(tap, true);
            self.tap = tap;
            self.tap_source = src;
        }
        Log::info("事件拦截已启动");
        true
    }

    fn shutdown(&mut self) {
        if let Some(v) = &self.voice {
            v.shutdown();
        }
        self.voice = None;
        self.dictating_button = None;
        self.cancel_all_key_timers();
        unsafe {
            if !self.tap.is_null() {
                ffi::CGEventTapEnable(self.tap, false);
                ffi::CFMachPortInvalidate(self.tap);
                ffi::CFRelease(self.tap as _);
                self.tap = std::ptr::null_mut();
            }
            if !self.tap_source.is_null() {
                ffi::CFRelease(self.tap_source as _);
                self.tap_source = std::ptr::null_mut();
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

        // 电源键守卫跟随绑定开/关
        let binds_power = config_binds_power_key(&cfg);
        if binds_power && !self.power.is_started() {
            self.power.start();
        } else if !binds_power && self.power.is_started() {
            self.power.stop();
        }

        // 新配置可能刚加入 hid: 信号，补开通道
        let needs_hid = cfg
            .raw_to_button()
            .keys()
            .any(|k| k.starts_with("hid:"));
        if self.hid.is_none() && needs_hid {
            let self_ptr = self as *mut EngineCore;
            self.start_hid_channel(self_ptr);
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
        let ctx = Box::new(KeyTimerCtx {
            core: self,
            button: button.to_string(),
            kind,
            action,
        });
        let ctx_ptr = Box::into_raw(ctx);
        let timer = make_timer(
            delay.as_secs_f64(),
            interval.as_secs_f64(),
            key_timer_cb,
            ctx_ptr as *mut c_void,
        );
        self.key_timers
            .entry(button.to_string())
            .or_default()
            .slot_mut(kind)
            .replace(KeyTimerSlot {
                timer,
                ctx: ctx_ptr,
            });
    }

    fn cancel_key_timer(&mut self, button: &str, kind: KeyTimerKind) {
        if let Some(timers) = self.key_timers.get_mut(button) {
            if let Some(slot) = timers.slot_mut(kind).take() {
                unsafe {
                    ffi::CFRunLoopTimerInvalidate(slot.timer);
                    drop(Box::from_raw(slot.ctx));
                }
            }
        }
    }

    /// 定时器触发时摘出 slot（不 invalidate、不释放 ctx —— 调用方正在用）
    fn take_timer_slot(&mut self, button: &str, kind: KeyTimerKind) {
        if let Some(timers) = self.key_timers.get_mut(button) {
            timers.slot_mut(kind).take();
        }
    }

    fn cancel_all_key_timers(&mut self) {
        let buttons: Vec<String> = self.key_timers.keys().cloned().collect();
        for b in buttons {
            for kind in [KeyTimerKind::Long, KeyTimerKind::TapFallback, KeyTimerKind::Repeat] {
                self.cancel_key_timer(&b, kind);
            }
        }
        self.key_timers.clear();
    }

    // MARK: HID 直读通道事件

    fn handle_hid(&mut self, page: u32, usage: u32, is_down: bool) {
        let signal = format!("hid:{page}:{usage}");
        if is_down {
            Log::debug(&format!("{} [HID 直读]", pretty_hid(page, usage)));
        }
        self.route_signal(&signal, is_down);
    }

    // MARK: 信号路由（tap / hid 两通道共用）

    /// 返回 true 表示事件已被映射消费（tap 通道据此决定是否吞掉原键）
    fn route_signal(&mut self, signal_id: &str, is_down: bool) -> bool {
        let Some(button) = self.raw_to_button.get(&signal_id.to_lowercase()).cloned() else {
            return false;
        };
        let (bid, bname) = frontmost::frontmost_app();
        let Some(profile) = self.config.resolve_profile(bid.as_deref(), bname.as_deref()) else {
            return false;
        };
        let Some(binding) = profile.bindings.get(&button) else {
            if is_down {
                Log::debug(&format!("[{}] {button} 无绑定，放行原键", profile.name));
            }
            return false;
        };
        let profile_name = profile.name.clone();
        let binding = binding.clone();

        // 显式 passthrough：不做任何映射，原样放行（不受 swallowOriginal 影响）
        if action::binding_is_passthrough(&binding) {
            if is_down {
                Log::info(&format!("[{profile_name}] {button} → 放行原键（passthrough）"));
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
                Eff::PerformLong => { /* 由长按定时器回调执行（动作在 ctx 里） */ }
                Eff::PerformDouble => {
                    if let Some(a) = &binding.double {
                        let act = Act::normalize(a);
                        Log::info(&format!("[{profile_name}] {button} 双击 → {}", act.describe()));
                        self.emitter.perform(&act);
                    }
                }
                Eff::ScheduleLong(d) => {
                    let act = binding.long.as_ref().map(Act::normalize);
                    if let Some(a) = &act {
                        Log::info(&format!("[{profile_name}] {button} 长按 → {}", a.describe()));
                    }
                    self.start_key_timer(&button, KeyTimerKind::Long, d, Duration::ZERO, act);
                }
                Eff::ScheduleTapFallback(d) => {
                    let act = binding.tap.as_ref().map(Act::normalize);
                    self.start_key_timer(&button, KeyTimerKind::TapFallback, d, Duration::ZERO, act);
                }
                Eff::CancelLong => self.cancel_key_timer(&button, KeyTimerKind::Long),
                Eff::CancelTapFallback => {
                    self.cancel_key_timer(&button, KeyTimerKind::TapFallback)
                }
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

    // MARK: tap 事件入口

    fn handle_tap_event(&mut self, type_: u32, event: *mut c_void) -> *mut c_void {
        unsafe {
            // senderID（field 87）= 设备 RegistryEntryID，只拦遥控器
            let sender = ffi::CGEventGetIntegerValueField(event, 87) as u64;
            if !self.watcher.contains(sender) {
                return event;
            }

            let Some((signal_id, pretty, is_down)) = decode_signal(type_, event) else {
                return event;
            };

            if !self.raw_to_button.contains_key(&signal_id) {
                Log::debug(&format!("未映射信号 {pretty}，放行"));
                return event;
            }

            let consumed = self.route_signal(&signal_id, is_down);
            if consumed && self.config.options.swallow_original {
                std::ptr::null_mut()
            } else {
                event
            }
        }
    }
}

extern "C" fn tap_cb(
    _proxy: *mut c_void,
    type_: u32,
    event: *mut c_void,
    user_info: *mut c_void,
) -> *mut c_void {
    if user_info.is_null() {
        return event;
    }
    // tap 被系统禁用（超时/用户输入过快）后自动重启
    if type_ == 0xFFFF_FFFE || type_ == 0xFFFF_FFFF {
        let core = unsafe { &mut *(user_info as *mut EngineCore) };
        if !core.tap.is_null() {
            unsafe { ffi::CGEventTapEnable(core.tap, true) };
        }
        Log::warn("事件拦截被系统暂停，已自动恢复");
        return event;
    }
    unsafe { (*(user_info as *mut EngineCore)).handle_tap_event(type_, event) }
}

// MARK: - 信号解码

/// 把 CGEvent 解码成 (信号 id, 展示名, isDown)
fn decode_signal(type_: u32, event: *mut c_void) -> Option<(String, String, bool)> {
    match type_ {
        // keyDown / keyUp
        10 | 11 => {
            let code = unsafe { ffi::CGEventGetIntegerValueField(event, 9) };
            let is_down = type_ == 10;
            let pretty = format!("{} (kc:{code})", emit::key_name(code));
            Some((format!("kc:{code}"), pretty, is_down))
        }
        // systemDefined（媒体键）
        14 => objc2::rc::autoreleasepool(|_| {
            let cg = unsafe { &*(event as *const objc2_core_graphics::CGEvent) };
            let ns = objc2_app_kit::NSEvent::eventWithCGEvent(cg)?;
            if ns.subtype().0 != 8 {
                return None;
            }
            let data1 = ns.data1();
            let key_code = (data1 & 0xFFFF_0000) >> 16;
            let key_state = (data1 & 0xFF00) >> 8;
            let is_down = key_state == 0xA;
            let pretty = format!("{} (aux:{key_code})", emit::media_name(key_code as i32));
            Some((format!("aux:{key_code}"), pretty, is_down))
        }),
        _ => None,
    }
}

fn pretty_hid(page: u32, usage: u32) -> String {
    let p = match page {
        0x07 => "Keyboard".to_string(),
        0x0C => "Consumer".to_string(),
        _ => format!("page 0x{page:x}"),
    };
    format!("{p} usage 0x{usage:x} (hid:{page}:{usage})")
}

/// 配置里是否把 power 键绑了真实动作（绑了才拦系统睡眠）
fn config_binds_power_key(cfg: &Config) -> bool {
    cfg.profiles.iter().any(|p| {
        p.bindings.get("power").is_some_and(|b| {
            [&b.tap, &b.long, &b.double]
                .into_iter()
                .flatten()
                .any(|a| {
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
