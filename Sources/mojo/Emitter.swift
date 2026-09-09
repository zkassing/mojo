import Foundation
import CoreGraphics
import IOKit
import IOKit.hid
import AppKit

// MARK: - 按键名 -> CGKeyCode

enum KeyMap {
    static let table: [String: CGKeyCode] = [
        "a":0,"s":1,"d":2,"f":3,"h":4,"g":5,"z":6,"x":7,"c":8,"v":9,
        "b":11,"q":12,"w":13,"e":14,"r":15,"y":16,"t":17,
        "1":18,"2":19,"3":20,"4":21,"6":22,"5":23,"9":25,"7":26,"8":28,"0":29,
        "=":24,"-":27,"]":30,"o":31,"u":32,"[":33,"i":34,"p":35,
        "l":37,"j":38,"'":39,"k":40,";":41,"\\":42,",":43,"/":44,"n":45,"m":46,".":47,
        "`":50,
        "return":36,"enter":36,"tab":48,"space":49,"delete":51,"backspace":51,
        "escape":53,"esc":53,
        "capslock":57,
        "f1":122,"f2":120,"f3":99,"f4":118,"f5":96,"f6":97,"f7":98,"f8":100,
        "f9":101,"f10":109,"f11":103,"f12":111,"f13":105,"f14":107,"f15":113,
        "f16":106,"f17":64,"f18":79,"f19":80,"f20":90,
        "home":115,"end":119,"pageup":116,"pagedown":121,
        "forwarddelete":117,"del":117,
        "left":123,"right":124,"down":125,"up":126,
        "help":114,"fn":63,
        // 单独的修饰键（左键位）：让「按一下 Cmd/Ctrl」也能作为动作发出去
        "cmd":55,"command":55,"ctrl":59,"control":59,
        "alt":58,"opt":58,"option":58,"shift":56,
        "keypad0":82,"keypad1":83,"keypad2":84,"keypad3":85,"keypad4":86,
        "keypad5":87,"keypad6":88,"keypad7":89,"keypad8":91,"keypad9":92,
        "keypadenter":76,"keypadplus":69,"keypadminus":78,
        "keypadmultiply":67,"keypaddivide":75,"keypaddecimal":65,"keypadequals":81,
        "keypadclear":71,
    ]

    static func code(for name: String) -> CGKeyCode? {
        table[name.lowercased()]
    }

    static func name(forCode code: Int64) -> String {
        for (k, v) in table where Int64(v) == code { return k }
        return "kc:\(code)"
    }

    static func flags(_ mods: [String]) -> CGEventFlags {
        var f = CGEventFlags()
        for m in mods {
            switch m.lowercased() {
            case "cmd", "command", "meta", "super": f.insert(.maskCommand)
            case "shift": f.insert(.maskShift)
            case "opt", "option", "alt": f.insert(.maskAlternate)
            case "ctrl", "control": f.insert(.maskControl)
            case "fn", "function": f.insert(.maskSecondaryFn)
            default: break
            }
        }
        return f
    }
}

// MARK: - 媒体键（NX_SYSDEFINED aux key）

enum MediaKey {
    // NX_KEYTYPE_*
    static let table: [String: Int32] = [
        "brightnessup": 2, "brightnessdown": 3,
        "mute": 7, "volup": 0, "voldown": 1,
        "playpause": 16, "play": 16, "pause": 16,
        "next": 17, "nexttrack": 17,
        "previous": 18, "prev": 18, "prevtrack": 18,
        "fast": 19, "forward": 19,
        "rewind": 20,
        "illuminationup": 21, "illuminationdown": 22,
        "eject": 14,
        "capslock": 4,
    ]

    static func name(forCode code: Int32) -> String {
        for (k, v) in table where v == code { return k }
        return "aux:\(code)"
    }
}

// MARK: - 事件发送

final class Emitter {
    private let src: CGEventSource?
    private let nxp = NXPoster()

    init() {
        // 用 hidSystemState：合成事件携带 HID 系统源状态，对全局热键监听者
        // （微信等会过滤 privateState 来源事件）更像真实硬件事件
        src = CGEventSource(stateID: .hidSystemState)
        src?.localEventsSuppressionInterval = 0
    }

    func perform(_ action: Action) {
        switch action {
        case .key(let key, let mods):
            sendKey(key, mods: mods)
        case .media(let key):
            sendMedia(key)
        case .shell(let cmd):
            runShell(cmd)
        case .open(let target):
            openTarget(target)
        case .mouseMove(let dx, let dy):
            moveMouse(dx: dx, dy: dy)
        case .mouseClick(let b, let n):
            clickMouse(button: b, count: n)
        case .mouseScroll(let dx, let dy):
            scroll(dx: dx, dy: dy)
        case .sequence(let list):
            for a in list { perform(a); usleep(25_000) }
        case .none, .passthrough, .dictate:
            // dictate 由 RemapEngine 的按住/松开钩子处理，不在这里发事件
            break
        }
    }

    // MARK: 键盘

