import Foundation

// MARK: - 配置模型

struct DeviceConfig: Codable {
    var vendorId: Int
    var productId: Int
    var name: String?

    enum CodingKeys: String, CodingKey { case vendorId, productId, name }

    init(vendorId: Int, productId: Int, name: String?) {
        self.vendorId = vendorId; self.productId = productId; self.name = name
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        vendorId = try DeviceConfig.decodeId(c, .vendorId)
        productId = try DeviceConfig.decodeId(c, .productId)
        name = try c.decodeIfPresent(String.self, forKey: .name)
    }

    /// 同时接受数字 12984 和字符串 "0x32b8"
    private static func decodeId(_ c: KeyedDecodingContainer<CodingKeys>,
                                 _ key: CodingKeys) throws -> Int {
        if let n = try? c.decode(Int.self, forKey: key) { return n }
        let s = try c.decode(String.self, forKey: key).trimmingCharacters(in: .whitespaces)
        let body = s.lowercased().hasPrefix("0x") ? String(s.dropFirst(2)) : s
        guard let v = Int(body, radix: 16) else {
            throw DecodingError.dataCorruptedError(forKey: key, in: c,
                debugDescription: "\(key.rawValue) 必须是数字或十六进制字符串（如 \"0x32b8\"）")
        }
        return v
    }
}

struct Options: Codable {
    var swallowOriginal: Bool = true
    var longPressMs: Int = 450
    var doublePressMs: Int = 280
    var verbose: Bool = false
    /// 按键去抖（ms）。遥控器一次物理按压有时会发两次 down，
    /// 设为 0 关闭。配了双击的键不受此影响。
    var debounceMs: Int = 150

    enum CodingKeys: String, CodingKey {
        case swallowOriginal, longPressMs, doublePressMs, verbose, debounceMs
    }

    init() {}

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        swallowOriginal = try c.decodeIfPresent(Bool.self, forKey: .swallowOriginal) ?? true
        longPressMs = try c.decodeIfPresent(Int.self, forKey: .longPressMs) ?? 450
        doublePressMs = try c.decodeIfPresent(Int.self, forKey: .doublePressMs) ?? 280
        verbose = try c.decodeIfPresent(Bool.self, forKey: .verbose) ?? false
        debounceMs = try c.decodeIfPresent(Int.self, forKey: .debounceMs) ?? 150
    }
}

/// 一个动作。用 type 区分。
indirect enum Action {
    /// 模拟键盘按键，如 key="up", mods=["cmd","shift"]
    case key(key: String, mods: [String])
    /// 系统媒体键，如 playpause / volup / next
    case media(key: String)
    /// 执行 shell 命令
    case shell(command: String)
    /// 打开 App（名字或 bundle id）或 URL / 文件路径
    case open(target: String)
    /// 鼠标：移动、点击、滚动
    case mouseMove(dx: Double, dy: Double)
    case mouseClick(button: String, count: Int)
    case mouseScroll(dx: Int, dy: Int)
    /// 顺序执行多个动作
    case sequence([Action])
    /// 遥控器麦克风语音转文字：按住开麦，松开识别并输入到光标处
    case dictate
    /// 吞掉按键，什么都不做
    case none
    /// 放行原始按键
    case passthrough

    var isPassthrough: Bool { if case .passthrough = self { return true }; return false }
    var isNone: Bool { if case .none = self { return true }; return false }
    var isDictate: Bool { if case .dictate = self { return true }; return false }
}

extension Action: Codable {
    enum CodingKeys: String, CodingKey {
        case type, key, mods, command, target, dx, dy, button, count, actions, app, url
    }

