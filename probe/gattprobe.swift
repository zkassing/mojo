// gattprobe.swift — CoreBluetooth GATT 探测器（小米蓝牙语音遥控器 / Android TV ATVV）
//
// 编译:  swiftc -O gattprobe.swift -o gattprobe
// 但要真跑起来必须打成 .app：./build-gattprobe.sh  然后
//        open ./GattProbe.app --stdout /tmp/gp.log --args --handshake
//
// 为什么裸 binary 跑不了：macOS 的蓝牙 TCC 只认真实 .app bundle 的 Info.plist。
// 裸命令行程序（哪怕用 -Xlinker -sectcreate 把 plist 嵌进 __TEXT,__info_plist）
// 一碰 CBCentralManager 就被 tccd SIGABRT 掉，crash log 里是：
//   namespace TCC / "must contain an NSBluetoothAlwaysUsageDescription key"
// 实测于 macOS 27 (26A5425a), Apple Silicon。
//
// ======== 本机实测结论（小米蓝牙语音遥控器, VID 0x2717 PID 0x32B8）========
//
// 1) CoreBluetooth 能和系统 HID 驱动【同时】持有这条 BLE 链路，不需要断开配对：
//    retrieveConnectedPeripherals(withServices:) 直接拿到已连接的 peripheral，
//    connect() 立即成功，GATT 读写全部正常，按键映射同时照常工作。
//    注意：不能靠 scanForPeripherals —— 遥控器已连接时不广播，扫描永远找不到它。
//
// 2) 实测 service 列表（HoGP 0x1812 被 macOS 对第三方隐藏，其余可见）：
//      180F Battery / 180A Device Info
//      AB5E0001-…  ATVV = Google ATV Voice Service   ← 语音走这里
//      8A7A0001-…  小米自定义服务
//      01BF / FE59 (Nordic DFU)
//
// 3) ATVV 握手实测成功（30 ms 内拿到回包）：
//      --> TX  0a 00 01 00 03              GET_CAPS
//      <-- CTL 0b 00 01 00 02 00 86 00 86  CAPS_RESP
//          → spec 0x0001, codec 0x0002 = ADPCM 16kHz/16bit,
//            帧长 134B, 每个 notification 134B
//      --> TX  0c 02                       MIC_OPEN（2 字节变体被接受）
//      <-- CTL 04 00 02 00                 AUDIO_START reason=0（MIC_OPEN 触发）
//      --> TX  0d 00                       MIC_CLOSE
//      <-- CTL 00 00                       AUDIO_STOP
//    ATT 载荷 244 字节（MTU 247），单个 notification 足够装下整帧 134B。
//
// 4) 所以「HID vendor report 6/7/8 收不到数据」是正常的：这台遥控器走 VoGP
//    （音频在独立 GATT 服务里），不是老款 ADT-1/Nexus Player 的 VoHoGP
//    （音频塞在 HID report 里）。那 3 个 120 字节 vendor report 是空壳。
//    不需要 kIOHIDOptionsTypeSeizeDevice，也不需要 root。
//
// 5) 音频帧需要真人按住语音键才会流出（AUDIO_START 只代表会话已建立）。
//    用 ./capture.sh 抓，再用 decode_adpcm.py 解成 wav。
//
// 协议依据：Google "Voice over BLE" spec v1.0（ATVV），
// 帧头结构对照 Infineon CYW20829 voice-remote 参考固件。
//
// 实现要点：
//  1. macOS CoreBluetooth 不暴露 MAC 地址，只能按 name / service UUID 匹配。
//  2. 命令行程序必须自己跑 RunLoop，否则 CB 的回调永远不会触发。
//  3. 蓝牙 TCC 权限：裸 binary 的权限归属于父进程（Terminal / iTerm）。
//     若 state == .unauthorized，见文件末尾注释里的 .app bundle 方案。

import Foundation
import CoreBluetooth

// ---------------------------------------------------------------- 常量

