import Foundation
import CoreBluetooth

/// Google ATVV (Android TV Voice over BLE) 客户端。
///
/// 遥控器的麦克风不走 HID，而走这个独立 GATT 服务。
/// CoreBluetooth 可以和系统 HID 驱动同时持有同一条 BLE 链路，无需断开配对。
///
/// 实测协议要点（小米蓝牙语音遥控器，固件 2671）：
///   - notification 每包 **120 字节裸 ADPCM，没有帧头**
///     （CAPS_RESP 宣称 134 = 6 头 + 128 数据，但真实推流不是这样）
///   - 解码器状态 **跨包连续**，不能逐包重置
///   - nibble 顺序是 **高 4 位先**
///   - IMA/DVI ADPCM 4-bit, 16 kHz, 16-bit, 单声道
final class ATVVClient: NSObject {

    // MARK: GATT UUID

    static let serviceUUID = CBUUID(string: "AB5E0001-5A21-4F05-BC7D-AF01F617B664")
    static let txUUID      = CBUUID(string: "AB5E0002-5A21-4F05-BC7D-AF01F617B664")
    static let audioUUID   = CBUUID(string: "AB5E0003-5A21-4F05-BC7D-AF01F617B664")
    static let ctlUUID     = CBUUID(string: "AB5E0004-5A21-4F05-BC7D-AF01F617B664")

    // MARK: 协议常量

    private enum Op: UInt8 {
        case audioStop   = 0x00
        case audioStart  = 0x04
        case startSearch = 0x08
        case getCaps     = 0x0A
        case capsResp    = 0x0B
        case micOpen     = 0x0C
        case micClose    = 0x0D
    }

    // MARK: 状态

    private var central: CBCentralManager!
    private var peripheral: CBPeripheral?
    private var txChar: CBCharacteristic?
    private var streamId: UInt8 = 0
    private var isStreaming = false
    private let deviceName: String

    /// 一段完整语音结束时回调（PCM 16-bit LE, 16 kHz 单声道）
    var onUtterance: ((Data) -> Void)?
    /// 逐帧音频回调（流式模式下，PCM 16-bit LE, 16kHz 单声道）
    var onPCMChunk: ((Data) -> Void)?
    /// 推流结束回调（流式模式下，收到 AUDIO_STOP 时触发）
    var onStreamEnd: (() -> Void)?
    /// 状态变化回调，用于 UI 提示
    var onState: ((String) -> Void)?

    private var decoder = ADPCMDecoder()
    private var pcmBuffer = Data()
    private var frameCount = 0
    /// 断开后持续重试连接，直到遥控器语音服务就绪
    private var reconnectTimer: DispatchSourceTimer?

    init(deviceName: String) {
        self.deviceName = deviceName
        super.init()
        central = CBCentralManager(delegate: self, queue: .main)
    }

    // MARK: 对外接口

    /// 主动开麦（on-request 模式）。若遥控器已在推流则不重复发指令，
    /// 否则会触发第二次 AUDIO_START、把已录到的帧清空。
    func openMic() {
        guard let p = peripheral, let tx = txChar else {
            Log.warn("ATVV 未就绪，无法开麦")
            return
        }
        if isStreaming {
            // 遥控器 hold-to-talk 已经自己开流了，什么都不用做
            Log.debug("ATVV 已在推流，跳过 MIC_OPEN")
            return
        }
        resetStream()
        // 实测该固件吃 2 字节变体
        let cmd = Data([Op.micOpen.rawValue, 0x02])
        p.writeValue(cmd, for: tx, type: .withoutResponse)
        Log.debug("ATVV → MIC_OPEN")
    }

    /// 关麦。遥控器松开按键时会自己发 AUDIO_STOP 并触发识别，
    /// 这里只在它没自动停时兜底。
    func closeMic() {
        guard let p = peripheral, let tx = txChar else { return }
        guard isStreaming else {
            // 已经被 AUDIO_STOP 收尾了，不用再发指令也不用再 flush
            Log.debug("ATVV 已停止推流，无需 MIC_CLOSE")
            return
        }
        let cmd = Data([Op.micClose.rawValue, streamId])
        p.writeValue(cmd, for: tx, type: .withoutResponse)
        Log.debug("ATVV → MIC_CLOSE (已收 \(frameCount) 帧)")
        flush()
    }

    var isReady: Bool { peripheral != nil && txChar != nil }

    // MARK: 内部

    private func resetStream() {
        decoder = ADPCMDecoder()
        pcmBuffer.removeAll(keepingCapacity: true)
        frameCount = 0
    }

