import Foundation
import IOKit
import IOKit.pwr_mgt

/// 拦住电源键触发的系统睡眠，把电源键还给应用层。
///
/// macOS 睡眠前会向注册者广播 `kIOMessageCanSystemSleep`，这个阶段
/// 允许否决 —— 调 `IOCancelPowerChange` 就把睡眠拦下来了。
/// （后一个阶段 `kIOMessageSystemWillSleep` 只能延迟，不能拒绝。）
///
/// 为什么不用 `pmset -a powerbutton 0`：macOS 26+ 已经不接受这个参数。
///
/// 注意这会拦住**所有**来源的空闲/按键睡眠请求，包括 Mac 自带电源键。
/// 合盖睡眠、菜单栏「睡眠」、`pmset sleepnow` 走的是别的路径，不受影响。
final class PowerButtonGuard {

    private var rootPort: io_connect_t = 0
    private var notifyPort: IONotificationPortRef?
    private var notifier: io_object_t = 0
    private var enabled = false

    /// IOMessage.h 里的常量，Swift 没导出，用字面值
    private static let canSystemSleep: UInt32 = 0xE000_0270
    private static let systemWillSleep: UInt32 = 0xE000_0280
    private static let systemHasPoweredOn: UInt32 = 0xE000_0300

    /// 开始拦截
    func start() {
        guard rootPort == 0 else { return }

        let ctx = Unmanaged.passUnretained(self).toOpaque()
        rootPort = IORegisterForSystemPower(ctx, &notifyPort, { refcon, _, msgType, msgArg in
            guard let refcon else { return }
            let me = Unmanaged<PowerButtonGuard>.fromOpaque(refcon).takeUnretainedValue()
            me.handle(messageType: msgType, argument: msgArg)
        }, &notifier)

        guard rootPort != 0, let notifyPort else {
            Log.warn("电源键守卫注册失败，电源键仍会触发系统睡眠")
            rootPort = 0
            return
        }

        CFRunLoopAddSource(
            CFRunLoopGetMain(),
            IONotificationPortGetRunLoopSource(notifyPort).takeUnretainedValue(),
            .defaultMode)

        enabled = true
        Log.info("电源键守卫已启动（拦截按键睡眠）")
    }

    /// 停止拦截，恢复系统默认行为
    func stop() {
        guard rootPort != 0 else { return }
        enabled = false

        if let notifyPort {
            CFRunLoopRemoveSource(
                CFRunLoopGetMain(),
                IONotificationPortGetRunLoopSource(notifyPort).takeUnretainedValue(),
                .defaultMode)
            IODeregisterForSystemPower(&notifier)
            IOServiceClose(rootPort)
            IONotificationPortDestroy(notifyPort)
        }
        rootPort = 0
        notifyPort = nil
        Log.info("电源键守卫已停止")
    }

    /// 临时放行一次睡眠（给「真的想睡」的动作用）
    func allowNextSleep() {
        enabled = false
        // 睡眠请求通常在 1s 内到达；之后自动恢复拦截
        DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
            self?.enabled = true
        }
    }

    private func handle(messageType: UInt32, argument: UnsafeMutableRawPointer?) {
        let arg = Int(bitPattern: argument)
        switch messageType {
        case Self.canSystemSleep:
            if enabled {
                // 否决掉，电源键的语义完全交给映射表
                IOCancelPowerChange(rootPort, arg)
                Log.debug("已拦截系统睡眠请求")
            } else {
                IOAllowPowerChange(rootPort, arg)
            }
        case Self.systemWillSleep:
            // 这阶段没法拒绝，放行即可
            IOAllowPowerChange(rootPort, arg)
        case Self.systemHasPoweredOn:
            break
        default:
            break
        }
    }

    deinit { stop() }
}