let ATVV_SVC = CBUUID(string: "AB5E0001-5A21-4F05-BC7D-AF01F617B664")
let ATVV_TX = CBUUID(string: "AB5E0002-5A21-4F05-BC7D-AF01F617B664") // write, host -> remote
let ATVV_AUDIO = CBUUID(string: "AB5E0003-5A21-4F05-BC7D-AF01F617B664") // notify, ADPCM
let ATVV_CTL = CBUUID(string: "AB5E0004-5A21-4F05-BC7D-AF01F617B664") // notify, control

// retrieveConnectedPeripherals 需要 service 过滤种子。把标准 + 厂商服务都塞进去。
let SEED: [CBUUID] = [
    ATVV_SVC,
    CBUUID(string: "1812"), // HID over GATT
    CBUUID(string: "180F"), // Battery
    CBUUID(string: "180A"), // Device Information
    CBUUID(string: "1800"), // Generic Access
    CBUUID(string: "FFF0"), // 常见厂商服务（G20S 等）
    CBUUID(string: "FE2C"), // Google Fast Pair
    CBUUID(string: "FD5A"), // Google
]

let NAME_HINTS = ["小米", "xiaomi", "mi ", "remote", "遥控", "voice", "atv", "bluetooth voice"]

// ATVV 命令 / 控制码
let GET_CAPS_V1 = Data([0x0A, 0x00, 0x01, 0x00, 0x03]) // version 0.1? -> 用 spec 的 (ver16, codecs16) 大端
let CTL_NAMES: [UInt8: String] = [
    0x00: "AUDIO_STOP", 0x04: "AUDIO_START", 0x08: "START_SEARCH",
    0x0A: "AUDIO_SYNC", 0x0B: "CAPS_RESP", 0x0C: "MIC_OPEN_ERROR",
]

// ---------------------------------------------------------------- CLI

struct Opts {
    var handshake = false
    var micopen = false
    var forceScan = false
    var timeout: Double = 45
    var capsVersion: UInt16 = 0x0001
    var capsCodecs: UInt16 = 0x0003
    var writeNoResp = false
    var subscribeAll = false
    var dumpRaw: String?
}
var opts = Opts()
do {
    var it = CommandLine.arguments.dropFirst().makeIterator()
    while let a = it.next() {
        func u16(_ s: String?) -> UInt16 {
            guard let s = s else { return 0 }
            return s.hasPrefix("0x") ? (UInt16(s.dropFirst(2), radix: 16) ?? 0) : (UInt16(s) ?? 0)
        }
        switch a {
        case "--handshake": opts.handshake = true
        case "--micopen": opts.handshake = true; opts.micopen = true
        case "--scan": opts.forceScan = true
        case "--write-noresp": opts.writeNoResp = true
        case "--subscribe-all": opts.handshake = true; opts.subscribeAll = true
        case "--timeout": opts.timeout = Double(it.next() ?? "45") ?? 45
        case "--caps-version": opts.capsVersion = u16(it.next())
        case "--caps-codecs": opts.capsCodecs = u16(it.next())
        case "--dump-raw": opts.dumpRaw = it.next()
        case "-h", "--help":
            print("""
            usage: gattprobe [options]
              --handshake          枚举后订阅 ATVV CTL/AUDIO 并发 GET_CAPS
              --micopen            同上，并不等按键直接发 MIC_OPEN（on-request 模式）
              --scan               强制走广播扫描（默认优先用已连接列表）
              --write-noresp       TX 用 write-without-response 发命令
              --subscribe-all      订阅设备上所有 notify 特征（查音频是不是走小米自定义服务）
              --timeout SECS       总超时，默认 45
              --caps-version N     GET_CAPS 声明的 spec 版本（1 / 0x0100 / 0x0004）
              --caps-codecs N      GET_CAPS 声明的 codec 位图（1=8k, 2=16k, 3=两者）
              --dump-raw FILE      把原始 ADPCM 帧字节落盘（供离线解码验证）
            """)
            exit(0)
        default: FileHandle.standardError.write("unknown arg: \(a)\n".data(using: .utf8)!)
        }
    }
}

let t0 = Date()
func log(_ s: String) {
    let t = String(format: "%6.2f", Date().timeIntervalSince(t0))
    print("[\(t)] \(s)")
    fflush(stdout)
}