    private func flush() {
        // AUDIO_STOP 和 closeMic 都可能触发，去重
        guard isStreaming || !pcmBuffer.isEmpty else { return }
        isStreaming = false

        // 流式模式：发完已收的余量，然后通知流结束
        if onPCMChunk != nil {
            if !pcmBuffer.isEmpty {
                onPCMChunk?(pcmBuffer)
                pcmBuffer.removeAll(keepingCapacity: true)
            }
            onStreamEnd?()
            return
        }

        guard !pcmBuffer.isEmpty else {
            Log.debug("ATVV 无音频数据")
            return
        }
        let pcm = pcmBuffer
        let secs = Double(pcm.count / 2) / 16000.0
        Log.info("语音采集完成：\(frameCount) 帧 / \(String(format: "%.2f", secs))s")
        resetStream()
        onUtterance?(pcm)
    }

    private func connect(_ p: CBPeripheral) {
        peripheral = p
        p.delegate = self
        central.connect(p, options: nil)
    }

    /// 安排周期性重连。遥控器唤醒时 HID 先起来、GATT 语音服务后就绪，
    /// 单次重试经常扑空，所以一直试到语音服务真的连上为止。
    private func scheduleReconnect(after ms: Int = 2000) {
        guard reconnectTimer == nil else { return }
        let t = DispatchSource.makeTimerSource(queue: .main)
        t.schedule(deadline: .now() + .milliseconds(ms), repeating: .milliseconds(2000))
        t.setEventHandler { [weak self] in
            guard let self else { return }
            // 语音服务已就绪，停止重试
            if self.txChar != nil { self.stopReconnect(); return }
            // 正在连接中（peripheral 已设但 txChar 还没拿到），等它走完
            if self.peripheral != nil { return }
            guard self.central.state == .poweredOn else { return }
            self.findConnected()
        }
        t.resume()
        reconnectTimer = t
    }

    private func stopReconnect() {
        reconnectTimer?.cancel()
        reconnectTimer = nil
    }

    /// 找已连接的遥控器（已连接设备不广播，不能用 scanForPeripherals）
    private func findConnected() {
        let known = central.retrieveConnectedPeripherals(withServices: [ATVVClient.serviceUUID])
        if let p = known.first(where: { $0.name == deviceName }) ?? known.first {
            Log.debug("ATVV 找到设备: \(p.name ?? "?")")
            connect(p)
            return
        }
        // 服务过滤拿不到时，退回按名字找
        let all = central.retrieveConnectedPeripherals(withServices: [
            CBUUID(string: "180F"), CBUUID(string: "180A"), ATVVClient.serviceUUID
        ])
        if let p = all.first(where: { $0.name == deviceName }) {
            Log.debug("ATVV 按名字找到设备")
            connect(p)
        } else {
            // 重试时会频繁走到这里，用 debug 避免刷日志
            Log.debug("ATVV 暂未找到已连接的「\(deviceName)」，稍后重试")
            onState?("语音：设备未连接")
        }
    }
}

// MARK: - CBCentralManagerDelegate

extension ATVVClient: CBCentralManagerDelegate {
    func centralManagerDidUpdateState(_ c: CBCentralManager) {
        switch c.state {
        case .poweredOn:
            Log.debug("ATVV 蓝牙就绪")
            findConnected()
        case .unauthorized:
            Log.error("蓝牙权限被拒绝。语音功能需要在 系统设置 › 隐私与安全性 › 蓝牙 中授权。")
            onState?("语音：缺蓝牙权限")
        case .poweredOff:
            onState?("语音：蓝牙已关闭")
        default:
            break
        }
    }

    func centralManager(_ c: CBCentralManager, didConnect p: CBPeripheral) {
        Log.debug("ATVV 已连接，发现服务…")
        p.discoverServices([ATVVClient.serviceUUID])
    }

    func centralManager(_ c: CBCentralManager, didFailToConnect p: CBPeripheral, error: Error?) {
        Log.warn("ATVV 连接失败: \(error?.localizedDescription ?? "?")")
        peripheral = nil
        scheduleReconnect(after: 2000)
    }

    func centralManager(_ c: CBCentralManager, didDisconnectPeripheral p: CBPeripheral, error: Error?) {
        Log.info("ATVV 连接断开，将持续重试直到遥控器就绪")
        txChar = nil
        peripheral = nil
        onState?("语音：已断开")
        scheduleReconnect(after: 2000)
    }
}

// MARK: - CBPeripheralDelegate

