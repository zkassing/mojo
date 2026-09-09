import Foundation
import CoreGraphics
import AppKit

/// 流式 ASR 会话协议：火山云端 / sherpa 本地，同一套回调形状
protocol ASRSession: AnyObject {
    var onPartial: ((String) -> Void)? { get set }
    var onFinal: ((String) -> Void)? { get set }
    func start()
    func feed(_ pcm: Data)
    func finish()
    func cancel()
}
extension VolcASRClient: ASRSession {}
extension SherpaASRClient: ASRSession {}

/// 一个来自遥控器的原始信号
struct RawSignal: Hashable, CustomStringConvertible {
    enum Kind { case key, aux, hid }
    let kind: Kind
    let code: Int64
    /// hid 通道用：usage page
    var page: Int64 = 0

    init(kind: Kind, code: Int64, page: Int64 = 0) {
        self.kind = kind; self.code = code; self.page = page
    }

    var id: String {
        switch kind {
        case .key: return "kc:\(code)"
        case .aux: return "aux:\(code)"
        case .hid: return "hid:\(page):\(code)"
        }
    }

    var pretty: String {
        switch kind {
        case .key: return "\(KeyMap.name(forCode: code)) (kc:\(code))"
        case .aux: return "\(MediaKey.name(forCode: Int32(code))) (aux:\(code))"
        case .hid:
            let p = page == 0x07 ? "Keyboard" : (page == 0x0C ? "Consumer" : "page 0x\(String(page, radix: 16))")
            return "\(p) usage 0x\(String(code, radix: 16)) (hid:\(page):\(code))"
        }
    }

    var description: String { id }
}

private let kSenderIDField = CGEventField(rawValue: 87)!
private let kSysDefinedType = CGEventType(rawValue: 14)!

/// 按键状态机：处理 单击 / 长按 / 双击 / 按住重复
private final class KeyState {
    var isDown = false
    var downAt: Date = .distantPast
    /// 上次松开的时间戳，用于去抖（遥控器硬件抖动会一次按压发两次 down）
    var lastUpAt: Date = .distantPast
    var longFired = false
    var pendingTapWork: DispatchWorkItem?
    var longWork: DispatchWorkItem?
    var repeatTimer: DispatchSourceTimer?
    var awaitingSecondTap = false
}

final class RemapEngine {
    private var config: Config
    private let watcher: DeviceWatcher
    private let emitter = Emitter()
    /// 拦住电源键的系统睡眠，把这个键还给映射表
    private let powerGuard = PowerButtonGuard()
    private var rawToButton: [String: String]
    private var tap: CFMachPort?
    private var hidWatcher: HIDWatcher?
    private var atvv: ATVVClient?
    /// 当前正在开麦的按键名（防重入）
    private var dictatingButton: String?
    /// 火山引擎流式 ASR 会话（当前这段录音）
    private var asrSession: ASRSession?
    /// 流式识别的实时输入器
    private lazy var liveTyper = LiveTyper(emitter: emitter)
    private var states = [String: KeyState]()
    /// 学习模式回调；返回 true 表示吞掉事件
    var onLearn: ((RawSignal) -> Bool)?
    /// 纯监听模式（watch 子命令）
    var watchOnly = false

    init(config: Config) {
        self.config = config
        self.rawToButton = config.rawToButton()
        self.watcher = DeviceWatcher(vendorId: config.device.vendorId,
                                     productId: config.device.productId)
        Log.verbose = config.options.verbose
    }

    // MARK: - 生命周期

