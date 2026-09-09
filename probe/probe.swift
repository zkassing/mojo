import Foundation
import IOKit.hid

let VENDOR = 0x2717
let PRODUCT = 0x32B8

func hex(_ b: UnsafePointer<UInt8>, _ len: Int) -> String {
    (0..<len).map { String(format: "%02x", b[$0]) }.joined(separator: " ")
}

let mgr = IOHIDManagerCreate(kCFAllocatorDefault, IOOptionBits(kIOHIDOptionsTypeNone))
let match: [String: Any] = [kIOHIDVendorIDKey: VENDOR, kIOHIDProductIDKey: PRODUCT]
IOHIDManagerSetDeviceMatching(mgr, match as CFDictionary)

func onMatch(_ ctx: UnsafeMutableRawPointer?, _ res: IOReturn, _ sender: UnsafeMutableRawPointer?, _ device: IOHIDDevice) {
    let name = IOHIDDeviceGetProperty(device, kIOHIDProductKey as CFString) as? String ?? "?"
    let maxSize = IOHIDDeviceGetProperty(device, kIOHIDMaxInputReportSizeKey as CFString) as? Int ?? 128
    let usage = IOHIDDeviceGetProperty(device, kIOHIDPrimaryUsageKey as CFString) as? Int ?? -1
    let page = IOHIDDeviceGetProperty(device, kIOHIDPrimaryUsagePageKey as CFString) as? Int ?? -1
    print("matched: \(name) max=\(maxSize) page=\(page) usage=\(usage)")
    fflush(stdout)
    let buf = UnsafeMutablePointer<UInt8>.allocate(capacity: maxSize)
    IOHIDDeviceRegisterInputReportCallback(device, buf, maxSize, { _, _, _, _, rid, rep, len in
        if rid >= 6 { return }
        print("report id=\(rid) len=\(len) data=[\(hex(rep, len))]"); fflush(stdout)
    }, nil)
    IOHIDDeviceScheduleWithRunLoop(device, CFRunLoopGetMain(), CFRunLoopMode.defaultMode.rawValue)
}

IOHIDManagerRegisterDeviceMatchingCallback(mgr, onMatch, nil)
IOHIDManagerScheduleWithRunLoop(mgr, CFRunLoopGetMain(), CFRunLoopMode.defaultMode.rawValue)
let r = IOHIDManagerOpen(mgr, IOOptionBits(kIOHIDOptionsTypeNone))
print("IOHIDManagerOpen -> \(String(format: "0x%08x", r))")
print("请依次按遥控器按键…")
fflush(stdout)
CFRunLoopRun()
