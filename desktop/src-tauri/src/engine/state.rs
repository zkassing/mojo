//! 按键状态机：单击 / 长按 / 双击 / 按住连发 / 去抖。
//!
//! 纯逻辑、跨平台、不碰定时器 —— 它只根据事件产出 `Eff` 效果列表，
//! 由平台驱动层（macOS 引擎线程）把 Schedule*/StartRepeat 落成真实定时器。
//! 语义与 Swift `RemapEngine.process(...)` 一一对应。

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 绑定的形状（状态机只关心有没有，不关心具体动作）
#[derive(Debug, Clone, Copy, Default)]
pub struct BindingShape {
    pub has_tap: bool,
    pub has_long: bool,
    pub has_double: bool,
    pub repeat: bool,
}

/// 各档时长，来自 `config.options`
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub long_press: Duration,
    pub double_press: Duration,
    pub debounce: Duration,
}

/// 状态机产出的效果，驱动层负责执行
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Eff {
    /// 执行 tap 动作
    PerformTap,
    /// 执行 long 动作
    PerformLong,
    /// 执行 double 动作
    PerformDouble,
    /// 安排长按定时器（到点未取消则回调 long_timeout）
    ScheduleLong(Duration),
    /// 安排双击兜底定时器（到点未取消则回调 tap_timeout）
    ScheduleTapFallback(Duration),
    /// 取消长按定时器
    CancelLong,
    /// 取消双击兜底定时器
    CancelTapFallback,
    /// 开始连发（initial 后每 interval 触发一次 PerformTap）
    StartRepeat { initial: Duration, interval: Duration },
    StopRepeat,
}

#[derive(Debug, Default)]
struct KeyState {
    is_down: bool,
    /// 上次松开时间，用于去抖（遥控器硬件抖动会一次按压发两次 down）
    last_up_at: Option<Instant>,
    long_fired: bool,
    awaiting_second_tap: bool,
}

#[derive(Debug, Default)]
pub struct Machine {
    states: HashMap<String, KeyState>,
}

impl Machine {
    pub fn new() -> Self {
        Self::default()
    }

    /// 按下。返回要执行的效果（可能为空：系统自动重复 / 去抖丢弃）。
    pub fn press(&mut self, button: &str, b: BindingShape, t: Timing, now: Instant) -> Vec<Eff> {
        let st = self.states.entry(button.into()).or_default();
        if st.is_down {
            return vec![]; // 系统自动重复
        }
        // 去抖：距上次松开太近的按下直接丢弃。
        // 只对没配双击的键生效 —— 双击本身就靠两次快速按压识别。
        if !b.has_double && t.debounce > Duration::ZERO {
            if let Some(up) = st.last_up_at {
                if now.duration_since(up) < t.debounce {
                    return vec![];
                }
            }
        }
        st.is_down = true;
        st.long_fired = false;

        let mut eff = vec![];
        if b.has_long {
            eff.push(Eff::ScheduleLong(t.long_press));
        }
        // 按住连发：仅当无长按/双击时，立即触发一次 tap 并启动连发
        if b.repeat && !b.has_long && !b.has_double && b.has_tap {
            eff.push(Eff::PerformTap);
            eff.push(Eff::StartRepeat {
                initial: Duration::from_millis(400),
                interval: Duration::from_millis(90),
            });
        }
        eff
    }

    /// 松开。
    pub fn release(&mut self, button: &str, b: BindingShape, _t: Timing, now: Instant) -> Vec<Eff> {
        let st = self.states.entry(button.into()).or_default();
        st.is_down = false;
        st.last_up_at = Some(now);

        let mut eff = vec![Eff::CancelLong, Eff::StopRepeat];

        if st.long_fired {
            st.long_fired = false;
            return eff;
        }
        if b.repeat && !b.has_long && !b.has_double {
            return eff; // 连发模式：松开不再补 tap
        }

        if b.has_double {
            if st.awaiting_second_tap {
                // 第二下到了：取消第一次留下的 tap 兜底，触发双击
                st.awaiting_second_tap = false;
                eff.push(Eff::CancelTapFallback);
                eff.push(Eff::PerformDouble);
            } else {
                // 第一下：先等 double_press 窗口，超时再按单击处理
                st.awaiting_second_tap = true;
                eff.push(Eff::ScheduleTapFallback(_t.double_press));
            }
            return eff;
        }

        if b.has_tap {
            eff.push(Eff::PerformTap);
        }
        eff
    }

    /// 长按定时器到点（驱动层回调）
    pub fn long_timeout(&mut self, button: &str) -> Vec<Eff> {
        let st = self.states.entry(button.into()).or_default();
        if !st.is_down {
            return vec![];
        }
        st.long_fired = true;
        st.awaiting_second_tap = false;
        // 长按成立：作废可能挂着的 tap 兜底
        vec![Eff::CancelTapFallback, Eff::PerformLong]
    }

    /// 双击窗口超时（驱动层回调）：按单击处理
    pub fn tap_timeout(&mut self, button: &str, has_tap: bool) -> Vec<Eff> {
        let st = self.states.entry(button.into()).or_default();
        st.awaiting_second_tap = false;
        if has_tap {
            vec![Eff::PerformTap]
        } else {
            vec![]
        }
    }

