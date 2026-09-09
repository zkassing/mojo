import Foundation
import IOKit

// hidsystem 头不在 Swift 模块里，手动声明（符号在 IOKit.framework，已实测可用）
@_silgen_name("NXOpenEventStatus")
private func NXOpenEventStatusRaw() -> io_connect_t
@_silgen_name("NXCloseEventStatus")
private func NXCloseEventStatusRaw(_ handle: io_connect_t)
@_silgen_name("IOHIDPostEvent")
private func IOHIDPostEventRaw(
    _ connect: io_connect_t, _ type: UInt32, _ location: NXIOGPoint,
    _ data: UnsafePointer<NXKeyEventData>, _ version: UInt32,
    _ flags: IOOptionBits, _ options: IOOptionBits
) -> kern_return_t
@_silgen_name("IOHIDRequestAccess")
private func IOHIDRequestAccessRaw(_ requestType: Int32) -> Bool

/// 与 C 的 IOGPoint 同布局（SInt16 x, y）
private struct NXIOGPoint { var x: Int16 = 0, y: Int16 = 0 }

/// NXEventData 中 key 成员的布局（union 全长 48 字节，见 IOLLEvent.h）
private struct NXKeyEventData {
    var origCharSet: UInt16 = 0
    var isRepeat: Int16 = 0
    var charSet: UInt16 = 0
    var charCode: UInt16 = 0
    var keyCode: UInt16 = 0
    var origCharCode: UInt16 = 0
    var reserved1: Int32 = 0
    var keyboardType: UInt32 = 0
    var reserved2: Int32 = 0
    var reserved3: Int32 = 0
    var reserved4: Int32 = 0
    var reserved5: (Int32, Int32, Int32, Int32) = (0, 0, 0, 0)
}

/// 驱动层事件注入：经 IOHIDPostEvent 把按键送进 HID 事件系统，
/// 事件不带发送进程 PID，与物理键盘不可区分 —— 能触发过滤
/// CGEvent 合成事件的全局热键（如微信语音快捷键）。
final class NXPoster {
    private var handle: io_connect_t = 0
    private(set) var available = false

    private static let kIOHIDRequestTypePostEvent: Int32 = 0
    private static let kNXEventDataVersion: UInt32 = 2
    private static let kIOHIDSetGlobalEventFlags: IOOptionBits = 0x1

    private static let NX_KEYDOWN: UInt32 = 10
    private static let NX_KEYUP: UInt32 = 11
    private static let NX_FLAGSCHANGED: UInt32 = 12

    // IOLLEvent.h 的修饰键 mask
    private static let NX_SHIFT: IOOptionBits = 0x0002_0000
    private static let NX_CONTROL: IOOptionBits = 0x0004_0000
    private static let NX_ALT: IOOptionBits = 0x0008_0000
    private static let NX_CMD: IOOptionBits = 0x0010_0000
    private static let NX_FN: IOOptionBits = 0x0080_0000

    init() {
        handle = NXOpenEventStatusRaw()
        guard handle != 0 else {
            Log.warn("NXOpenEventStatus 失败，按键注入走 CGEvent")
            return
        }
        if !IOHIDRequestAccessRaw(Self.kIOHIDRequestTypePostEvent) {
            Log.warn("IOHIDPostEvent 访问未授权，按键注入走 CGEvent")
            NXCloseEventStatusRaw(handle)
            handle = 0
            return
        }
        available = true
        Log.info("驱动层按键注入已启用（IOHIDPostEvent）")
    }

    deinit {
        if handle != 0 { NXCloseEventStatusRaw(handle) }
    }

    /// 发一次完整按键（按下+松开），macOS 虚拟键码。不可用时返回 false。
    @discardableResult
    func sendKey(macKeyCode code: Int64, mods: [String]) -> Bool {
        guard available else { return false }
        var data = NXKeyEventData()
        data.keyCode = UInt16(clamping: code)
        let modFlags = nxFlags(mods)

        if let selfMask = modifierMask(code) {
            // 键本身是修饰键：flagsChanged，按下带自己的 mask、松开清零
            post(Self.NX_FLAGSCHANGED, &data, modFlags | selfMask)
            usleep(10000)
            post(Self.NX_FLAGSCHANGED, &data, 0)
        } else {
            // 普通键（可带组合修饰）：down 带修饰 mask，up 清零
            post(Self.NX_KEYDOWN, &data, modFlags)
            usleep(8000)
            post(Self.NX_KEYUP, &data, 0)
        }
        return true
    }

    private func post(_ type: UInt32, _ data: inout NXKeyEventData, _ flags: IOOptionBits) {
        _ = IOHIDPostEventRaw(
            handle, type, NXIOGPoint(), &data,
            Self.kNXEventDataVersion, flags, Self.kIOHIDSetGlobalEventFlags
        )
    }

    private func nxFlags(_ mods: [String]) -> IOOptionBits {
        var f: IOOptionBits = 0
        for m in mods {
            switch m.lowercased() {
            case "shift": f |= Self.NX_SHIFT
            case "ctrl", "control": f |= Self.NX_CONTROL
            case "alt", "opt", "option": f |= Self.NX_ALT
            case "cmd", "command", "meta", "super": f |= Self.NX_CMD
            case "fn", "function": f |= Self.NX_FN
            default: break
            }
        }
        return f
    }

    /// macOS 虚拟键码 → 修饰键 NX mask（是修饰键则非 nil）
    private func modifierMask(_ code: Int64) -> IOOptionBits? {
        switch code {
        case 56, 60: return Self.NX_SHIFT
        case 59, 62: return Self.NX_CONTROL
        case 58, 61: return Self.NX_ALT
        case 55, 54: return Self.NX_CMD
        case 63: return Self.NX_FN
        default: return nil
        }
    }
}
