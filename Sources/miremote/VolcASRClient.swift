import Foundation
import Compression

/// 火山引擎「流式语音识别大模型」客户端（WebSocket v3 / sauc bigmodel）。
///
/// 选它的理由：遥控器的麦克风本来就是 15ms 一包连续推流，
/// 边收边往上送，松开按键时结果基本已经算完了。
/// 中英混说是这个模型的强项，正好是本地模型的短板。
///
/// 协议是自定义二进制帧，不是纯 JSON：
///   [4 字节头][4 字节 sequence（可选）][4 字节 payload 长度][payload]
final class VolcASRClient: NSObject {

    // MARK: 配置

    struct Credentials {
        let appId: String
        let accessToken: String
        /// 资源 ID：小时版 `volc.bigasr.sauc.duration`，并发版 `volc.bigasr.sauc.concurrent`
        let resourceId: String
    }

    private let creds: Credentials
    private let endpoint = URL(string: "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel")!

    /// 部分结果（边说边出字），可用于日志或未来做实时预览
    var onPartial: ((String) -> Void)?
    /// 最终结果
    var onFinal: ((String) -> Void)?

    // MARK: 协议常量

    private enum MsgType: UInt8 {
        case fullClientRequest = 0b0001
        case audioOnlyRequest  = 0b0010
        case fullServerResponse = 0b1001
        case serverAck         = 0b1011
        case errorResponse     = 0b1111
    }

    private struct Flags {
        static let none: UInt8         = 0b0000
        static let positiveSeq: UInt8  = 0b0001
        static let lastNoSeq: UInt8    = 0b0010
        static let negativeSeq: UInt8  = 0b0011
    }

    // MARK: 状态

    private var session: URLSession!
    private var task: URLSessionWebSocketTask?
    private var seq: Int32 = 1
    /// 攒到 100ms 再发一包 —— 火山推荐的粒度，太碎会浪费往返
    private var pending = Data()
    private static let chunkBytes = 3200      // 100ms @ 16kHz 16bit mono
    private var lastText = ""
    private var finished = false
    private var isOpen = false
    /// 连接还没建好时先攒着，建好后补发
    private var preOpenBuffer = Data()

    init(credentials: Credentials) {
        self.creds = credentials
        super.init()
        session = URLSession(configuration: .default, delegate: nil, delegateQueue: nil)
    }

    // MARK: 对外接口

    /// 开一条新的识别会话
    func start() {
        cleanup()
        seq = 1
        lastText = ""
        finished = false
        isOpen = false
        pending.removeAll(keepingCapacity: true)
        preOpenBuffer.removeAll(keepingCapacity: true)

        var req = URLRequest(url: endpoint)
        req.setValue(creds.appId, forHTTPHeaderField: "X-Api-App-Key")
        req.setValue(creds.accessToken, forHTTPHeaderField: "X-Api-Access-Key")
        req.setValue(creds.resourceId, forHTTPHeaderField: "X-Api-Resource-Id")
        req.setValue(UUID().uuidString, forHTTPHeaderField: "X-Api-Connect-Id")

        let t = session.webSocketTask(with: req)
        task = t
        t.resume()
        receiveLoop()

        sendFullClientRequest()
    }

    /// 喂一段 PCM（16-bit LE, 16 kHz, 单声道）
    func feed(_ pcm: Data) {
        guard !finished else { return }
        pending.append(pcm)
        while pending.count >= Self.chunkBytes {
            let chunk = pending.prefix(Self.chunkBytes)
            pending.removeFirst(Self.chunkBytes)
            sendAudio(Data(chunk), isLast: false)
        }
    }

    /// 说完了：把余量发出去并标记最后一包，等服务端给最终结果
    func finish() {
        guard !finished else { return }
        finished = true
        // 余量连同 last 标记一起发；没余量也要发个空包收尾
        sendAudio(pending, isLast: true)
        pending.removeAll(keepingCapacity: true)
    }

    /// 放弃这次识别
    func cancel() {
        finished = true
        cleanup()
    }

    private func cleanup() {
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
    }

    // MARK: 发送

    private func sendFullClientRequest() {
        let request: [String: Any] = [
            "model_name": "bigmodel",
            "enable_itn": true,        // 数字规范化：「一百二」→「120」
            "enable_punc": false,      // 命令行不要标点
            "enable_ddc": false,
            "show_utterances": false,
            "result_type": "full",     // 每次返回累计全文，解析简单
        ]
        let body: [String: Any] = [
            "user": ["uid": "miremote"],
            "audio": [
                "format": "pcm",
                "codec": "raw",
                "rate": 16000,
                "bits": 16,
                "channel": 1,
            ],
            "request": request,
        ]
        guard let json = try? JSONSerialization.data(withJSONObject: body) else {
            Log.warn("火山 ASR 请求体序列化失败")
            return
        }
        var frame = header(type: .fullClientRequest,
                           flags: Flags.positiveSeq,
                           serialization: 0b0001,   // JSON
                           compression: 0b0000)     // 不压缩
        frame.append(be32(UInt32(bitPattern: seq)))
        frame.append(be32(UInt32(json.count)))
        frame.append(json)
        send(frame)
        seq += 1
    }

    private func sendAudio(_ pcm: Data, isLast: Bool) {
        // 最后一包用负数 sequence 表示（协议规定）
        let s = isLast ? -seq : seq
        var frame = header(type: .audioOnlyRequest,
                           flags: isLast ? Flags.negativeSeq : Flags.positiveSeq,
                           serialization: 0b0000,   // raw
                           compression: 0b0000)
        frame.append(be32(UInt32(bitPattern: s)))
        frame.append(be32(UInt32(pcm.count)))
        frame.append(pcm)
        send(frame)
        if !isLast { seq += 1 }
    }

