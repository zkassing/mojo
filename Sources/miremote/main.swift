import Foundation
import AppKit
import CoreGraphics
import Speech

let VERSION = "1.0.0"

// MARK: - 默认配置

func defaultConfig() -> Config {
    var c = Config(device: DeviceConfig(vendorId: 0x2717, productId: 0x32B8,
                                       name: "小米蓝牙语音遥控器"))
    c.options.swallowOriginal = true
    c.buttons = [:]
    c.profiles = [
        Profile(name: "default", match: nil, bindings: [:])
    ]
    return c
}

// MARK: - 子命令

func cmdInit(force: Bool) throws {
    if ConfigStore.exists() && !force {
        print("配置文件已存在: \(ConfigStore.path.path)")
        print("如需覆盖请加 --force")
        return
    }
    var c = defaultConfig()

    // 尝试自动识别已连接的小米遥控器
    let all = DeviceWatcher.listAll()
    if let mi = all.first(where: { $0.vendor == 0x2717 }) {
        c.device = DeviceConfig(vendorId: mi.vendor, productId: mi.product, name: mi.name)
        print("✅ 已自动识别设备: \(mi.name) (VID 0x\(String(mi.vendor, radix: 16)), PID 0x\(String(mi.product, radix: 16)))")
    } else {
        print("⚠️  未检测到小米遥控器，先用默认 VID/PID 写入配置（0x2717/0x32B8）")
        print("   连接遥控器后可运行 `miremote devices` 核对。")
    }
    try ConfigStore.save(c)
    print("📄 配置已写入: \(ConfigStore.path.path)")
    print("\n下一步：运行 `miremote learn` 逐个记录遥控器按键。")
}

func cmdDevices() {
    let all = DeviceWatcher.listAll()
    print("已连接的 HID 设备：\n")
    print("名称".padded(36) + "VID".padded(9) + "PID".padded(9) + "接口".padded(24) + "registryID")
    print(String(repeating: "─", count: 96))
    for d in all {
        let name = d.name.count > 34 ? String(d.name.prefix(33)) + "…" : d.name
        let ids = d.registryIds.count > 4
            ? d.registryIds.prefix(4).map(String.init).joined(separator: ",") + "… (\(d.registryIds.count) 个)"
            : d.registryIds.map(String.init).joined(separator: ",")
        let mark = d.vendor == 0x2717 ? "  ← 小米" : ""
        print(name.padded(36)
              + "0x\(String(format: "%04x", d.vendor))".padded(9)
              + "0x\(String(format: "%04x", d.product))".padded(9)
              + d.transport.padded(24)
              + ids + mark)
    }
    print("\n提示：把目标设备的 VID/PID 写进 \(ConfigStore.path.path) 的 device 字段。")
}

/// 交互式学习按键
func cmdLearn() throws {
    guard ConfigStore.exists() else { throw MiRemoteError.noConfig }
    var config = try ConfigStore.load()
    let engine = RemapEngine(config: config)

    print("""

    ┌─────────────────────────────────────────────────────────┐
    │  按键学习模式                                            │
    ├─────────────────────────────────────────────────────────┤
    │  • 按下遥控器上的一个键，程序会记录它的原始码             │
    │  • 然后在终端输入这个键的名字（如 up / ok / back）        │
    │  • 输入 done 结束并保存，Ctrl-C 放弃                     │
    └─────────────────────────────────────────────────────────┘

    """)

    var captured: [RawSignal] = []
    let lock = NSLock()

    engine.onLearn = { sig in
        lock.lock(); defer { lock.unlock() }
        if !captured.contains(sig) {
            captured.append(sig)
            print("  ⬅︎ 捕获: \(sig.pretty)")
            fflush(stdout)
        }
        return true // 学习模式吞掉按键，避免误触发
    }
    try engine.start()

    // 在后台线程读取用户输入，主线程跑 RunLoop
    let inputThread = Thread {
        var buttons = config.buttons
        while true {
            lock.lock()
            let pending = captured
            lock.unlock()

            if pending.isEmpty {
                print("\n等待你按下遥控器按键…（已记录 \(buttons.count) 个）")
                fflush(stdout)
                Thread.sleep(forTimeInterval: 0.6)
                lock.lock(); let still = captured.isEmpty; lock.unlock()
                if still { continue }
                continue
            }

            let sigList = pending.map { $0.id }
            print("\n当前捕获的信号: \(pending.map { $0.pretty }.joined(separator: " + "))")
            print("给它起个名字（回车跳过并清空 / done 保存退出）> ", terminator: "")
            fflush(stdout)
            guard let line = readLine(strippingNewline: true) else { break }
            let name = line.trimmingCharacters(in: .whitespaces)

            lock.lock(); captured.removeAll(); lock.unlock()

            if name.isEmpty { print("  已跳过"); continue }
            if name.lowercased() == "done" || name.lowercased() == "q" {
                config.buttons = buttons
                do {
                    try ConfigStore.save(config)
                    print("\n✅ 已保存 \(buttons.count) 个按键到 \(ConfigStore.path.path)")
                    print("   现在可以编辑该文件的 profiles 部分来设置动作。")
                } catch {
                    print("❌ 保存失败: \(error.localizedDescription)")
                }
                exit(0)
            }
            buttons[name] = sigList
            print("  ✅ \(name) = \(sigList.joined(separator: ", "))")
        }
    }
    inputThread.start()
    CFRunLoopRun()
}

