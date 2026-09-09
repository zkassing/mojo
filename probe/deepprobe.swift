import Foundation
import IOKit.hid
import CoreGraphics
import AppKit
import IOKit

let VENDOR = 0x2717, PRODUCT = 0x32B8

// ============ 1. HID element 级监听（能抓到被系统驱动接管的键盘 usage）============
let mgr = IOHIDManagerCreate(kCFAllocatorDefault, IOOptionBits(kIOHIDOptionsTypeNone))
IOHIDManagerSetDeviceMatching(mgr, [kIOHIDVendorIDKey: VENDOR, kIOHIDProductIDKey: PRODUCT] as CFDictionary)

func pageName(_ p: UInt32) -> String {
    switch p {
    case 0x01: return "GenericDesktop"
    case 0x07: return "Keyboard"
    case 0x0C: return "Consumer"
    case 0xFF00...0xFFFF: return "Vendor(0x\(String(p, radix:16)))"
    default: return "0x\(String(p, radix: 16))"
    }
}
// Consumer page 常见 usage
let consumerNames: [UInt32: String] = [
    0x30:"Power", 0x40:"Menu", 0x41:"MenuPick", 0x42:"MenuUp", 0x43:"MenuDown",
    0x44:"MenuLeft", 0x45:"MenuRight", 0x46:"MenuEscape",
    0xB0:"Play", 0xB1:"Pause", 0xCD:"PlayPause", 0xB5:"ScanNext", 0xB6:"ScanPrev",
    0xE2:"Mute", 0xE9:"VolUp", 0xEA:"VolDown",
    0x221:"AC_Search", 0x223:"AC_Home", 0x224:"AC_Back", 0x225:"AC_Forward",
    0x226:"AC_Stop", 0x227:"AC_Refresh", 0x21F:"AC_Find",
    0x0CF:"VoiceCommand", 0x1A6:"AL_ContextMenu",
]

func onValue(_ ctx: UnsafeMutableRawPointer?, _ res: IOReturn, _ sender: UnsafeMutableRawPointer?, _ value: IOHIDValue) {
    let el = IOHIDValueGetElement(value)
    let page = IOHIDElementGetUsagePage(el)
    let usage = IOHIDElementGetUsage(el)
    let v = IOHIDValueGetIntegerValue(value)
    // 只看按下，且过滤掉恒为 0 的填充
    // 不过滤：全部打印
    var label = "page=\(pageName(page)) usage=0x\(String(usage, radix:16))"
    if page == 0x0C, let n = consumerNames[usage] { label += " (\(n))" }
    if page == 0x07 { label += " (HID keycode \(usage))" }
    print("HID-ELEM \(v != 0 ? "↓" : "↑") \(label) value=\(v)")
    fflush(stdout)
}

func onMatch(_ c: UnsafeMutableRawPointer?, _ r: IOReturn, _ s: UnsafeMutableRawPointer?, _ d: IOHIDDevice) {
    IOHIDDeviceRegisterInputValueCallback(d, onValue, nil)
    IOHIDDeviceScheduleWithRunLoop(d, CFRunLoopGetMain(), CFRunLoopMode.defaultMode.rawValue)
    // 列出这个 device 支持的所有 element
    if let els = IOHIDDeviceCopyMatchingElements(d, nil, 0) as? [IOHIDElement] {
        var seen = Set<String>()
        var lines = [String]()
        for e in els {
            let p = IOHIDElementGetUsagePage(e), u = IOHIDElementGetUsage(e)
            let key = "\(p):\(u)"
            if seen.contains(key) { continue }
            seen.insert(key)
            var s = "  \(pageName(p)) usage=0x\(String(u, radix:16))"
            if p == 0x0C, let n = consumerNames[u] { s += " (\(n))" }
            lines.append(s)
        }
        let pages = Set(els.map { IOHIDElementGetUsagePage($0) }).sorted()
        print("✅ HID element 监听已启动，element=\(lines.count) 个, usage pages=\(pages.map { pageName($0) })")
        fflush(stdout)
    }
}
IOHIDManagerRegisterDeviceMatchingCallback(mgr, onMatch, nil)
IOHIDManagerScheduleWithRunLoop(mgr, CFRunLoopGetMain(), CFRunLoopMode.defaultMode.rawValue)
IOHIDManagerOpen(mgr, IOOptionBits(kIOHIDOptionsTypeNone))