    private func header(type: MsgType, flags: UInt8,
                        serialization: UInt8, compression: UInt8) -> Data {
        Data([
            0x11,                                   // version 1 | header size 1 (=4字节)
            (type.rawValue << 4) | flags,
            (serialization << 4) | compression,
            0x00,                                   // reserved
        ])
    }

    private func be32(_ v: UInt32) -> Data {
        withUnsafeBytes(of: v.bigEndian) { Data($0) }
    }

    private func send(_ data: Data) {
        guard let task else { return }
        // 连接尚未 open 时 URLSessionWebSocketTask 会自己排队，无需额外处理
        task.send(.data(data)) { err in
            if let err {
                Log.warn("火山 ASR 发送失败: \(err.localizedDescription)")
            }
        }
    }

    // MARK: 接收

    private func receiveLoop() {
        guard let task else { return }
        task.receive { [weak self] result in
            guard let self else { return }
            switch result {
            case .failure(let err):
                let ns = err as NSError
                // 正常收尾时的 cancel 不算错误
                if ns.code != NSURLErrorCancelled {
                    Log.warn("火山 ASR 连接错误: \(err.localizedDescription)")
                }
                self.emitFinalIfNeeded()
            case .success(let msg):
                switch msg {
                case .data(let d):  self.handle(d)
                case .string(let s): Log.debug("火山 ASR 文本帧: \(s.prefix(200))")
                @unknown default: break
                }
                self.receiveLoop()
            }
        }
    }

    private func handle(_ data: Data) {
        guard data.count >= 4 else { return }
        let b1 = data[1]
        let type = b1 >> 4
        let flags = b1 & 0x0F
        let compression = data[2] & 0x0F

        var i = 4

        if type == MsgType.errorResponse.rawValue {
            guard data.count >= i + 8 else { return }
            let code = readBE32(data, i); i += 4
            let size = Int(readBE32(data, i)); i += 4
            let msg = payloadString(data, from: i, size: size, gzipped: compression == 1)
            Log.error("火山 ASR 错误 \(code): \(msg ?? "?")")
            emitFinalIfNeeded()
            return
        }

        guard type == MsgType.fullServerResponse.rawValue
                || type == MsgType.serverAck.rawValue else { return }

        // flags 带 sequence 位时，payload 前面多 4 字节序号
        if flags & 0x01 != 0 || flags & 0x02 != 0 {
            guard data.count >= i + 4 else { return }
            i += 4
        }
        guard data.count >= i + 4 else { return }
        let size = Int(readBE32(data, i)); i += 4
        guard size > 0,
              let text = payloadString(data, from: i, size: size, gzipped: compression == 1)
        else { return }

        parseResult(text)

        // flags 第二位 = 服务端最后一包
        let isServerLast = (flags & 0x02) != 0
        if isServerLast {
            emitFinalIfNeeded()
        }
    }

    private func parseResult(_ json: String) {
        guard let d = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: d) as? [String: Any]
        else {
            Log.debug("火山 ASR 无法解析: \(json.prefix(300))")
            return
        }
        // 结构：{ "result": { "text": "…" } }
        guard let result = obj["result"] as? [String: Any],
              let text = result["text"] as? String, !text.isEmpty else { return }

        if text != lastText {
            lastText = text
            onPartial?(text)
        }
    }

    private var didEmitFinal = false
    private func emitFinalIfNeeded() {
        guard !didEmitFinal else { return }
        didEmitFinal = true
        let t = lastText
        cleanup()
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            if t.isEmpty {
                Log.info("火山 ASR 没识别到内容")
            }
            self.onFinal?(t)
        }
    }

    // MARK: 工具

    private func readBE32(_ d: Data, _ off: Int) -> UInt32 {
        guard d.count >= off + 4 else { return 0 }
        return d.withUnsafeBytes { raw in
            let p = raw.baseAddress!.advanced(by: off)
            return UInt32(bigEndian: p.loadUnaligned(as: UInt32.self))
        }
    }

    private func payloadString(_ d: Data, from: Int, size: Int, gzipped: Bool) -> String? {
        let end = min(d.count, from + size)
        guard from < end else { return nil }
        let raw = d.subdata(in: from..<end)
        let bytes = gzipped ? (Self.gunzip(raw) ?? raw) : raw
        return String(data: bytes, encoding: .utf8)
    }

    /// gzip 解压。火山有时会压缩响应体。
    private static func gunzip(_ data: Data) -> Data? {
        // 跳过 10 字节 gzip 头，余下是 raw deflate
        guard data.count > 18, data[0] == 0x1f, data[1] == 0x8b else { return nil }
        var start = 10
        let flg = data[3]
        if flg & 0x08 != 0 {   // FNAME
            while start < data.count, data[start] != 0 { start += 1 }
            start += 1
        }
        guard start < data.count else { return nil }
        let deflated = data.subdata(in: start..<data.count)

        let capacity = max(deflated.count * 8, 64 * 1024)
        var out = Data(count: capacity)
        let written: Int = out.withUnsafeMutableBytes { dst in
            deflated.withUnsafeBytes { src in
                compression_decode_buffer(
                    dst.bindMemory(to: UInt8.self).baseAddress!, capacity,
                    src.bindMemory(to: UInt8.self).baseAddress!, deflated.count,
                    nil, COMPRESSION_ZLIB)
            }
        }
        guard written > 0 else { return nil }
        return out.prefix(written)
    }
}
