import Foundation

/// 把流式识别的中间结果实时写到当前输入框。
///
/// 难点：识别器会不断修正已经说过的词（「你好这是」→「你好这是一个」，
/// 也可能「提交」→「commit」），所以不能只做追加，必须删掉分歧点之后
/// 的旧字再补新字。用公共前缀比对，把敲退格的次数降到最低。
final class LiveTyper {

    private let emitter: Emitter
    /// 已经敲进去的文字
    private var onScreen = ""
    /// 串行掉所有输入操作，避免中间结果互相赶车
    private let queue = DispatchQueue(label: "miremote.livetyper")

    init(emitter: Emitter) {
        self.emitter = emitter
    }

    /// 开新一句
    func reset() {
        queue.async { [weak self] in self?.onScreen = "" }
    }

    /// 收到新的（累计全文）识别结果，把屏幕上的内容追上去
    func update(to target: String) {
        queue.async { [weak self] in
            self?.applyDiff(to: target)
        }
    }

    /// 一句话结束：确保屏幕内容等于最终文本
    func finalize(_ text: String, completion: (() -> Void)? = nil) {
        queue.async { [weak self] in
            guard let self else { return }
            self.applyDiff(to: text)
            self.onScreen = ""
            if let completion {
                DispatchQueue.main.async(execute: completion)
            }
        }
    }

    /// 把已敲的内容全部删掉（识别失败时回滚）
    func clear() {
        queue.async { [weak self] in
            guard let self, !self.onScreen.isEmpty else { return }
            self.emitter.pressBackspace(times: self.onScreen.count)
            self.onScreen = ""
        }
    }

    // MARK: 差异计算

    private func applyDiff(to target: String) {
        guard target != onScreen else { return }

        let common = Self.commonPrefixLength(onScreen, target)
        let toDelete = onScreen.count - common
        if toDelete > 0 {
            emitter.pressBackspace(times: toDelete)
        }
        let suffix = String(target.dropFirst(common))
        if !suffix.isEmpty {
            emitter.typeText(suffix)
        }
        onScreen = target
    }

    /// 按字符（而非 UTF-16 code unit）算公共前缀长度，
    /// 因为退格键删的是字符，中文一个字一下。
    private static func commonPrefixLength(_ a: String, _ b: String) -> Int {
        var n = 0
        var i = a.startIndex
        var j = b.startIndex
        while i < a.endIndex, j < b.endIndex, a[i] == b[j] {
            n += 1
            i = a.index(after: i)
            j = b.index(after: j)
        }
        return n
    }
}