    /// 配置重载时清空所有按键状态
    pub fn reset(&mut self) {
        self.states.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Timing = Timing {
        long_press: Duration::from_millis(450),
        double_press: Duration::from_millis(280),
        debounce: Duration::from_millis(150),
    };

    fn tap_only() -> BindingShape {
        BindingShape {
            has_tap: true,
            ..Default::default()
        }
    }

    fn tap_long() -> BindingShape {
        BindingShape {
            has_tap: true,
            has_long: true,
            ..Default::default()
        }
    }

    fn tap_double() -> BindingShape {
        BindingShape {
            has_tap: true,
            has_double: true,
            ..Default::default()
        }
    }

    fn repeat_key() -> BindingShape {
        BindingShape {
            has_tap: true,
            repeat: true,
            ..Default::default()
        }
    }

    fn at(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    #[test]
    fn simple_tap() {
        let mut m = Machine::new();
        assert!(m.press("ok", tap_only(), T, at(0)).is_empty());
        let eff = m.release("ok", tap_only(), T, at(200));
        assert!(eff.contains(&Eff::PerformTap));
    }

    #[test]
    fn long_press_fires_long_not_tap() {
        let mut m = Machine::new();
        let eff = m.press("ok", tap_long(), T, at(0));
        assert_eq!(eff, vec![Eff::ScheduleLong(T.long_press)]);
        let eff = m.long_timeout("ok");
        assert_eq!(eff, vec![Eff::CancelTapFallback, Eff::PerformLong]);
        // 长按后松开：不再触发 tap
        let eff = m.release("ok", tap_long(), T, at(600));
        assert!(!eff.contains(&Eff::PerformTap));
    }

    #[test]
    fn short_press_with_long_binding_fires_tap() {
        let mut m = Machine::new();
        m.press("ok", tap_long(), T, at(0));
        let eff = m.release("ok", tap_long(), T, at(200));
        assert!(eff.contains(&Eff::CancelLong));
        assert!(eff.contains(&Eff::PerformTap));
    }

    #[test]
    fn double_tap_fires_double() {
        let mut m = Machine::new();
        // 第一下：挂起等待
        m.press("ok", tap_double(), T, at(0));
        let eff = m.release("ok", tap_double(), T, at(100));
        assert_eq!(
            eff,
            vec![
                Eff::CancelLong,
                Eff::StopRepeat,
                Eff::ScheduleTapFallback(T.double_press)
            ]
        );
        // 第二下在窗口内
        m.press("ok", tap_double(), T, at(200));
        let eff = m.release("ok", tap_double(), T, at(300));
        assert!(eff.contains(&Eff::CancelTapFallback));
        assert!(eff.contains(&Eff::PerformDouble));
        assert!(!eff.contains(&Eff::PerformTap));
    }

    #[test]
    fn single_tap_with_double_binding_falls_back() {
        let mut m = Machine::new();
        m.press("ok", tap_double(), T, at(0));
        m.release("ok", tap_double(), T, at(100));
        // 窗口超时 → 单击
        let eff = m.tap_timeout("ok", true);
        assert_eq!(eff, vec![Eff::PerformTap]);
    }

    #[test]
    fn repeat_fires_immediately_and_starts_timer() {
        let mut m = Machine::new();
        let eff = m.press("up", repeat_key(), T, at(0));
        assert!(eff.contains(&Eff::PerformTap));
        assert!(eff.contains(&Eff::StartRepeat {
            initial: Duration::from_millis(400),
            interval: Duration::from_millis(90)
        }));
        let eff = m.release("up", repeat_key(), T, at(1000));
        assert!(eff.contains(&Eff::StopRepeat));
        assert!(!eff.contains(&Eff::PerformTap)); // 松开不补发
    }

    #[test]
    fn debounce_drops_fast_repress_without_double() {
        let mut m = Machine::new();
        m.press("ok", tap_only(), T, at(0));
        m.release("ok", tap_only(), T, at(100));
        // 距上次松开仅 50ms < 150ms 去抖窗口：按下被丢弃
        assert!(m.press("ok", tap_only(), T, at(150)).is_empty());
        // 窗口外正常
        let eff = m.press("ok", tap_only(), T, at(400));
        assert!(eff.is_empty()); // tap_only 按下本来就没效果，但不能是「被去抖」
        let eff = m.release("ok", tap_only(), T, at(500));
        assert!(eff.contains(&Eff::PerformTap));
    }

    #[test]
    fn double_binding_key_is_not_debounced() {
        let mut m = Machine::new();
        m.press("ok", tap_double(), T, at(0));
        m.release("ok", tap_double(), T, at(100));
        // 50ms 内再按：配了双击的键不去抖（否则第二下被吃掉）
        let eff = m.press("ok", tap_double(), T, at(150));
        // 状态机接受了这次按下（返回空效果但 is_down 已置位）
        assert!(eff.is_empty());
        let eff = m.release("ok", tap_double(), T, at(250));
        assert!(eff.contains(&Eff::PerformDouble));
    }

    #[test]
    fn auto_repeat_down_ignored() {
        let mut m = Machine::new();
        m.press("ok", tap_only(), T, at(0));
        // 按住时系统补发的 down
        assert!(m.press("ok", tap_only(), T, at(300)).is_empty());
        let eff = m.release("ok", tap_only(), T, at(400));
        assert!(eff.contains(&Eff::PerformTap));
    }

    #[test]
    fn long_timeout_after_release_does_nothing() {
        let mut m = Machine::new();
        m.press("ok", tap_long(), T, at(0));
        m.release("ok", tap_long(), T, at(200)); // 短按，定时器应已取消
        // 即使定时器误触发也不应出动作
        assert!(m.long_timeout("ok").is_empty());
    }
}