func propsDesc(_ p: CBCharacteristicProperties) -> String {
    var v: [String] = []
    if p.contains(.broadcast) { v.append("broadcast") }
    if p.contains(.read) { v.append("read") }
    if p.contains(.writeWithoutResponse) { v.append("write-no-resp") }
    if p.contains(.write) { v.append("write") }
    if p.contains(.notify) { v.append("notify") }
    if p.contains(.indicate) { v.append("indicate") }
    if p.contains(.authenticatedSignedWrites) { v.append("signed-write") }
    if p.contains(.extendedProperties) { v.append("ext-props") }
    if p.contains(.notifyEncryptionRequired) { v.append("notify-enc") }
    if p.contains(.indicateEncryptionRequired) { v.append("indicate-enc") }
    return v.isEmpty ? "none" : v.joined(separator: ",")
}

func hex(_ d: Data) -> String { d.map { String(format: "%02x", $0) }.joined(separator: " ") }

func annotate(_ u: CBUUID) -> String {
    switch u.uuidString.uppercased() {
    case "1800": return "  <- Generic Access"
    case "1801": return "  <- Generic Attribute"
    case "180A": return "  <- Device Information"
    case "180F": return "  <- Battery"
    case "1812": return "  <- HID over GATT (HoGP)"
    case ATVV_SVC.uuidString.uppercased(): return "  <<<=== ATVV (Google ATV Voice Service)"
    case ATVV_TX.uuidString.uppercased(): return "  <- ATVV TX (host->remote cmds)"
    case ATVV_AUDIO.uuidString.uppercased(): return "  <- ATVV AUDIO (ADPCM notify)"
    case ATVV_CTL.uuidString.uppercased(): return "  <- ATVV CTL (control notify)"
    case "FFF0": return "  <- vendor 0xFFF0"
    case "FE2C": return "  <- Google Fast Pair"
    default: return ""
    }
}

// ---------------------------------------------------------------- Probe