    init(from decoder: Decoder) throws {
        // 允许简写：字符串直接当作 key 动作，例如 "up" 或 "cmd+tab"
        if let s = try? decoder.singleValueContainer().decode(String.self) {
            let low = s.lowercased()
            if ["dictate", "voice", "speech", "asr"].contains(low) {
                self = .dictate
            } else {
                self = Action.parseShorthand(s)
            }
            return
        }
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let type = (try c.decodeIfPresent(String.self, forKey: .type) ?? "key").lowercased()
        switch type {
        case "key", "keyboard", "hotkey":
            let key = try c.decodeIfPresent(String.self, forKey: .key) ?? ""
            let mods = try c.decodeIfPresent([String].self, forKey: .mods) ?? []
            // 支持 key 里直接写 "cmd+tab"
            if mods.isEmpty, key.contains("+") {
                self = Action.parseShorthand(key)
            } else {
                self = .key(key: key, mods: mods)
            }
        case "media", "aux", "system":
            self = .media(key: try c.decodeIfPresent(String.self, forKey: .key) ?? "")
        case "shell", "command", "exec":
            self = .shell(command: try c.decodeIfPresent(String.self, forKey: .command) ?? "")
        case "open", "app", "url", "launch":
            let t = try c.decodeIfPresent(String.self, forKey: .target)
                ?? c.decodeIfPresent(String.self, forKey: .app)
                ?? c.decodeIfPresent(String.self, forKey: .url) ?? ""
            self = .open(target: t)
        case "mousemove", "move":
            self = .mouseMove(dx: try c.decodeIfPresent(Double.self, forKey: .dx) ?? 0,
                              dy: try c.decodeIfPresent(Double.self, forKey: .dy) ?? 0)
        case "mouseclick", "click":
            self = .mouseClick(button: try c.decodeIfPresent(String.self, forKey: .button) ?? "left",
                               count: try c.decodeIfPresent(Int.self, forKey: .count) ?? 1)
        case "mousescroll", "scroll":
            self = .mouseScroll(dx: try c.decodeIfPresent(Int.self, forKey: .dx) ?? 0,
                                dy: try c.decodeIfPresent(Int.self, forKey: .dy) ?? 0)
        case "sequence", "seq", "multi":
            self = .sequence(try c.decodeIfPresent([Action].self, forKey: .actions) ?? [])
        case "none", "nothing", "ignore":
            self = .none
        case "dictate", "voice", "speech", "asr":
            self = .dictate
        case "passthrough", "pass":
            self = .passthrough
        default:
            throw DecodingError.dataCorruptedError(forKey: .type, in: c,
                debugDescription: "未知的动作类型: \(type)")
        }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .key(let key, let mods):
            try c.encode("key", forKey: .type); try c.encode(key, forKey: .key)
            if !mods.isEmpty { try c.encode(mods, forKey: .mods) }
        case .media(let key):
            try c.encode("media", forKey: .type); try c.encode(key, forKey: .key)
        case .shell(let cmd):
            try c.encode("shell", forKey: .type); try c.encode(cmd, forKey: .command)
        case .open(let t):
            try c.encode("open", forKey: .type); try c.encode(t, forKey: .target)
        case .mouseMove(let dx, let dy):
            try c.encode("mouseMove", forKey: .type); try c.encode(dx, forKey: .dx); try c.encode(dy, forKey: .dy)
        case .mouseClick(let b, let n):
            try c.encode("mouseClick", forKey: .type); try c.encode(b, forKey: .button); try c.encode(n, forKey: .count)
        case .mouseScroll(let dx, let dy):
            try c.encode("mouseScroll", forKey: .type); try c.encode(dx, forKey: .dx); try c.encode(dy, forKey: .dy)
        case .sequence(let a):
            try c.encode("sequence", forKey: .type); try c.encode(a, forKey: .actions)
        case .none:
            try c.encode("none", forKey: .type)
        case .dictate:
            try c.encode("dictate", forKey: .type)
        case .passthrough:
            try c.encode("passthrough", forKey: .type)
        }
    }

    /// "cmd+shift+t" -> .key(key:"t", mods:["cmd","shift"])
    static func parseShorthand(_ s: String) -> Action {
        // 特殊词不是键名：忽略 / 放行
        switch s.lowercased() {
        case "none", "nothing", "ignore": return .none
        case "passthrough", "pass": return .passthrough
        default: break
        }
        let parts = s.split(separator: "+").map { $0.trimmingCharacters(in: .whitespaces) }
        guard let last = parts.last else { return .none }
        let mods = parts.dropLast().map { $0.lowercased() }
        return .key(key: last, mods: Array(mods))
    }
}

