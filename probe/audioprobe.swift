import Foundation
import IOKit.hid
let VENDOR = 0x2717, PRODUCT = 0x32B8
var counts = [UInt32: Int]()
var bytes = [UInt32: Int]()
var firstSample = [UInt32: String]()
var startTime: Date? = nil
let mgr = IOHIDManagerCreate(kCFAllocatorDefault, IOOptionBits(kIOHIDOptionsTypeNone))
IOHIDManagerSetDeviceMatching(mgr, [kIOHIDVendorIDKey: VENDOR, kIOHIDProductIDKey: PRODUCT] as CFDictionary)
func onMatch(_ c: UnsafeMutableRawPointer?, _ r: IOReturn, _ s: UnsafeMutableRawPointer?, _ d: IOHIDDevice) {
    let maxSize = IOHIDDeviceGetProperty(d, kIOHIDMaxInputReportSizeKey as CFString) as? Int ?? 128
    let buf = UnsafeMutablePointer<UInt8>.allocate(capacity: maxSize)
    IOHIDDeviceRegisterInputReportCallback(d, buf, maxSize, { _,_,_,_, rid, rep, len in
        if startTime == nil { startTime = Date() }
        counts[rid, default: 0] += 1
        bytes[rid, default: 0] += len
        if firstSample[rid] == nil {
            firstSample[rid] = (0..<min(len,32)).map { String(format: "%02x", rep[$0]) }.joined(separator: " ")
        }
    }, nil)
    IOHIDDeviceScheduleWithRunLoop(d, CFRunLoopGetMain(), CFRunLoopMode.defaultMode.rawValue)
    print("✅ HID 已连接 (maxInputReportSize=\(maxSize))")
    fflush(stdout)
}
IOHIDManagerRegisterDeviceMatchingCallback(mgr, onMatch, nil)
IOHIDManagerScheduleWithRunLoop(mgr, CFRunLoopGetMain(), CFRunLoopMode.defaultMode.rawValue)
IOHIDManagerOpen(mgr, IOOptionBits(kIOHIDOptionsTypeNone))
let t = Timer(timeInterval: 2.0, repeats: true) { _ in
    guard !counts.isEmpty else { print("… 暂无数据"); fflush(stdout); return }
    let dur = Date().timeIntervalSince(startTime ?? Date())
    print("\n=== 累计 \(String(format: "%.1f", dur))s ===")
    for (rid, n) in counts.sorted(by: { $0.key < $1.key }) {
        let b = bytes[rid] ?? 0
        print("  report id=\(rid): \(n) 帧, \(b) 字节, \(String(format: "%.0f", Double(b)/max(dur,0.01))) B/s")
        print("     首帧: \(firstSample[rid] ?? "")")
    }
    fflush(stdout)
}
RunLoop.main.add(t, forMode: .common)
print(">>> 请【按住】遥控器的语音键（话筒图标）说几句话，保持按住 5 秒 <<<\n"); fflush(stdout)
CFRunLoopRun()