final class Probe: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    var central: CBCentralManager!
    var target: CBPeripheral?
    var pendingServices = 0
    var tx: CBCharacteristic?
    var audioFrames = 0
    var audioBytes = 0
    var streamId: UInt8 = 0
    var rawAudio = Data()
    var otherNotifies = 0
    var firstAudioSizes: [Int] = []
    var enumerated = false

    func start() {
        central = CBCentralManager(delegate: self, queue: .main)
    }

    // ---- state
    func centralManagerDidUpdateState(_ c: CBCentralManager) {
        let names: [CBManagerState: String] = [
            .unknown: "unknown", .resetting: "resetting", .unsupported: "unsupported",
            .unauthorized: "unauthorized", .poweredOff: "poweredOff", .poweredOn: "poweredOn",
        ]
        log("CBCentralManager state = \(names[c.state] ?? "?")")
        switch c.state {
        case .unauthorized:
            print("""

            ❌ 蓝牙权限被拒 (.unauthorized)
               裸命令行 binary 的 TCC 权限归属父进程（Terminal/iTerm/VS Code）。
               解决：系统设置 → 隐私与安全性 → 蓝牙 → 勾上你的终端 App；
               或把它打成 .app bundle（见文件末尾注释）。
            """)
            exit(2)
        case .unsupported:
            print("❌ 本机不支持 BLE"); exit(2)
        case .poweredOff:
            print("❌ 蓝牙已关闭，请打开后重试"); exit(2)
        case .poweredOn:
            findTarget()
        default: break
        }
    }

    func findTarget() {
        if !opts.forceScan {
            let connected = central.retrieveConnectedPeripherals(withServices: SEED)
            log("retrieveConnectedPeripherals → \(connected.count) 个已连接设备")
            for p in connected {
                log("   • \(p.name ?? "(无名)")  id=\(p.identifier.uuidString)")
            }
            // 先按名字挑，挑不到就取第一个（可能 name 为空）
            let pick = connected.first { p in
                guard let n = p.name?.lowercased() else { return false }
                return NAME_HINTS.contains { n.contains($0) }
            } ?? connected.first
            if let p = pick {
                log("✅ 命中已连接设备: \(p.name ?? "(无名)") — 直接 connect（不需要扫描）")
                attach(p)
                return
            }
            log("已连接列表里没有候选，回退到广播扫描…")
        }
        log("开始扫描（遥控器空闲时不广播；按任意键唤醒它可能有帮助）")
        central.scanForPeripherals(withServices: nil,
                                   options: [CBCentralManagerScanOptionAllowDuplicatesKey: false])
    }

    func attach(_ p: CBPeripheral) {
        target = p
        p.delegate = self
        central.stopScan()
        central.connect(p, options: nil)
    }

    var seenAds = Set<UUID>()
    func centralManager(_ c: CBCentralManager, didDiscover p: CBPeripheral,
                        advertisementData ad: [String: Any], rssi: NSNumber) {
        let advName = (ad[CBAdvertisementDataLocalNameKey] as? String) ?? p.name ?? ""
        let svcs = (ad[CBAdvertisementDataServiceUUIDsKey] as? [CBUUID]) ?? []
        if seenAds.insert(p.identifier).inserted {
            log("adv: \(advName.isEmpty ? "(无名)" : advName)  rssi=\(rssi)  svcs=\(svcs.map { $0.uuidString })")
        }
        let n = advName.lowercased()
        let nameHit = !n.isEmpty && NAME_HINTS.contains { n.contains($0) }
        let svcHit = svcs.contains(ATVV_SVC) || svcs.contains(CBUUID(string: "1812"))
        if nameHit || svcHit {
            log("✅ 扫描命中: \(advName) — connect")
            attach(p)
        }
    }

    func centralManager(_ c: CBCentralManager, didConnect p: CBPeripheral) {
        log("已连接: \(p.name ?? "(无名)")  — discoverServices(nil)")
        p.discoverServices(nil)
    }

    func centralManager(_ c: CBCentralManager, didFailToConnect p: CBPeripheral, error: Error?) {
        log("❌ 连接失败: \(error?.localizedDescription ?? "?")")
        exit(3)
    }

    func centralManager(_ c: CBCentralManager, didDisconnectPeripheral p: CBPeripheral, error: Error?) {
        log("⚠️ 断开: \(error?.localizedDescription ?? "正常断开")")
        if opts.handshake { log("（handshake 模式下断开通常意味着系统 HID 驱动抢回了链路）") }
    }

    // ---- services / characteristics
    func peripheral(_ p: CBPeripheral, didDiscoverServices error: Error?) {
        if let e = error { log("❌ discoverServices: \(e.localizedDescription)"); exit(3) }
        let svcs = p.services ?? []
        print("")
        print("================ GATT DUMP: \(p.name ?? "(无名)") ================")
        print("peripheral identifier: \(p.identifier.uuidString)")
        print("services: \(svcs.count)")
        pendingServices = svcs.count
        if svcs.isEmpty {
            print("⚠️ 一个 service 都没发现 —— 说明 CoreBluetooth 拿不到这条链路的 GATT 数据库")
            finish()
            return
        }
        for s in svcs { p.discoverCharacteristics(nil, for: s) }
    }

    func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor s: CBService, error: Error?) {
        if let e = error {
            print("❌ chars for \(s.uuid.uuidString): \(e.localizedDescription)")
        }
        for ch in s.characteristics ?? [] where ch.uuid == ATVV_TX { tx = ch }
        pendingServices -= 1
        if pendingServices <= 0 { servicesDone(p) }
    }

    func servicesDone(_ p: CBPeripheral) {
        guard !enumerated else { return }
        enumerated = true
        // 所有 characteristic 都发现完了再按 service 分组打印，避免回调交错导致输出错位
        for s in p.services ?? [] {
            print("")
            print("[service] \(s.uuid.uuidString)\(annotate(s.uuid))  primary=\(s.isPrimary)")
            let chs = s.characteristics ?? []
            if chs.isEmpty { print("    (无 characteristic)") }
            for ch in chs {
                print("    char \(ch.uuid.uuidString)  props=[\(propsDesc(ch.properties))]\(annotate(ch.uuid))")
            }
        }
        print("")
        print("================ 结论 ================")
        let hasATVV = (p.services ?? []).contains { $0.uuid == ATVV_SVC }
        let hasHID = (p.services ?? []).contains { $0.uuid == CBUUID(string: "1812") }
        // ATT MTU 诊断：maximumWriteValueLength(.withoutResponse) == MTU - 3。
        // ATVV 帧 134 字节，需要 MTU >= 137；若链路停在默认 MTU 23，遥控器根本发不出整帧。
        let payloadCap = p.maximumWriteValueLength(for: .withoutResponse)
        print("ATT 有效载荷  : \(payloadCap) 字节  (≈ MTU \(payloadCap + 3))")
        print("ATVV 语音服务 (ab5e0001…): \(hasATVV ? "✅ 存在" : "❌ 不存在")")
        print("HID over GATT (0x1812):    \(hasHID ? "✅ 可见" : "❌ 不可见（macOS 通常对第三方隐藏 HoGP）")")
        if hasATVV {
            print("→ 语音可以走 ATVV：TX 写 GET_CAPS / MIC_OPEN，AUDIO 收 IMA-ADPCM 帧。")
            print("   加 --handshake 跑真实握手。")
        }
        print("")
        fflush(stdout)

        if opts.handshake, hasATVV {
            startHandshake(p)
        } else {
            finish()
        }
    }

    // ---- handshake（可选）
    func startHandshake(_ p: CBPeripheral) {
        guard let s = (p.services ?? []).first(where: { $0.uuid == ATVV_SVC }) else { finish(); return }
        for ch in s.characteristics ?? [] where ch.uuid == ATVV_CTL || ch.uuid == ATVV_AUDIO {
            log("setNotifyValue(true) → \(ch.uuid.uuidString)")
            p.setNotifyValue(true, for: ch)
        }
        guard opts.subscribeAll else { return }
        // 把其余所有 notify 特征也订阅上，看音频/按键状态是不是走小米自定义通道
        for svc in p.services ?? [] where svc.uuid != ATVV_SVC {
            for ch in svc.characteristics ?? []
            where ch.properties.contains(.notify) || ch.properties.contains(.indicate) {
                log("setNotifyValue(true) → \(svc.uuid.uuidString)/\(ch.uuid.uuidString)")
                p.setNotifyValue(true, for: ch)
            }
        }
    }

    var capsSent = false
    var micOpenTries = 0
    /// MIC_OPEN 长度/字节序在不同固件上不一致：
    ///   · Google Chromecast Remote 只接受恰好 2 字节 (0x0C + 1B)
    ///   · G20S Pro 只接受 3 字节大端 codec (0x0C 0x00 0x02)
    /// 所以逐个重试，收到 MIC_OPEN_ERROR 就换下一种。
    let micOpenVariants: [Data] = [
        Data([0x0C, 0x02]),
        Data([0x0C, 0x00, 0x02]),
        Data([0x0C, 0x01]),
        Data([0x0C, 0x00, 0x01]),
    ]

    var writeType: CBCharacteristicWriteType { opts.writeNoResp ? .withoutResponse : .withResponse }

    func sendMicOpen(_ p: CBPeripheral) {
        guard let tx = tx, micOpenTries < micOpenVariants.count else {
            if micOpenTries >= micOpenVariants.count { log("⚠️ MIC_OPEN 所有变体均被拒") }
            return
        }
        let d = micOpenVariants[micOpenTries]
        micOpenTries += 1
        log("--> TX MIC_OPEN(变体\(micOpenTries)) \(hex(d))")
        p.writeValue(d, for: tx, type: writeType)
    }

    func peripheral(_ p: CBPeripheral, didUpdateNotificationStateFor ch: CBCharacteristic, error: Error?) {
        if let e = error {
            log("❌ notify \(ch.uuid.uuidString): \(e.localizedDescription)")
            return
        }
        log("notify=\(ch.isNotifying) on \(ch.uuid.uuidString)")
        if ch.uuid == ATVV_CTL, ch.isNotifying, !capsSent, let tx = tx {
            capsSent = true
            // spec: GET_CAPS = 0x0A + version(2, BE) + codecs(2, BE)
            // codecs 0x0003 = 声明同时支持 ADPCM 8k 和 16k，让遥控器自己挑
            let v = opts.capsVersion, c = opts.capsCodecs
            let caps = Data([0x0A, UInt8(v >> 8), UInt8(v & 0xFF), UInt8(c >> 8), UInt8(c & 0xFF)])
            log("--> TX GET_CAPS \(hex(caps))")
            p.writeValue(caps, for: tx, type: writeType)
        }
    }

    func peripheral(_ p: CBPeripheral, didWriteValueFor ch: CBCharacteristic, error: Error?) {
        if let e = error { log("❌ write \(ch.uuid.uuidString): \(e.localizedDescription)") }
    }

    func peripheral(_ p: CBPeripheral, didUpdateValueFor ch: CBCharacteristic, error: Error?) {
        if let e = error { log("❌ update \(ch.uuid.uuidString): \(e.localizedDescription)"); return }
        guard let d = ch.value else { return }
        if ch.uuid == ATVV_AUDIO {
            audioFrames += 1
            audioBytes += d.count
            rawAudio.append(d)
            if firstAudioSizes.count < 5 { firstAudioSizes.append(d.count) }
            if audioFrames <= 3 || audioFrames % 25 == 0 {
                log("AUDIO frame #\(audioFrames) len=\(d.count) head=\(hex(d.prefix(8)))")
            }
        } else if ch.uuid == ATVV_CTL {
            let op = d.first ?? 0
            log("CTL \(CTL_NAMES[op] ?? String(format: "0x%02x", op)): \(hex(d))")
            switch op {
            case 0x0B: // CAPS_RESP
                describeCaps(d)
                if opts.micopen {
                    log("on-request 模式测试：不等按键，直接发 MIC_OPEN")
                    sendMicOpen(p)
                } else {
                    print("\n>>> 现在按住遥控器语音键说话（收到 START_SEARCH 会自动回 MIC_OPEN）\n")
                    fflush(stdout)
                }
            case 0x08: // START_SEARCH
                micOpenTries = 0
                sendMicOpen(p)
            case 0x04: // AUDIO_START
                let reason = d.count > 1 ? d[1] : 0
                let codec = d.count > 2 ? d[2] : 0
                let sid = d.count > 3 ? d[3] : 0
                let rn = [UInt8(0): "MIC_OPEN 触发", 1: "PTT 按键触发", 3: "HTT 按住触发"][reason] ?? "?"
                let cn = codec == 1 ? "ADPCM 8kHz/16bit" : (codec == 2 ? "ADPCM 16kHz/16bit" : "?")
                log("✅ AUDIO_START reason=\(rn) codec=\(cn) streamId=\(sid)")
                streamId = sid
                let cap = p.maximumWriteValueLength(for: .withoutResponse)
                log("当前 ATT 载荷 \(cap) 字节（需 ≥134 才能装下一帧）")
            case 0x00: // AUDIO_STOP
                let r = d.count > 1 ? d[1] : 0
                let rn = [UInt8(0x00): "MIC_CLOSE", 0x02: "HTT 释放按键", 0x04: "即将重新 AUDIO_START",
                          0x08: "传输超时", 0x10: "AUDIO 通知被关", 0x80: "其他"][r] ?? "?"
                log("AUDIO_STOP reason=\(rn)")
            case 0x0C: // MIC_OPEN_ERROR
                log("⚠️ MIC_OPEN 被拒 (payload \(hex(d.dropFirst())))，换下一个变体")
                sendMicOpen(p)
            default: break
            }
        } else {
            otherNotifies += 1
            log("🔔 \(ch.service?.uuid.uuidString ?? "?")/\(ch.uuid.uuidString) len=\(d.count): \(hex(d.prefix(24)))")
        }
    }

    /// CAPS_RESP = 0x0B + version(2 BE) + codecs(2 BE) + frameSize(2 BE) + bytesPerChar(2 BE) [+ fw data]
    func describeCaps(_ d: Data) {
        let b = [UInt8](d)
        func be(_ i: Int) -> Int { b.count > i + 1 ? Int(b[i]) << 8 | Int(b[i + 1]) : -1 }
        let ver = be(1), codecs = be(3), frame = be(5), perChar = be(7)
        let codecName: String
        switch codecs {
        case 1: codecName = "ADPCM 8kHz/16bit"
        case 2: codecName = "ADPCM 16kHz/16bit"
        case 3: codecName = "ADPCM 8k+16k (动态带宽)"
        default: codecName = "0x" + String(format: "%04x", codecs)
        }
        let payload = max(frame - 6, 0)
        let rate = codecs == 1 ? 8.0 : 16.0
        print("""
            ---- CAPS_RESP 解析 ----
            spec version     : 0x\(String(format: "%04x", ver))
            codecs supported : 0x\(String(format: "%04x", codecs))  → \(codecName)
            audio frame size : \(frame) 字节
            bytes per notify : \(perChar) 字节\(perChar >= frame ? "  (整帧可装进单个 notification)" : "")
            帧结构推断     : 3B 序号头 + 3B DVI(pred16BE + stepIdx) + \(payload)B ADPCM
                               = \(payload * 2) 样本/帧 ≈ \(String(format: "%.1f", Double(payload * 2) / rate)) ms
            ------------------------
            """)
        fflush(stdout)
    }

    func finish() {
        if opts.handshake {
            print("")
            print("audio frames=\(audioFrames) bytes=\(audioBytes) 前几帧长度=\(firstAudioSizes)")
            if opts.subscribeAll { print("其他特征通知数=\(otherNotifies)") }
            if audioFrames > 0 {
                print("✅ 收到音频帧 —— ATVV 通路打通")
                if let path = opts.dumpRaw {
                    try? rawAudio.write(to: URL(fileURLWithPath: path))
                    print("原始帧已写入 \(path) (\(rawAudio.count) 字节)")
                }
            } else {
                print("⚠️ 没收到音频帧")
            }
            if let p = target, let tx = tx {
                let close = Data([0x0D, streamId]) // MIC_CLOSE + stream id
                log("--> TX MIC_CLOSE \(hex(close))")
                p.writeValue(close, for: tx, type: writeType)
                // 给写入一点时间落地，否则遥控器会被留在开麦状态
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
                    print("\n[done]")
                    fflush(stdout)
                    exit(0)
                }
                return
            }
        }
        print("\n[done]")
        fflush(stdout)
        exit(0)
    }
}