/// 一个按键的绑定：可分别设置 单击 / 长按 / 双击 / 按住重复
struct Binding: Codable {
    var tap: Action?
    var long: Action?
    var double: Action?
    /// true 表示按住时持续触发 tap 动作（用于方向键等）
    var repeats: Bool?

    enum CodingKeys: String, CodingKey { case tap, long, double, repeats = "repeat" }

    /// 是否需要延迟判定（存在长按或双击）
    var needsDefer: Bool { long != nil || double != nil }
}

extension Binding {
    init(tap: Action) { self.tap = tap; self.long = nil; self.double = nil; self.repeats = nil }
}

struct ProfileMatch: Codable {
    var bundleIds: [String]?
    var appNames: [String]?

    func matches(bundleId: String?, appName: String?) -> Bool {
        if let ids = bundleIds, let b = bundleId,
           ids.contains(where: { $0.caseInsensitiveCompare(b) == .orderedSame }) { return true }
        if let ns = appNames, let n = appName,
           ns.contains(where: { $0.caseInsensitiveCompare(n) == .orderedSame }) { return true }
        return false
    }
}

struct Profile: Codable {
    var name: String
    var match: ProfileMatch?
    /// 逻辑按键名 -> 绑定
    var bindings: [String: Binding]
}

struct VoiceConfig: Codable {
    /// 识别语言，如 zh-CN / en-US
    var locale: String = "zh-CN"
    /// 识别后怎么处理：type / typeEnter / clipboard
    var output: String = "type"
    /// 去掉中文标点（命令行里通常是干扰）
    var stripPunctuation: Bool = true
    /// 识别引擎："volc" / "sherpa"。
    /// volc = 火山引擎流式大模型，边说边传，中英混说最强；
    /// apple = 系统自带 SFSpeechRecognizer，没网时的备胎（已下线）；
    /// sherpa = sherpa-onnx 本地流式（免费、离线、跨平台）。
    var engine: String = "sherpa"
    /// sherpa 模型目录（留空 = ~/.config/mojo/models/<默认双语流式模型>）
    var sherpaModelDir: String = ""
    /// 火山引擎 AppID
    var volcAppId: String = ""
    /// 火山引擎 Access Token
    var volcAccessToken: String = ""
    /// 资源 ID：小时版 volc.bigasr.sauc.duration，并发版 volc.bigasr.sauc.concurrent
    var volcResourceId: String = "volc.bigasr.sauc.duration"
    /// 边说边出字：把流式中间结果实时写进输入框（仅 volc 引擎支持）
    var liveTyping: Bool = true
    /// 纠正技术术语谐音误识（main→闷、diff→地府）
    var fixTerms: Bool = true

    enum CodingKeys: String, CodingKey {
        case locale, output, stripPunctuation
        case engine
        case volcAppId, volcAccessToken, volcResourceId, liveTyping, fixTerms
        case sherpaModelDir
    }

    init() {}

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        locale = try c.decodeIfPresent(String.self, forKey: .locale) ?? "zh-CN"
        output = try c.decodeIfPresent(String.self, forKey: .output) ?? "type"
        stripPunctuation = try c.decodeIfPresent(Bool.self, forKey: .stripPunctuation) ?? true
        engine = try c.decodeIfPresent(String.self, forKey: .engine) ?? "sherpa"
        volcAppId = try c.decodeIfPresent(String.self, forKey: .volcAppId) ?? ""
        volcAccessToken = try c.decodeIfPresent(String.self, forKey: .volcAccessToken) ?? ""
        volcResourceId = try c.decodeIfPresent(String.self, forKey: .volcResourceId)
            ?? "volc.bigasr.sauc.duration"
        liveTyping = try c.decodeIfPresent(Bool.self, forKey: .liveTyping) ?? true
        fixTerms = try c.decodeIfPresent(Bool.self, forKey: .fixTerms) ?? true
        sherpaModelDir = try c.decodeIfPresent(String.self, forKey: .sherpaModelDir) ?? ""
    }

    var usesVolc: Bool {
        engine.lowercased() == "volc" && !volcAppId.isEmpty && !volcAccessToken.isEmpty
    }

    var usesSherpa: Bool {
        engine.lowercased() == "sherpa"
    }

    /// sherpa 模型目录（解析默认值）
    var sherpaDir: String {
        if !sherpaModelDir.isEmpty { return sherpaModelDir }
        return (NSHomeDirectory() as NSString)
            .appendingPathComponent(".config/mojo/models/sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05")
    }
}