/// 只看不改：打印遥控器发出的所有信号
func cmdWatch() throws {
    let config = ConfigStore.exists() ? try ConfigStore.load() : defaultConfig()
    let engine = RemapEngine(config: config)
    engine.watchOnly = true
    try engine.start()
    print("正在监听设备 VID 0x\(String(config.device.vendorId, radix: 16)) / PID 0x\(String(config.device.productId, radix: 16))")
    print("按遥控器按键查看原始信号，Ctrl-C 退出。\n")
    CFRunLoopRun()
}

func cmdRun(verbose: Bool) throws {
    guard ConfigStore.exists() else { throw MiRemoteError.noConfig }
    var config = try ConfigStore.load()
    if verbose { config.options.verbose = true }
    let engine = RemapEngine(config: config)
    try engine.start()

    print("""
    miremote \(VERSION) 已启动
      设备: \(config.device.name ?? "?") (0x\(String(format: "%04x", config.device.vendorId))/0x\(String(format: "%04x", config.device.productId)))
      按键: \(config.buttons.count) 个
      方案: \(config.profiles.map { $0.name }.joined(separator: ", "))
      拦截原键: \(config.options.swallowOriginal ? "是" : "否")
    """)

    // 监听配置文件变化，自动热重载
    watchConfigFile { newCfg in engine.reload(newCfg) }

    // SIGHUP 手动重载
    signal(SIGHUP, SIG_IGN)
    let hup = DispatchSource.makeSignalSource(signal: SIGHUP, queue: .main)
    hup.setEventHandler {
        if let c = try? ConfigStore.load() { engine.reload(c) }
    }
    hup.resume()

    CFRunLoopRun()
}

private nonisolated(unsafe) var configWatchSource: DispatchSourceFileSystemObject?

func watchConfigFile(_ onChange: @escaping (Config) -> Void) {
    func attach() {
        let fd = open(ConfigStore.path.path, O_EVTONLY)
        guard fd >= 0 else { return }
        let src = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: fd, eventMask: [.write, .rename, .delete], queue: .main)
        src.setEventHandler {
            let flags = src.data
            if flags.contains(.write) {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                    if let c = try? ConfigStore.load() { onChange(c) }
                }
            } else {
                // 编辑器保存常是 rename，需要重新 attach
                src.cancel()
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                    if let c = try? ConfigStore.load() { onChange(c) }
                    attach()
                }
            }
        }
        src.setCancelHandler { close(fd) }
        src.resume()
        configWatchSource = src
    }
    attach()
}

// MARK: - launchd 安装

let plistLabel = "com.zyk.miremote"

func launchAgentPath() -> URL {
    FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/LaunchAgents/\(plistLabel).plist")
}

