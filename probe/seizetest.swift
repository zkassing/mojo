import Foundation
import IOKit.hid
let VENDOR = 0x2717, PRODUCT = 0x32B8
let mgr = IOHIDManagerCreate(kCFAllocatorDefault, IOOptionBits(kIOHIDOptionsTypeNone))
IOHIDManagerSetDeviceMatching(mgr, [kIOHIDVendorIDKey: VENDOR, kIOHIDProductIDKey: PRODUCT] as CFDictionary)
IOHIDManagerOpen(mgr, IOOptionBits(kIOHIDOptionsTypeNone))
guard let set = IOHIDManagerCopyDevices(mgr) as? Set<IOHIDDevice>, let dev = set.first else {
    print("no device"); exit(1)
}
let r = IOHIDDeviceOpen(dev, IOOptionBits(kIOHIDOptionsTypeSeizeDevice))
print("SeizeOpen -> \(String(format: "0x%08x", r)) \(r == kIOReturnSuccess ? "OK 可独占" : "FAILED")")
print("uid=\(getuid())")
