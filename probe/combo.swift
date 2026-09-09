import Foundation
import IOKit.hid
import CoreGraphics
import IOKit

let VENDOR = 0x2717, PRODUCT = 0x32B8

// ---- 找出遥控器 IOHIDEventService 的 RegistryEntryID（用于匹配 CGEvent senderID） ----
var remoteRegIDs = Set<UInt64>()
do {
    var iter: io_iterator_t = 0
    let matching = IOServiceMatching("IOHIDEventService")
    IOServiceGetMatchingServices(kIOMainPortDefault, matching, &iter)
    var svc = IOIteratorNext(iter)
    while svc != 0 {
        var rid: UInt64 = 0
        IORegistryEntryGetRegistryEntryID(svc, &rid)
        let vid = IORegistryEntryCreateCFProperty(svc, "VendorID" as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue() as? Int
        let pid = IORegistryEntryCreateCFProperty(svc, "ProductID" as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue() as? Int
        if vid == VENDOR && pid == PRODUCT {
            remoteRegIDs.insert(rid)
            print("遥控器 IOHIDEventService RegistryEntryID = \(rid)")
        }
        IOObjectRelease(svc)
        svc = IOIteratorNext(iter)
    }
    IOObjectRelease(iter)
}
if remoteRegIDs.isEmpty { print("⚠️ 未找到遥控器的 IOHIDEventService") }

// ---- CGEventTap ----
let mask = (1 << CGEventType.keyDown.rawValue) | (1 << CGEventType.keyUp.rawValue)
          | (1 << CGEventType.flagsChanged.rawValue) | (1 << 14 /* NX_SYSDEFINED */)
let cb: CGEventTapCallBack = { proxy, type, event, _ in
    let kc = event.getIntegerValueField(.keyboardEventKeycode)
    let sender = event.getIntegerValueField(CGEventField(rawValue: 87)!)
    let pid = event.getIntegerValueField(.eventSourceUnixProcessID)
    let tname = type == .keyDown ? "keyDown" : (type == .keyUp ? "keyUp" : (type.rawValue == 14 ? "sysDefined" : "flags/\(type.rawValue)"))
    let isRemote = remoteRegIDs.contains(UInt64(bitPattern: Int64(sender)))
    print("CG \(tname) keycode=\(kc) senderID=\(sender) pid=\(pid) flags=\(event.flags.rawValue) \(isRemote ? "<<< 遥控器" : "")")
    fflush(stdout)
    return Unmanaged.passUnretained(event)
}
guard let tap = CGEvent.tapCreate(tap: .cghidEventTap, place: .headInsertEventTap,
                                  options: .listenOnly, eventsOfInterest: CGEventMask(mask),
                                  callback: cb, userInfo: nil) else {
    print("❌ CGEventTap 创建失败 —— 需要在 系统设置 > 隐私与安全性 > 辅助功能 里授权")
    exit(1)
}
CFRunLoopAddSource(CFRunLoopGetMain(), CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0), .commonModes)
CGEvent.tapEnable(tap: tap, enable: true)
print("✅ CGEventTap 已启动")

// ---- HID 原始报文 ----
let mgr = IOHIDManagerCreate(kCFAllocatorDefault, IOOptionBits(kIOHIDOptionsTypeNone))
IOHIDManagerSetDeviceMatching(mgr, [kIOHIDVendorIDKey: VENDOR, kIOHIDProductIDKey: PRODUCT] as CFDictionary)
func onMatch(_ c: UnsafeMutableRawPointer?, _ r: IOReturn, _ s: UnsafeMutableRawPointer?, _ d: IOHIDDevice) {
    let maxSize = IOHIDDeviceGetProperty(d, kIOHIDMaxInputReportSizeKey as CFString) as? Int ?? 128
    let buf = UnsafeMutablePointer<UInt8>.allocate(capacity: maxSize)
    IOHIDDeviceRegisterInputReportCallback(d, buf, maxSize, { _,_,_,_, rid, rep, len in
        if rid >= 6 { return }
        let h = (0..<len).map { String(format: "%02x", rep[$0]) }.joined(separator: " ")
        print("HID  id=\(rid) len=\(len) [\(h)]"); fflush(stdout)
    }, nil)
    IOHIDDeviceScheduleWithRunLoop(d, CFRunLoopGetMain(), CFRunLoopMode.defaultMode.rawValue)
    print("✅ HID 已连接")
}
IOHIDManagerRegisterDeviceMatchingCallback(mgr, onMatch, nil)
IOHIDManagerScheduleWithRunLoop(mgr, CFRunLoopGetMain(), CFRunLoopMode.defaultMode.rawValue)
IOHIDManagerOpen(mgr, IOOptionBits(kIOHIDOptionsTypeNone))
print("\n>>> 请按遥控器上的键，也按一下笔记本键盘的方向键做对比 <<<\n"); fflush(stdout)
CFRunLoopRun()