func cmdInstall() throws {
    // 优先指向 .app 内的二进制：只有真实 .app bundle 才能拿到蓝牙/语音识别权限
    let appBinary = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/Application Support/miremote/miremote.app/Contents/MacOS/miremote")
    let exe = URL(fileURLWithPath: CommandLine.arguments[0]).standardizedFileURL.path
    let plainResolved = (try? FileManager.default.destinationOfSymbolicLink(atPath: exe)) ?? exe

    let resolved: String
    if FileManager.default.fileExists(atPath: appBinary.path) {
        resolved = appBinary.path
        print("✓ 使用 .app bundle（语音功能可用）")
    } else {
        resolved = plainResolved
        print("⚠️  未找到 .app bundle，语音转文字将不可用（蓝牙权限限制）")
        print("   先运行 ./build-app.sh 再 install")
    }

    let logDir = ConfigStore.dir.appendingPathComponent("logs")
    try FileManager.default.createDirectory(at: logDir, withIntermediateDirectories: true)

    let plist: [String: Any] = [
        "Label": plistLabel,
        "ProgramArguments": [resolved, "run"],
        "RunAtLoad": true,
        "KeepAlive": ["SuccessfulExit": false],
        "ProcessType": "Interactive",
        "StandardOutPath": logDir.appendingPathComponent("miremote.log").path,
        "StandardErrorPath": logDir.appendingPathComponent("miremote.err.log").path,
        "EnvironmentVariables": ["PATH": "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"],
    ]
    let data = try PropertyListSerialization.data(fromPropertyList: plist, format: .xml, options: 0)
    let path = launchAgentPath()
    try FileManager.default.createDirectory(at: path.deletingLastPathComponent(),
                                           withIntermediateDirectories: true)
    try data.write(to: path, options: .atomic)

    let uid = getuid()
    _ = shell("/bin/launchctl", ["bootout", "gui/\(uid)/\(plistLabel)"])
    let r = shell("/bin/launchctl", ["bootstrap", "gui/\(uid)", path.path])
    print("📄 LaunchAgent: \(path.path)")
    if r.status == 0 {
        print("✅ 已安装并启动，开机自动运行。")
    } else {
        print("⚠️  launchctl bootstrap 返回 \(r.status): \(r.out)\(r.err)")
        print("   可手动执行: launchctl bootstrap gui/\(uid) \(path.path)")
    }
    print("📋 日志: \(logDir.appendingPathComponent("miremote.log").path)")
    print("\n重要：可执行文件路径变了要重新授权「辅助功能」。")
}

func cmdUninstall() {
    let uid = getuid()
    _ = shell("/bin/launchctl", ["bootout", "gui/\(uid)/\(plistLabel)"])
    try? FileManager.default.removeItem(at: launchAgentPath())
    print("✅ 已停止并移除开机自启。")
}

func cmdStatus() {
    let uid = getuid()
    let r = shell("/bin/launchctl", ["print", "gui/\(uid)/\(plistLabel)"])
    let installed = FileManager.default.fileExists(atPath: launchAgentPath().path)
    print("配置文件: \(ConfigStore.exists() ? "✅ \(ConfigStore.path.path)" : "❌ 未创建")")
    print("开机自启: \(installed ? "✅ 已安装" : "❌ 未安装")")
    print("运行状态: \(r.status == 0 ? "✅ 运行中" : "❌ 未运行")")
    if let c = try? ConfigStore.load() {
        let ids = DeviceWatcher.listAll().first { $0.vendor == c.device.vendorId && $0.product == c.device.productId }
        print("设备连接: \(ids != nil ? "✅ \(ids!.name)" : "❌ 未连接")")
        print("已学按键: \(c.buttons.keys.sorted().joined(separator: ", "))")
    }
    let trusted = CGPreflightListenEventAccess()
    print("辅助功能权限: \(trusted ? "✅ 已授权" : "❌ 未授权")")

    // 语音相关
    let appBinary = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/Application Support/miremote/miremote.app")
    let hasApp = FileManager.default.fileExists(atPath: appBinary.path)
    print(".app 包装: \(hasApp ? "✅ \(appBinary.path)" : "❌ 未构建（语音需要，运行 ./build-app.sh）")")
    let sr = SFSpeechRecognizer.authorizationStatus()
    let srName = [0: "未请求", 1: "❌ 已拒绝", 2: "❌ 受限", 3: "✅ 已授权"][sr.rawValue] ?? "?"
    print("语音识别权限: \(srName)")
}

func cmdReload() {
    let out = shell("/usr/bin/pkill", ["-HUP", "-f", "miremote run"])
    print(out.status == 0 ? "✅ 已发送重载信号" : "⚠️  没找到运行中的 miremote")
}