// ============ 2. CGEventTap：监听「所有」事件类型 ============
var remoteIDs = Set<UInt64>()
do {
    var iter: io_iterator_t = 0
    IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching("IOHIDEventService"), &iter)
    var svc = IOIteratorNext(iter)
    while svc != 0 {
        let vid = (IORegistryEntryCreateCFProperty(svc, "VendorID" as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue() as? NSNumber)?.intValue
        let pid = (IORegistryEntryCreateCFProperty(svc, "ProductID" as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue() as? NSNumber)?.intValue
        if vid == VENDOR && pid == PRODUCT {
            var rid: UInt64 = 0; IORegistryEntryGetRegistryEntryID(svc, &rid); remoteIDs.insert(rid)
        }
        IOObjectRelease(svc); svc = IOIteratorNext(iter)
    }
    IOObjectRelease(iter)
}

func typeName(_ t: CGEventType) -> String {
    switch t.rawValue {
    case 1: return "leftMouseDown"; case 2: return "leftMouseUp"
    case 3: return "rightMouseDown"; case 4: return "rightMouseUp"
    case 5: return "mouseMoved"; case 6: return "leftMouseDragged"
    case 7: return "rightMouseDragged"; case 10: return "keyDown"
    case 11: return "keyUp"; case 12: return "flagsChanged"
    case 13: return "kitDefined"; case 14: return "sysDefined"
    case 15: return "appDefined"; case 22: return "scrollWheel"
    case 23: return "tabletPointer"; case 25: return "otherMouseDown"
    case 26: return "otherMouseUp"; case 27: return "otherMouseDragged"
    default: return "type\(t.rawValue)"
    }
}

let allMask: CGEventMask = ~0
let cb: CGEventTapCallBack = { _, type, ev, _ in
    let sender = UInt64(bitPattern: ev.getIntegerValueField(CGEventField(rawValue: 87)!))
    guard remoteIDs.contains(sender) else { return Unmanaged.passUnretained(ev) }
    var extra = ""
    if type == .keyDown || type == .keyUp {
        extra = " keycode=\(ev.getIntegerValueField(.keyboardEventKeycode))"
    } else if type.rawValue == 14 || type.rawValue == 13 {
        if let ns = NSEvent(cgEvent: ev) {
            extra = " subtype=\(ns.subtype.rawValue) data1=0x\(String(ns.data1, radix:16)) data2=\(ns.data2)"
            if ns.subtype.rawValue == 8 {
                let kc = (ns.data1 & 0xFFFF0000) >> 16
                let st = (ns.data1 & 0xFF00) >> 8
                extra += " → auxKey=\(kc) \(st == 0xA ? "down" : "up")"
            }
        }
    }
    print("CGEVENT \(typeName(type))\(extra)")
    fflush(stdout)
    return Unmanaged.passUnretained(ev)
}
if let tap = CGEvent.tapCreate(tap: .cghidEventTap, place: .headInsertEventTap,
                               options: .listenOnly, eventsOfInterest: allMask,
                               callback: cb, userInfo: nil) {
    CFRunLoopAddSource(CFRunLoopGetMain(), CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0), .commonModes)
    CGEvent.tapEnable(tap: tap, enable: true)
    print("✅ CGEventTap（全事件类型）已启动, remoteIDs=\(remoteIDs.sorted())")
} else {
    print("❌ CGEventTap 创建失败")
}
print("\n>>> 请【按住返回键】3 秒，松开，再连按 3 下 <<<\n")
fflush(stdout)
CFRunLoopRun()
