import Foundation
import IOKit
import IOKit.hid

/// 监视目标 HID 设备，维护它对应的 IOHIDEventService RegistryEntryID 集合。
/// CGEvent 的 senderID 字段（field 87）就等于这个 ID，用来判断事件来自哪个物理设备。
final class DeviceWatcher {
    private let vendorId: Int
    private let productId: Int
    private let queue = DispatchQueue(label: "miremote.device")
    private var _ids = Set<UInt64>()
    private var manager: IOHIDManager?
    private var onChange: ((Set<UInt64>) -> Void)?

    init(vendorId: Int, productId: Int) {
        self.vendorId = vendorId
        self.productId = productId
    }

    var registryIds: Set<UInt64> {
        queue.sync { _ids }
    }

    func contains(_ id: UInt64) -> Bool {
        queue.sync { _ids.contains(id) }
    }

    /// 立即扫一遍 IORegistry
    func refresh() {
        var found = Set<UInt64>()
        for className in ["IOHIDEventService", "IOHIDDevice", "AppleUserHIDEventService"] {
            var iter: io_iterator_t = 0
            guard let matching = IOServiceMatching(className) else { continue }
            guard IOServiceGetMatchingServices(kIOMainPortDefault, matching, &iter) == KERN_SUCCESS else { continue }
            var svc = IOIteratorNext(iter)
            while svc != 0 {
                if intProp(svc, "VendorID") == vendorId, intProp(svc, "ProductID") == productId {
                    var rid: UInt64 = 0
                    if IORegistryEntryGetRegistryEntryID(svc, &rid) == KERN_SUCCESS {
                        found.insert(rid)
                    }
                }
                IOObjectRelease(svc)
                svc = IOIteratorNext(iter)
            }
            IOObjectRelease(iter)
        }
        let changed: Bool = queue.sync {
            if found == _ids { return false }
            _ids = found
            return true
        }
        if changed {
            Log.debug("设备 registryID 更新: \(found.sorted())")
            onChange?(found)
        }
    }

    /// 订阅设备插拔，自动 refresh
    func start(onChange: ((Set<UInt64>) -> Void)? = nil) {
        self.onChange = onChange
        refresh()

        let mgr = IOHIDManagerCreate(kCFAllocatorDefault, IOOptionBits(kIOHIDOptionsTypeNone))
        manager = mgr
        IOHIDManagerSetDeviceMatching(mgr, [
            kIOHIDVendorIDKey: vendorId,
            kIOHIDProductIDKey: productId
        ] as CFDictionary)

        let ctx = Unmanaged.passUnretained(self).toOpaque()
        let cb: IOHIDDeviceCallback = { context, _, _, _ in
            guard let context else { return }
            let me = Unmanaged<DeviceWatcher>.fromOpaque(context).takeUnretainedValue()
            // 设备刚上线时 IOHIDEventService 可能还没建好，稍等
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) { me.refresh() }
        }
        IOHIDManagerRegisterDeviceMatchingCallback(mgr, cb, ctx)
        IOHIDManagerRegisterDeviceRemovalCallback(mgr, cb, ctx)
        IOHIDManagerScheduleWithRunLoop(mgr, CFRunLoopGetMain(), CFRunLoopMode.commonModes.rawValue)
        IOHIDManagerOpen(mgr, IOOptionBits(kIOHIDOptionsTypeNone))

        // 蓝牙设备偶尔不触发回调，兜底定时刷新
        let timer = Timer(timeInterval: 5.0, repeats: true) { [weak self] _ in self?.refresh() }
        RunLoop.main.add(timer, forMode: .common)
    }

    /// 列出所有已连接 HID 设备（供 devices 子命令）
    static func listAll() -> [(vendor: Int, product: Int, name: String, registryIds: [UInt64], transport: String)] {
        var byKey = [String: (vendor: Int, product: Int, name: String, ids: Set<UInt64>, transport: String)]()
        for className in ["IOHIDEventService", "IOHIDDevice"] {
            var iter: io_iterator_t = 0
            guard let matching = IOServiceMatching(className) else { continue }
            guard IOServiceGetMatchingServices(kIOMainPortDefault, matching, &iter) == KERN_SUCCESS else { continue }
            var svc = IOIteratorNext(iter)
            while svc != 0 {
                defer { IOObjectRelease(svc); svc = IOIteratorNext(iter) }
                guard let v = intProp(svc, "VendorID"), let p = intProp(svc, "ProductID") else { continue }
                let name = strProp(svc, "Product") ?? strProp(svc, "DeviceAddress") ?? "(无名)"
                let transport = strProp(svc, "Transport") ?? ""
                var rid: UInt64 = 0
                IORegistryEntryGetRegistryEntryID(svc, &rid)
                let key = "\(v):\(p)"
                var e = byKey[key] ?? (v, p, name, [], transport)
                if e.name == "(无名)" && name != "(无名)" { e.name = name }
                if e.transport.isEmpty { e.transport = transport }
                e.ids.insert(rid)
                byKey[key] = e
            }
            IOObjectRelease(iter)
        }
        return byKey.values
            .map { (vendor: $0.vendor, product: $0.product, name: $0.name,
                    registryIds: $0.ids.sorted(), transport: $0.transport) }
            .sorted { $0.name < $1.name }
    }

    private func intProp(_ svc: io_service_t, _ key: String) -> Int? {
        DeviceWatcher.intProp(svc, key)
    }

    fileprivate static func intProp(_ svc: io_service_t, _ key: String) -> Int? {
        guard let v = IORegistryEntryCreateCFProperty(svc, key as CFString, kCFAllocatorDefault, 0)?
            .takeRetainedValue() else { return nil }
        return (v as? NSNumber)?.intValue
    }

    fileprivate static func strProp(_ svc: io_service_t, _ key: String) -> String? {
        guard let v = IORegistryEntryCreateCFProperty(svc, key as CFString, kCFAllocatorDefault, 0)?
            .takeRetainedValue() else { return nil }
        return v as? String
    }
}

private func intProp(_ svc: io_service_t, _ key: String) -> Int? { DeviceWatcher.intProp(svc, key) }
private func strProp(_ svc: io_service_t, _ key: String) -> String? { DeviceWatcher.strProp(svc, key) }
