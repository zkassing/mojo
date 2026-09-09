import Foundation
import SherpaBridge

/// sherpa-onnx 本地流式识别：免费、离线、跨平台（zipformer 中英双语）。
/// 与 VolcASRClient 同接口（start/feed/finish/cancel + onPartial/onFinal），
/// 引擎全局只加载一次（prewarm），每次会话 reset 流即可复用。
final class SherpaASRClient {
    var onPartial: ((String) -> Void)?
    var onFinal: ((String) -> Void)?

    private let queue = DispatchQueue(label: "mojo.sherpa", qos: .userInitiated)
    /// 端点（句尾静音）已确认的文本
    private var committed = ""
    private var active = false

    private static var engine: OpaquePointer?
    private static var engineReady = false
    private static let lock = NSLock()

    /// 后台加载模型（daemon 启动时调一次，避免首次按键等待）
    static func prewarm(modelDir: String) {
        DispatchQueue.global(qos: .userInitiated).async {
            lock.lock()
            defer { lock.unlock() }
            guard engine == nil else { return }
            let t = Date()
            engine = modelDir.withCString { mr_sherpa_create($0, 2) }
            if engine == nil {
                Log.error("sherpa 模型加载失败: \(modelDir)")
            } else {
                engineReady = true
                Log.info(String(format: "sherpa 模型已加载（%.1fs）", Date().timeIntervalSince(t)))
            }
        }
    }

    func start() {
        queue.async { [weak self] in
            guard let self else { return }
            Self.lock.lock()
            let ready = Self.engineReady
            Self.lock.unlock()
            guard ready, let eng = Self.engine else {
                Log.warn("sherpa 模型未就绪，本次忽略")
                return
            }
            committed = ""
            mr_sherpa_reset(eng)
            active = true
        }
    }

    func feed(_ pcm: Data) {
        queue.async { [weak self] in
            guard let self, active, let eng = Self.engine else { return }
            // Int16 LE → Float / 32768
            let count = pcm.count / 2
            guard count > 0 else { return }
            var floats = [Float](repeating: 0, count: count)
            pcm.withUnsafeBytes { raw in
                let p = raw.bindMemory(to: Int16.self)
                for i in 0..<count { floats[i] = Float(p[i]) / 32768.0 }
            }
            floats.withUnsafeBufferPointer { buf in
                mr_sherpa_accept(eng, buf.baseAddress, Int32(count))
            }
            let text = String(cString: mr_sherpa_text(eng))
            if mr_sherpa_is_endpoint(eng) != 0 {
                committed += text
                mr_sherpa_reset(eng)
                emitPartial(committed)
            } else {
                emitPartial(committed + text)
            }
        }
    }

    func finish() {
        queue.async { [weak self] in
            guard let self, active else { return }
            active = false
            guard let eng = Self.engine else { return }
            mr_sherpa_input_finished(eng)
            let full = committed + String(cString: mr_sherpa_text(eng))
            committed = ""
            let cb = onFinal
            DispatchQueue.main.async { cb?(full) }
        }
    }

    func cancel() {
        queue.async { [weak self] in
            self?.active = false
            self?.committed = ""
        }
    }

    private func emitPartial(_ text: String) {
        let cb = onPartial
        DispatchQueue.main.async { cb?(text) }
    }
}