    func start() throws {
        // 只要有任何方案给 power 绑了真实动作，就拦住系统睡眠
        if !watchOnly, configBindsPowerKey() {
            powerGuard.start()
        }

        watcher.start { ids in
            if ids.isEmpty {
                Log.info("遥控器已断开")
            } else {
                Log.info("遥控器已连接 (registryID: \(ids.sorted().map(String.init).joined(separator: ", ")))")
            }
        }

        // HID element 直读通道：只处理配置里写成 hid:<page>:<usage> 的信号。
        // 用于那些 macOS 不认识、因而不会产生 CGEvent 的按键
        //（小米遥控器返回键 = Keyboard usage 0xF1）。
        if needsHIDChannel() {
            let hw = HIDWatcher(vendorId: config.device.vendorId,
                                productId: config.device.productId) { [weak self] page, usage, isDown in
                self?.handleHID(page: page, usage: usage, isDown: isDown)
            }
            hidWatcher = hw
            hw.start()
        }

        // 语音通道：配置里有 dictate 动作时才启用
        if usesDictate() {
            setupVoice()
        }

        let mask: CGEventMask =
            (1 << CGEventType.keyDown.rawValue) |
            (1 << CGEventType.keyUp.rawValue) |
            (1 << kSysDefinedType.rawValue)

        let ctx = Unmanaged.passUnretained(self).toOpaque()
        guard let t = CGEvent.tapCreate(tap: .cghidEventTap,
                                        place: .headInsertEventTap,
                                        options: .defaultTap,
                                        eventsOfInterest: mask,
                                        callback: { proxy, type, event, ctx in
            guard let ctx else { return Unmanaged.passUnretained(event) }
            let me = Unmanaged<RemapEngine>.fromOpaque(ctx).takeUnretainedValue()
            return me.handle(proxy: proxy, type: type, event: event)
        }, userInfo: ctx) else {
            throw MojoError.tapCreationFailed
        }
        tap = t
        let src = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, t, 0)
        CFRunLoopAddSource(CFRunLoopGetMain(), src, .commonModes)
        CGEvent.tapEnable(tap: t, enable: true)
        Log.info("事件拦截已启动")
    }

    func reload(_ newConfig: Config) {
        let oldVoice = config.voice
        config = newConfig
        rawToButton = newConfig.rawToButton()
        Log.verbose = newConfig.options.verbose
        cancelAll()
        // 电源键绑定变了，守卫跟着开/关
        if !watchOnly {
            if configBindsPowerKey() {
                powerGuard.start()
            } else {
                powerGuard.stop()
            }
        }
        // 新配置可能刚加入 hid: 信号，此时需要补开通道
        if hidWatcher == nil, needsHIDChannel() {
            let hw = HIDWatcher(vendorId: newConfig.device.vendorId,
                                productId: newConfig.device.productId) { [weak self] page, usage, isDown in
                self?.handleHID(page: page, usage: usage, isDown: isDown)
            }
            hidWatcher = hw
            hw.start()
        }
        // 语音：刚启用则建链路；locale/output 改了则只重建识别器
        // （不重建 ATVV，避免白白断开重连蓝牙）
        if usesDictate() {
            if atvv == nil {
                setupVoice()
            } else if newConfig.voice.usesVolc || newConfig.voice.usesSherpa {
                // 流式引擎：凭证/模型在每次会话时读，无需重建
            }
        }
        Log.info("配置已重新加载（\(newConfig.buttons.count) 个按键，\(newConfig.profiles.count) 个方案）")
    }

    /// 配置里是否用到了 hid: 前缀的信号（或处于学习/监听模式）
    private func needsHIDChannel() -> Bool {
        if onLearn != nil || watchOnly { return true }
        return rawToButton.keys.contains { $0.hasPrefix("hid:") }
    }

    /// 配置里是否把 power 键绑了真实动作。
    /// 绑了就需要拦系统睡眠；全部是 none/passthrough 则不用扰动系统。
    private func configBindsPowerKey() -> Bool {
        for p in config.profiles {
            guard let b = p.bindings["power"] else { continue }
            for a in [b.tap, b.long, b.double].compactMap({ $0 }) {
                if !a.isPassthrough && !a.isNone { return true }
            }
        }
        return false
    }

    /// 配置里是否有 dictate 动作
    private func usesDictate() -> Bool {
        for p in config.profiles {
            for (_, b) in p.bindings {
                for a in [b.tap, b.long, b.double].compactMap({ $0 }) {
                    if Self.containsDictate(a) { return true }
                }
            }
        }
        return false
    }

    private static func containsDictate(_ a: Action) -> Bool {
        if a.isDictate { return true }
        if case .sequence(let list) = a { return list.contains(where: containsDictate) }
        return false
    }

    private func expandPath(_ p: String) -> String {
        p.hasPrefix("~")
            ? NSString(string: p).expandingTildeInPath
            : p
    }
    /// 初始化语音链路：ATVV（遥控器麦克风）+ 识别引擎
    private func setupVoice() {
        guard config.voice.usesVolc || config.voice.usesSherpa else {
            Log.warn("未选择识别引擎（在面板的语音识别页选 火山 或 sherpa）")
            return
        }
        let client = ATVVClient(deviceName: config.device.name ?? "小米蓝牙语音遥控器")
        client.onState = { s in Log.debug("ATVV 状态: \(s)") }
        // 流式：每帧音频直接喂给当前 ASR 会话，不等录完
        client.onPCMChunk = { [weak self] pcm in
            self?.asrSession?.feed(pcm)
        }
        client.onStreamEnd = { [weak self] in
            self?.asrSession?.finish()
        }
        atvv = client
        if config.voice.usesSherpa {
            Log.info("语音引擎: sherpa-onnx 本地流式（免费离线）")
            SherpaASRClient.prewarm(modelDir: config.voice.sherpaDir)
        } else {
            Log.info("语音引擎: 火山引擎流式大模型")
        }
    }

    /// HID element 通道的事件入口
    private func handleHID(page: UInt32, usage: UInt32, isDown: Bool) {
        let signal = RawSignal(kind: .hid, code: Int64(usage), page: Int64(page))

        if let learn = onLearn {
            if isDown { _ = learn(signal) }
            return
        }

        if watchOnly {
            // 只报 CGEvent 看不到的信号，否则每个普通按键会双重打印
            if isNativelyHandled(page: page, usage: usage) { return }
            Log.info("\(isDown ? "↓" : "↑") \(signal.pretty)  →  \(rawToButton[signal.id] ?? "(未映射)")  [HID 直读]")
            return
        }

        guard let buttonName = rawToButton[signal.id] else { return }
        let (front, frontName) = frontmostApp()
        guard let profile = config.resolveProfile(bundleId: front, appName: frontName),
              let binding = profile.bindings[buttonName] else { return }
        if binding.long == nil, binding.double == nil, let t = binding.tap, t.isPassthrough { return }

        process(button: buttonName, binding: binding, isDown: isDown,
                profileName: profile.name, signal: signal)
    }

    /// 按住语音键 → 遥控器开麦；松开 → 关麦并识别
    private func handleDictate(button: String, isDown: Bool, profileName: String) {
        guard let client = atvv else {
            Log.warn("语音通道未初始化")
            return
        }
        if isDown {
            guard dictatingButton == nil else { return }   // 防重入
            guard client.isReady else {
                Log.warn("语音通道未就绪（遥控器可能休眠，先按任意键唤醒）")
                return
            }
            dictatingButton = button
            Log.info("[\(profileName)] \(button) 按住 → 开始录音")
            if config.voice.usesVolc || config.voice.usesSherpa { startASRSession() }
            client.openMic()
        } else {
            guard dictatingButton == button else { return }
            dictatingButton = nil
            Log.info("[\(profileName)] \(button) 松开 → 识别中")
            // 流式模式下由 ATVV 的 onStreamEnd 触发 finish，仍然要关麦
            client.closeMic()
        }
    }

    /// 启动火山引擎一次识别会话
    private func startASRSession() {
        asrSession?.cancel()
        let client: ASRSession
        if config.voice.usesSherpa {
            client = SherpaASRClient()
        } else {
            client = VolcASRClient(credentials: VolcASRClient.Credentials(
                appId: config.voice.volcAppId,
                accessToken: config.voice.volcAccessToken,
                resourceId: config.voice.volcResourceId))
        }
        let strip = config.voice.stripPunctuation
        let fix = config.voice.fixTerms
        let out = config.voice.output.lowercased()
        let live = config.voice.liveTyping && out != "clipboard"

        if live { liveTyper.reset() }

        // 边说边出字：中间结果直接进输入框
        client.onPartial = { [weak self] partial in
            guard let self else { return }
            Log.debug("中间结果: \(partial.prefix(80))")
            guard live else { return }
            var t = strip ? Self.cleanChinesePunct(partial) : partial
            if fix { t = TermFixer.fix(t) }
            self.liveTyper.update(to: t)
        }

        client.onFinal = { [weak self] text in
            guard let self else { return }
            var t = strip ? Self.cleanChinesePunct(text) : text
            if fix { t = TermFixer.fix(t) }

            if live {
                guard !t.isEmpty else {
                    self.liveTyper.clear()      // 什么都没识别到，把中间结果撤回去
                    Log.info("没识别到内容")
                    return
                }
                Log.info("识别结果: \(t)")
                self.liveTyper.finalize(t) { [weak self] in
                    if out == "typeenter" || out == "type_enter" || out == "enter" {
                        self?.emitter.sendKey("return", mods: [])
                    }
                }
                return
            }

            // 非实时模式：等最终结果一次性输出
            guard !t.isEmpty else {
                Log.info("没识别到内容")
                return
            }
            Log.info("识别结果: \(t)")
            switch out {
            case "clipboard", "copy":
                Self.copyToClipboard(t)
                Log.info("已复制到剪贴板")
            case "typeenter", "type_enter", "enter":
                self.emitter.typeText(t)
                usleep(80_000)
                self.emitter.sendKey("return", mods: [])
            default:
                self.emitter.typeText(t)
            }
        }
        client.start()
        asrSession = client
    }

    private static func copyToClipboard(_ s: String) {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/pbcopy")
        let pipe = Pipe()
        p.standardInput = pipe
        try? p.run()
        pipe.fileHandleForWriting.write(Data(s.utf8))
        pipe.fileHandleForWriting.closeFile()
        p.waitUntilExit()
    }

    private static func cleanChinesePunct(_ s: String) -> String {
        let drop: Set<Character> = ["。", "，", "、", "；", "！", "？", "：",
                                    "“", "”", "‘", "’", "（", "）", "《", "》"]
        return String(s.filter { !drop.contains($0) })
            .trimmingCharacters(in: .whitespaces)
    }

    /// 这个 HID usage 是不是 macOS 本身就会翻译成 CGEvent 的（避免 watch 双重打印）
    private func isNativelyHandled(page: UInt32, usage: UInt32) -> Bool {
        // Keyboard page 的标准范围 0x04..0xA4 及修饰键 0xE0..0xE7 都会变成 keyDown
        if page == 0x07 { return (0x04...0xA4).contains(usage) || (0xE0...0xE7).contains(usage) }
        // Consumer page 的常见媒体键会变成 NX_SYSDEFINED
        if page == 0x0C { return true }
        return false
    }

    private func cancelAll() {
        for (_, s) in states {
            s.pendingTapWork?.cancel(); s.longWork?.cancel(); s.repeatTimer?.cancel()
        }
        states.removeAll()
    }

    // MARK: - 事件处理

    private func handle(proxy: CGEventTapProxy, type: CGEventType, event: CGEvent) -> Unmanaged<CGEvent>? {
        // tap 被系统禁用（超时/用户输入过快）后自动重启
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            if let t = tap { CGEvent.tapEnable(tap: t, enable: true) }
            Log.warn("事件拦截被系统暂停，已自动恢复")
            return Unmanaged.passUnretained(event)
        }

        let sender = UInt64(bitPattern: event.getIntegerValueField(kSenderIDField))
        guard watcher.contains(sender) else { return Unmanaged.passUnretained(event) }

        guard let (signal, isDown) = decode(type: type, event: event) else {
            return Unmanaged.passUnretained(event)
        }

        // 学习模式
        if let learn = onLearn {
            if isDown {
                return learn(signal) ? nil : Unmanaged.passUnretained(event)
            }
            return nil
        }

        if watchOnly {
            Log.info("\(isDown ? "↓" : "↑") \(signal.pretty)  →  \(rawToButton[signal.id] ?? "(未映射)")")
            return Unmanaged.passUnretained(event)
        }

        guard let buttonName = rawToButton[signal.id] else {
            Log.debug("未映射信号 \(signal.pretty)，放行")
            return Unmanaged.passUnretained(event)
        }

        let (front, frontName) = frontmostApp()
        guard let profile = config.resolveProfile(bundleId: front, appName: frontName),
              let binding = profile.bindings[buttonName] else {
            if isDown { Log.info("[\(config.resolveProfile(bundleId: front, appName: frontName)?.name ?? "?")] \(buttonName) 无绑定，放行原键") }
            return Unmanaged.passUnretained(event)
        }

        // 显式 passthrough：不做任何映射，原样放行（不受 swallowOriginal 影响）
        if binding.long == nil, binding.double == nil,
           let t = binding.tap, t.isPassthrough {
            if isDown { Log.info("[\(profile.name)] \(buttonName) → 放行原键（passthrough）") }
            return Unmanaged.passUnretained(event)
        }

        process(button: buttonName, binding: binding, isDown: isDown,
                profileName: profile.name, signal: signal)

        return config.options.swallowOriginal ? nil : Unmanaged.passUnretained(event)
    }

    /// 把 CGEvent 解析成 RawSignal
    private func decode(type: CGEventType, event: CGEvent) -> (RawSignal, Bool)? {
        switch type {
        case .keyDown:
            return (RawSignal(kind: .key, code: event.getIntegerValueField(.keyboardEventKeycode)), true)
        case .keyUp:
            return (RawSignal(kind: .key, code: event.getIntegerValueField(.keyboardEventKeycode)), false)
        default:
            guard type == kSysDefinedType, let ns = NSEvent(cgEvent: event), ns.subtype.rawValue == 8 else {
                return nil
            }
            let data1 = ns.data1
            let keyCode = Int64((data1 & 0xFFFF0000) >> 16)
            let keyState = (data1 & 0xFF00) >> 8
            return (RawSignal(kind: .aux, code: keyCode), keyState == 0xA)
        }
    }

    // MARK: - 状态机

    private func process(button: String, binding: Binding, isDown: Bool,
                         profileName: String, signal: RawSignal) {
        // dictate 是「按住-松开」语义，不进普通的单击/长按状态机
        if let t = binding.tap, t.isDictate, binding.long == nil, binding.double == nil {
            handleDictate(button: button, isDown: isDown, profileName: profileName)
            return
        }

        let st = states[button] ?? { let s = KeyState(); states[button] = s; return s }()

        if isDown {
            if st.isDown { // 系统自动重复
                return
            }
            // 去抖：遥控器一次物理按压有时会发 down→up→down，
            // 导致单击被当成两次。距上次松开太近的按下直接丢弃。
            // 只对没配双击的键生效 —— 双击本身就靠两次快速按压识别，
            // 去抖会把第二下吃掉。
            if binding.double == nil, config.options.debounceMs > 0 {
                let sinceUp = Date().timeIntervalSince(st.lastUpAt) * 1000
                if sinceUp < Double(config.options.debounceMs) {
                    Log.debug("\(button) 去抖丢弃（距上次松开 \(Int(sinceUp))ms）")
                    return
                }
            }
            st.isDown = true
            st.downAt = Date()
            st.longFired = false

            // 长按
            if let longAction = binding.long {
                let w = DispatchWorkItem { [weak self] in
                    guard let self, st.isDown else { return }
                    st.longFired = true
                    st.pendingTapWork?.cancel(); st.pendingTapWork = nil
                    st.awaitingSecondTap = false
                    Log.info("[\(profileName)] \(button) 长按 → \(self.describe(longAction))")
                    self.emitter.perform(longAction)
                }
                st.longWork = w
                DispatchQueue.main.asyncAfter(
                    deadline: .now() + .milliseconds(config.options.longPressMs), execute: w)
            }

            // 按住重复
            if binding.repeats == true, let tapAction = binding.tap, binding.long == nil, binding.double == nil {
                emitter.perform(tapAction)
                Log.info("[\(profileName)] \(button) → \(describe(tapAction))")
                let timer = DispatchSource.makeTimerSource(queue: .main)
                timer.schedule(deadline: .now() + .milliseconds(400), repeating: .milliseconds(90))
                timer.setEventHandler { [weak self] in
                    guard let self, st.isDown else { return }
                    self.emitter.perform(tapAction)
                }
                st.repeatTimer = timer
                timer.resume()
            }
            return
        }

        // key up
        st.isDown = false
        st.lastUpAt = Date()
        st.longWork?.cancel(); st.longWork = nil
        st.repeatTimer?.cancel(); st.repeatTimer = nil

        if st.longFired { st.longFired = false; return }
        if binding.repeats == true && binding.long == nil && binding.double == nil { return }

        // 双击
        if let doubleAction = binding.double {
            if st.awaitingSecondTap {
                st.awaitingSecondTap = false
                st.pendingTapWork?.cancel(); st.pendingTapWork = nil
                Log.info("[\(profileName)] \(button) 双击 → \(describe(doubleAction))")
                emitter.perform(doubleAction)
                return
            }
            st.awaitingSecondTap = true
            let w = DispatchWorkItem { [weak self] in
                guard let self else { return }
                st.awaitingSecondTap = false
                st.pendingTapWork = nil
                if let tapAction = binding.tap {
                    Log.info("[\(profileName)] \(button) → \(self.describe(tapAction))")
                    self.emitter.perform(tapAction)
                }
            }
            st.pendingTapWork = w
            DispatchQueue.main.asyncAfter(
                deadline: .now() + .milliseconds(config.options.doublePressMs), execute: w)
            return
        }

        if let tapAction = binding.tap {
            Log.info("[\(profileName)] \(button) → \(describe(tapAction))")
            emitter.perform(tapAction)
        }
    }

    // MARK: - 辅助

    private func frontmostApp() -> (String?, String?) {
        guard let app = NSWorkspace.shared.frontmostApplication else { return (nil, nil) }
        return (app.bundleIdentifier, app.localizedName)
    }

    private func describe(_ a: Action) -> String {
        switch a {
        case .key(let k, let m): return m.isEmpty ? "按键 \(k)" : "按键 \(m.joined(separator: "+"))+\(k)"
        case .media(let k): return "媒体 \(k)"
        case .shell(let c): return "shell `\(c.prefix(50))`"
        case .open(let t): return "打开 \(t)"
        case .mouseMove(let x, let y): return "鼠标移动 (\(x), \(y))"
        case .mouseClick(let b, let n): return "鼠标\(b)键 x\(n)"
        case .mouseScroll(let x, let y): return "滚动 (\(x), \(y))"
        case .sequence(let l): return "序列[" + l.map(describe).joined(separator: ", ") + "]"
        case .dictate: return "语音转文字"
        case .none: return "忽略"
        case .passthrough: return "放行"
        }
    }
}

enum MojoError: LocalizedError {
    case tapCreationFailed
    case noConfig
    case deviceNotFound

    var errorDescription: String? {
        switch self {
        case .tapCreationFailed:
            return """
            无法创建事件拦截（CGEventTap）。
            请到 系统设置 › 隐私与安全性 › 辅助功能 中，把 mojo 加入并勾选，然后重试。
            若已勾选过旧版本，先移除再重新添加。
            """
        case .noConfig:
            return "找不到配置文件 \(ConfigStore.path.path)，请先运行 `mojo init`。"
        case .deviceNotFound:
            return "找不到目标设备，请确认遥控器已通过蓝牙连接。"
        }
    }
}