// ---------------------------------------------------------------- main

let probe = Probe()
probe.start()

DispatchQueue.main.asyncAfter(deadline: .now() + opts.timeout) {
    log("⏱ 超时 \(Int(opts.timeout))s")
    probe.finish()
}

// CoreBluetooth 需要活跃的 RunLoop，命令行程序必须自己跑
RunLoop.main.run()

// ---------------------------------------------------------------------------
// 如果 state == .unauthorized：把它包成 .app bundle
//
//   mkdir -p GattProbe.app/Contents/MacOS
//   cp gattprobe GattProbe.app/Contents/MacOS/GattProbe
//   cat > GattProbe.app/Contents/Info.plist <<'PLIST'
//   <?xml version="1.0" encoding="UTF-8"?>
//   <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
//     "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
//   <plist version="1.0"><dict>
//     <key>CFBundleExecutable</key><string>GattProbe</string>
//     <key>CFBundleIdentifier</key><string>com.miremote.gattprobe</string>
//     <key>CFBundleName</key><string>GattProbe</string>
//     <key>CFBundlePackageType</key><string>APPL</string>
//     <key>NSBluetoothAlwaysUsageDescription</key>
//     <string>读取遥控器语音数据</string>
//   </dict></plist>
//   PLIST
//   codesign --force --sign - GattProbe.app
//   ./GattProbe.app/Contents/MacOS/GattProbe        # 直接跑，权限归 bundle id
// ---------------------------------------------------------------------------
