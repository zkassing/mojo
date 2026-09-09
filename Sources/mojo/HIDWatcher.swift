import Foundation
import IOKit
import IOKit.hid

/// 直接监听 HID element 值变化。
///
/// 用途：某些按键发出的 HID usage 不被 macOS 键盘驱动识别（例如小米遥控器的
/// 返回键发 Keyboard usage 0xF1），因此**永远不会**产生 CGEvent，
/// CGEventTap 拦不到。这条通道绕开事件系统直接读设备。
///
/// 只对配置里显式写成 `hid:<page>:<usage>` 的信号生效，
/// 避免和 CGEventTap 通道重复触发。
final class HIDWatcher {
    private let vendorId: Int
    private let productId: Int
    private var manager: IOHIDManager?
    /// 回调：(page, usage, isDown)
    private let onValue: (UInt32, UInt32, Bool) -> Void
    /// 已按下的 (page,usage)，用于去重和过滤自动重复
    private var down = Set<UInt64>()

    init(vendorId: Int, productId: Int, onValue: @escaping (UInt32, UInt32, Bool) -> Void) {
        self.vendorId = vendorId
        self.productId = productId
        self.onValue = onValue
    }

    func start() {
        let mgr = IOHIDManagerCreate(kCFAllocatorDefault, IOOptionBits(kIOHIDOptionsTypeNone))
        manager = mgr
        IOHIDManagerSetDeviceMatching(mgr, [
            kIOHIDVendorIDKey: vendorId,
            kIOHIDProductIDKey: productId
        ] as CFDictionary)

        let ctx = Unmanaged.passUnretained(self).toOpaque()
        IOHIDManagerRegisterInputValueCallback(mgr, { context, _, _, value in
            guard let context else { return }
            let me = Unmanaged<HIDWatcher>.fromOpaque(context).takeUnretainedValue()
            me.handle(value)
        }, ctx)

        IOHIDManagerScheduleWithRunLoop(mgr, CFRunLoopGetMain(), CFRunLoopMode.commonModes.rawValue)
        let r = IOHIDManagerOpen(mgr, IOOptionBits(kIOHIDOptionsTypeNone))
        if r != kIOReturnSuccess {
            Log.warn("HID element 通道打开失败 (0x\(String(format: "%08x", r)))，"
                   + "需要 系统设置 › 隐私与安全性 › 输入监控 授权")
        } else {
            Log.debug("HID element 通道已启动")
        }
    }

    private func handle(_ value: IOHIDValue) {
        let el = IOHIDValueGetElement(value)
        let page = IOHIDElementGetUsagePage(el)
        let usage = IOHIDElementGetUsage(el)
        let v = IOHIDValueGetIntegerValue(value)

        // 过滤噪声：
        //  - 0xffffffff 是 IOKit 内部的聚合/填充 element
        //  - Keyboard usage 0x00 (Reserved) / 0x01 (ErrorRollOver) 是状态位，不是真按键
        guard usage != 0xFFFFFFFF else { return }
        if page == 0x07 && (usage == 0x00 || usage == 0x01) { return }

        let key = (UInt64(page) << 32) | UInt64(usage)
        let isDown = v != 0

        if isDown {
            // 去掉系统自动重复
            if down.contains(key) { return }
            down.insert(key)
        } else {
            guard down.contains(key) else { return }
            down.remove(key)
        }
        onValue(page, usage, isDown)
    }
}