func cmdPreset() throws {
    guard ConfigStore.exists() else { throw MiRemoteError.noConfig }
    var config = try ConfigStore.load()
    guard !config.buttons.isEmpty else {
        print("还没有学习任何按键，请先运行 `miremote learn`。")
        return
    }
    let names = Set(config.buttons.keys)
    func b(_ key: String, _ a: Action) -> (String, Binding)? {
        names.contains(key) ? (key, Binding(tap: a)) : nil
    }

    var media: [String: Binding] = [:]
    for pair in [
        b("up", .media(key: "volup")),
        b("down", .media(key: "voldown")),
        b("left", .key(key: "left", mods: [])),
        b("right", .key(key: "right", mods: [])),
        b("ok", .media(key: "playpause")),
        b("back", .key(key: "escape", mods: [])),
        b("home", .open(target: "com.apple.finder")),
        b("menu", .key(key: "space", mods: [])),
    ].compactMap({ $0 }) { media[pair.0] = pair.1 }

    config.profiles = [
        Profile(name: "default", match: nil, bindings: media)
    ]
    try ConfigStore.save(config)
    print("✅ 已生成基础映射（音量/方向/播放暂停）到 default 方案。")
    print("   编辑 \(ConfigStore.path.path) 可继续细化。")
}

// MARK: - 工具

func shell(_ exe: String, _ args: [String]) -> (status: Int32, out: String, err: String) {
    let p = Process()
    p.executableURL = URL(fileURLWithPath: exe)
    p.arguments = args
    let o = Pipe(), e = Pipe()
    p.standardOutput = o; p.standardError = e
    do { try p.run() } catch { return (-1, "", error.localizedDescription) }
    let od = o.fileHandleForReading.readDataToEndOfFile()
    let ed = e.fileHandleForReading.readDataToEndOfFile()
    p.waitUntilExit()
    return (p.terminationStatus, String(decoding: od, as: UTF8.self), String(decoding: ed, as: UTF8.self))
}

extension String {
    func padded(_ n: Int) -> String {
        // 中文按 2 宽度粗略计算
        var w = 0
        for ch in self { w += ch.unicodeScalars.first!.value > 0x2000 ? 2 : 1 }
        return self + String(repeating: " ", count: max(0, n - w))
    }
}

func usage() {
    print("""
    miremote \(VERSION) — 小米蓝牙遥控器按键映射工具 (macOS)

    用法: miremote <命令> [选项]

    命令:
      init [--force]   生成默认配置文件
      devices          列出所有已连接 HID 设备（查 VID/PID）
      learn            交互式学习遥控器按键，写入配置
      preset           根据已学按键生成一套基础映射
      watch            实时打印遥控器发出的原始信号（不映射）
      run [-v]         前台运行映射服务
      install          安装为开机自启的后台服务 (launchd)
      uninstall        卸载开机自启
      status           查看配置 / 权限 / 运行状态
      reload           让运行中的服务重新读取配置
      help             显示本帮助

    配置文件: \(ConfigStore.path.path)

    首次使用:
      1. miremote init
      2. 到 系统设置 › 隐私与安全性 › 辅助功能 授权 miremote
      3. miremote learn      # 逐个按键并命名
      4. 编辑配置文件设置动作，或先跑 miremote preset
      5. miremote run        # 试运行；满意后 miremote install
    """)
}

// MARK: - 入口

let args = Array(CommandLine.arguments.dropFirst())
let cmd = args.first ?? "help"
let flags = Set(args.dropFirst())

do {
    switch cmd {
    case "init":      try cmdInit(force: flags.contains("--force") || flags.contains("-f"))
    case "devices":   cmdDevices()
    case "learn":     try cmdLearn()
    case "preset":    try cmdPreset()
    case "watch":     try cmdWatch()
    case "run":       try cmdRun(verbose: flags.contains("-v") || flags.contains("--verbose"))
    case "install":   try cmdInstall()
    case "uninstall": cmdUninstall()
    case "status":    cmdStatus()
    case "reload":    cmdReload()
    case "version", "--version", "-V": print("miremote \(VERSION)")
    default:          usage()
    }
} catch {
    Log.error(error.localizedDescription)
    exit(1)
}