struct Config: Codable {
    var device: DeviceConfig
    var options: Options
    var voice: VoiceConfig
    /// 逻辑按键名 -> 原始信号列表，如 "up": ["kc:126"], "volup": ["aux:0"]
    var buttons: [String: [String]]
    var profiles: [Profile]

    enum CodingKeys: String, CodingKey { case device, options, voice, buttons, profiles }

    init(device: DeviceConfig, options: Options = Options(),
         voice: VoiceConfig = VoiceConfig(),
         buttons: [String: [String]] = [:], profiles: [Profile] = []) {
        self.device = device; self.options = options; self.voice = voice
        self.buttons = buttons; self.profiles = profiles
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        device = try c.decode(DeviceConfig.self, forKey: .device)
        options = try c.decodeIfPresent(Options.self, forKey: .options) ?? Options()
        voice = try c.decodeIfPresent(VoiceConfig.self, forKey: .voice) ?? VoiceConfig()
        buttons = try c.decodeIfPresent([String: [String]].self, forKey: .buttons) ?? [:]
        profiles = try c.decodeIfPresent([Profile].self, forKey: .profiles) ?? []
    }

    /// 原始信号 -> 逻辑按键名（反查表）
    func rawToButton() -> [String: String] {
        var m = [String: String]()
        for (name, raws) in buttons { for r in raws { m[r.lowercased()] = name } }
        return m
    }

    /// 选出匹配当前前台 App 的 profile（有 match 的优先，其次名为 default 的）
    func resolveProfile(bundleId: String?, appName: String?) -> Profile? {
        for p in profiles where p.match?.matches(bundleId: bundleId, appName: appName) == true {
            return p
        }
        return profiles.first { $0.name.caseInsensitiveCompare("default") == .orderedSame }
            ?? profiles.first { $0.match == nil }
    }
}

// MARK: - 读写

enum ConfigStore {
    static var dir: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".config/mojo", isDirectory: true)
    }
    static var path: URL { dir.appendingPathComponent("config.json") }

    static func load() throws -> Config {
        let data = try Data(contentsOf: path)
        // 允许 // 注释
        let cleaned = stripComments(String(decoding: data, as: UTF8.self))
        return try JSONDecoder().decode(Config.self, from: Data(cleaned.utf8))
    }

    static func save(_ c: Config) throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let enc = JSONEncoder()
        enc.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        try enc.encode(c).write(to: path, options: .atomic)
    }

    static func exists() -> Bool { FileManager.default.fileExists(atPath: path.path) }

    private static func stripComments(_ s: String) -> String {
        var out = ""
        var inString = false, escaped = false
        var i = s.startIndex
        while i < s.endIndex {
            let ch = s[i]
            if inString {
                out.append(ch)
                if escaped { escaped = false }
                else if ch == "\\" { escaped = true }
                else if ch == "\"" { inString = false }
                i = s.index(after: i); continue
            }
            if ch == "\"" { inString = true; out.append(ch); i = s.index(after: i); continue }
            if ch == "/", s.index(after: i) < s.endIndex, s[s.index(after: i)] == "/" {
                while i < s.endIndex, s[i] != "\n" { i = s.index(after: i) }
                continue
            }
            out.append(ch)
            i = s.index(after: i)
        }
        return out
    }
}