    func sendKey(_ key: String, mods: [String]) {
        guard let code = KeyMap.code(for: key) else {
            Log.warn("未知按键名: \(key)")
            return
        }
        // 优先驱动层注入（IOHIDPostEvent）：事件无进程 PID，与物理键盘
        // 不可区分，能触发过滤 CGEvent 合成事件的全局热键（微信等）
        if nxp.sendKey(macKeyCode: Int64(code), mods: mods) { return }
        // CGEvent 兜底
        // 键本身是修饰键（fn/cmd/ctrl/alt/shift）时，按下事件必须带上自己的
        // flag —— 修饰键走的是 flagsChanged 事件，不带 flag 的话监听方
        // 会认为是空事件而忽略。松开时清掉自己的 flag。
        let selfFlag = KeyMap.flags([key])
        let downFlags = KeyMap.flags(mods).union(selfFlag)
        let upFlags = KeyMap.flags(mods).subtracting(selfFlag)
        guard let down = CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: true),
              let up = CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: false) else { return }
        down.flags = downFlags
        up.flags = upFlags
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
    }

    // MARK: 媒体键

    func sendMedia(_ key: String) {
        guard let code = MediaKey.table[key.lowercased()] else {
            Log.warn("未知媒体键: \(key)")
            return
        }
        postAux(code, down: true)
        postAux(code, down: false)
    }

    private func postAux(_ keyCode: Int32, down: Bool) {
        let flags = NSEvent.ModifierFlags(rawValue: down ? 0xA00 : 0xB00)
        let data1 = Int((keyCode << 16) | ((down ? 0xA : 0xB) << 8))
        guard let ev = NSEvent.otherEvent(with: .systemDefined,
                                          location: .zero,
                                          modifierFlags: flags,
                                          timestamp: 0,
                                          windowNumber: 0,
                                          context: nil,
                                          subtype: 8,
                                          data1: data1,
                                          data2: -1),
              let cg = ev.cgEvent else { return }
        cg.post(tap: .cghidEventTap)
    }

    // MARK: 文本输入（LiveTyper 用）

    /// 输入一段文本（Unicode 直输，不经键码）
    func typeText(_ text: String) {
        guard !text.isEmpty else { return }
        guard let down = CGEvent(keyboardEventSource: src, virtualKey: 0, keyDown: true),
              let up = CGEvent(keyboardEventSource: src, virtualKey: 0, keyDown: false) else { return }
        var chars = Array(text.utf16)
        down.keyboardSetUnicodeString(stringLength: chars.count, unicodeString: &chars)
        up.keyboardSetUnicodeString(stringLength: chars.count, unicodeString: &chars)
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
    }

    /// 连按 n 次退格（撤回已上屏的中间结果）
    func pressBackspace(times n: Int) {
        guard n > 0 else { return }
        for _ in 0..<n { sendKey("delete", mods: []) }
    }

    // MARK: shell

    func runShell(_ cmd: String) {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/zsh")
        p.arguments = ["-lc", cmd]
        p.standardOutput = FileHandle.nullDevice
        p.standardError = FileHandle.nullDevice
        do { try p.run() } catch { Log.warn("shell 执行失败: \(error.localizedDescription)") }
    }

    // MARK: open

    func openTarget(_ target: String) {
        let t = target.trimmingCharacters(in: .whitespaces)
        guard !t.isEmpty else { return }
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/open")
        if t.contains("://") {
            p.arguments = [t]
        } else if t.contains(".") && !t.hasPrefix("/") && !t.hasSuffix(".app") {
            // 看起来像 bundle id
            p.arguments = ["-b", t]
        } else if t.hasPrefix("/") {
            p.arguments = [t]
        } else {
            p.arguments = ["-a", t]
        }
        p.standardOutput = FileHandle.nullDevice
        p.standardError = FileHandle.nullDevice
        try? p.run()
    }

    // MARK: 鼠标

    func moveMouse(dx: Double, dy: Double) {
        let cur = currentMouse()
        let target = CGPoint(x: cur.x + dx, y: cur.y + dy)
        CGEvent(mouseEventSource: src, mouseType: .mouseMoved,
                mouseCursorPosition: target, mouseButton: .left)?.post(tap: .cghidEventTap)
    }

    func clickMouse(button: String, count: Int) {
        let pos = currentMouse()
        let (btn, dn, up): (CGMouseButton, CGEventType, CGEventType) = {
            switch button.lowercased() {
            case "right": return (.right, .rightMouseDown, .rightMouseUp)
            case "middle", "center": return (.center, .otherMouseDown, .otherMouseUp)
            default: return (.left, .leftMouseDown, .leftMouseUp)
            }
        }()
        for i in 1...max(1, count) {
            guard let d = CGEvent(mouseEventSource: src, mouseType: dn, mouseCursorPosition: pos, mouseButton: btn),
                  let u = CGEvent(mouseEventSource: src, mouseType: up, mouseCursorPosition: pos, mouseButton: btn)
            else { return }
            d.setIntegerValueField(.mouseEventClickState, value: Int64(i))
            u.setIntegerValueField(.mouseEventClickState, value: Int64(i))
            d.post(tap: .cghidEventTap); u.post(tap: .cghidEventTap)
            if i < count { usleep(40_000) }
        }
    }

    func scroll(dx: Int, dy: Int) {
        CGEvent(scrollWheelEvent2Source: src, units: .pixel, wheelCount: 2,
                wheel1: Int32(dy), wheel2: Int32(dx), wheel3: 0)?.post(tap: .cghidEventTap)
    }

    private func currentMouse() -> CGPoint {
        let p = NSEvent.mouseLocation
        let h = NSScreen.screens.first?.frame.height ?? 0
        // NSEvent 是左下原点，CGEvent 是左上原点
        let maxY = NSScreen.screens.map { $0.frame.maxY }.max() ?? h
        return CGPoint(x: p.x, y: maxY - p.y)
    }
}