extension ATVVClient: CBPeripheralDelegate {
    func peripheral(_ p: CBPeripheral, didDiscoverServices error: Error?) {
        guard let svc = p.services?.first(where: { $0.uuid == ATVVClient.serviceUUID }) else {
            Log.warn("ATVV 服务不存在，该遥控器可能不支持语音")
            return
        }
        p.discoverCharacteristics([ATVVClient.txUUID, ATVVClient.audioUUID, ATVVClient.ctlUUID],
                                  for: svc)
    }

    func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor svc: CBService, error: Error?) {
        for ch in svc.characteristics ?? [] {
            switch ch.uuid {
            case ATVVClient.txUUID:
                txChar = ch
            case ATVVClient.audioUUID, ATVVClient.ctlUUID:
                p.setNotifyValue(true, for: ch)
            default:
                break
            }
        }
        guard let tx = txChar else { return }
        // 语音服务就绪，停掉重连重试
        stopReconnect()
        // 握手：拿能力
        let caps = Data([Op.getCaps.rawValue, 0x00, 0x01, 0x00, 0x03])
        p.writeValue(caps, for: tx, type: .withoutResponse)
        Log.info("语音通道已就绪（ATVV）")
        onState?("语音：就绪")
    }

    func peripheral(_ p: CBPeripheral, didUpdateValueFor ch: CBCharacteristic, error: Error?) {
        guard let data = ch.value, !data.isEmpty else { return }

        if ch.uuid == ATVVClient.audioUUID {
            guard isStreaming else { return }
            frameCount += 1
            // 整包都是 ADPCM 数据，无帧头；解码器状态跨包连续
            let pcm = decoder.decode(data)
            // 流式模式：每帧直接转发；否则攒到结束再给
            if let onChunk = onPCMChunk {
                onChunk(pcm)
            } else {
                pcmBuffer.append(pcm)
            }
            return
        }

        guard ch.uuid == ATVVClient.ctlUUID else { return }
        switch data[0] {
        case Op.capsResp.rawValue:
            Log.debug("ATVV CAPS_RESP: \(data.map { String(format: "%02x", $0) }.joined(separator: " "))")
        case Op.audioStart.rawValue:
            let reason = data.count > 1 ? data[1] : 0
            // reason 0x03 = 按住语音键（遥控器自发），其余 = 我们发 MIC_OPEN 触发
            // 已在推流时不要重置，否则会丢掉开头的音频
            if isStreaming {
                Log.debug("ATVV AUDIO_START 重复到达，忽略（保留已录 \(frameCount) 帧）")
                return
            }
            streamId = data.count > 3 ? data[3] : 0
            resetStream()
            isStreaming = true
            Log.debug("ATVV AUDIO_START (reason=\(reason == 0x03 ? "按住语音键" : "MIC_OPEN") stream=\(streamId))")
            onState?("语音：录音中…")
        case Op.audioStop.rawValue:
            Log.debug("ATVV AUDIO_STOP")
            flush()
        case Op.startSearch.rawValue:
            Log.debug("ATVV START_SEARCH")
        default:
            break
        }
    }
}

// MARK: - IMA/DVI ADPCM 解码器

/// 状态跨包连续的 IMA ADPCM 解码器。
/// 注意 nibble 顺序：这个固件是**高 4 位先**。
struct ADPCMDecoder {
    private static let stepTable: [Int32] = [
        7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37,
        41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173,
        190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658,
        724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066,
        2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894,
        6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289,
        16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
    ]
    private static let indexTable: [Int32] = [
        -1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8
    ]

    private var pred: Int32 = 0
    private var index: Int32 = 0

    private mutating func nibble(_ code: UInt8) -> Int16 {
        let step = ADPCMDecoder.stepTable[Int(index)]
        var diff = step >> 3
        if code & 1 != 0 { diff += step >> 2 }
        if code & 2 != 0 { diff += step >> 1 }
        if code & 4 != 0 { diff += step }
        if code & 8 != 0 { diff = -diff }
        pred = max(-32768, min(32767, pred + diff))
        index = max(0, min(88, index + ADPCMDecoder.indexTable[Int(code)]))
        return Int16(pred)
    }

    /// 解一包 ADPCM → PCM 16-bit LE
    mutating func decode(_ data: Data) -> Data {
        var out = Data(capacity: data.count * 4)
        for byte in data {
            // 高 4 位先（实测）
            for code in [(byte >> 4) & 0x0F, byte & 0x0F] {
                let s = nibble(code)
                out.append(UInt8(truncatingIfNeeded: s))
                out.append(UInt8(truncatingIfNeeded: s >> 8))
            }
        }
        return out
    }
}
