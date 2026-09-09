import Foundation

/// 把 ASR 识别错的技术术语纠正回来。
///
/// 火山的大模型 ASR 对中文口语很准，但技术黑话经常按谐音写成中文词：
/// 「main 分支」→「闷分之」、「diff」→「地府」、「commit」→「康米特」。
/// 热词表能改善一部分，但覆盖不全，所以在输出前再过一遍本地替换。
///
/// 规则按长度降序匹配，避免「git push」被「git」先吃掉。
struct TermFixer {

    /// 谐音 → 正确术语。键全部小写化后比对。
    private static let table: [String: String] = [
        // Git
        "闷分之": "main 分支", "闷分支": "main 分支", "麦分支": "main 分支",
        "马斯特": "master", "马斯特分支": "master 分支",
        "地府": "diff", "低阜": "diff", "底部": "diff",
        "康米特": "commit", "开米特": "commit", "科密特": "commit",
        "普希": "push", "扑西": "push", "铺设": "push",
        "普尔": "pull", "扑尔": "pull",
        "瑞贝斯": "rebase", "瑞贝": "rebase",
        "麦哲": "merge", "莫吉": "merge", "么吉": "merge",
        "切克奥特": "checkout", "checkout 到": "checkout ",
        "斯塔什": "stash", "克隆": "clone",
        "布兰奇": "branch", "布朗奇": "branch",
        "瑞莫特": "remote", "瑞破": "repo", "瑞泡": "repo",
        "康弗利克特": "conflict", "冲突解决": "conflict",
        "黑客": "hunk", "亨克": "hunk",

        // 常见命令
        "cd 到": "cd ", "艾尔艾斯": "ls", "格瑞普": "grep",
        "卡特": "cat", "麦克": "make", "赛德": "sed",
        "艾瓦克": "awk", "肯": "cat",
        "恩皮恩": "npm", "阳": "yarn", "偏": "pnpm",
        "派森": "python", "派": "pip",
        "斗客": "docker", "多克": "docker",
        "库伯耐踢死": "kubernetes", "k8s": "k8s",

        // 编程概念
        "放克身": "function", "放克宪": "function", "方克身": "function",
        "瑞特恩": "return", "润": "return",
        "阿辛克": "async", "阿维特": "await",
        "康波内特": "component", "康朋内特": "component",
        "普罗普斯": "props", "普罗普": "prop",
        "斯泰特": "state", "斯坦特": "state",
        "胡克": "hook", "虎克": "hook",
        "泰普斯克瑞普特": "TypeScript", "太谱": "type",
        "因特菲斯": "interface", "英特菲斯": "interface",
        "瑞菲克特": "refactor", "瑞法克特": "refactor",
        "迪巴格": "debug", "地八哥": "debug",
        "特斯特": "test", "泰斯特": "test",
        "林特": "lint", "领特": "lint",
        "拜偶德": "build", "比欧德": "build",
        "艾皮艾": "API", "诶皮诶": "API",
        "恩德泡因特": "endpoint",
        "瑞快斯特": "request", "瑞斯庞斯": "response",
        "阿瑞": "array", "奥布杰克特": "object",
        "斯特灵": "string", "布尔": "bool",
        "艾瑞尔": "error", "诶勒": "error",
        "洛格": "log", "康所": "console",
        "英波特": "import", "艾克斯波特": "export",
        "康斯特": "const", "累特": "let",
        "克拉斯": "class", "买色的": "method",
        "帕拉米特": "parameter", "阿规门特": "argument",
        "瓦瑞博": "variable",

        // 常见句式修正
        "和平笔记": "合并", "和平": "合并",
    ]

    /// 按长度降序排好的规则，长的先匹配
    private static let sorted: [(String, String)] =
        table.sorted { $0.key.count > $1.key.count }.map { ($0.key, $0.value) }

    /// 纠正一段识别文本
    static func fix(_ text: String) -> String {
        guard !text.isEmpty else { return text }
        var out = text
        for (wrong, right) in sorted {
            guard out.localizedCaseInsensitiveContains(wrong) else { continue }
            out = out.replacingOccurrences(
                of: wrong, with: right, options: [.caseInsensitive])
        }
        return out
    }

    /// 热词表：喂给 ASR 让它优先往这些词上靠
    static let hotWords: [String] = [
        // Git
        "git", "commit", "push", "pull", "merge", "rebase", "branch",
        "checkout", "stash", "clone", "remote", "origin", "main", "master",
        "diff", "hunk", "conflict", "cherry-pick", "reset", "revert", "log",
        // Shell
        "cd", "ls", "grep", "cat", "sed", "awk", "find", "chmod", "sudo",
        "npm", "yarn", "pnpm", "node", "python", "pip", "cargo", "make",
        "docker", "kubectl", "kubernetes", "ssh", "curl", "brew", "swift",
        // 编程
        "function", "return", "async", "await", "const", "let", "var",
        "class", "interface", "type", "TypeScript", "JavaScript", "Swift",
        "React", "component", "props", "state", "hook", "useEffect",
        "import", "export", "default", "null", "undefined", "boolean",
        "string", "number", "array", "object", "promise", "callback",
        "API", "endpoint", "request", "response", "JSON", "HTTP",
        "refactor", "debug", "test", "lint", "build", "deploy", "error",
        "console", "log", "warning", "exception", "stack", "trace",
        "database", "query", "schema", "migration", "index",
        // AI coding
        "Claude", "Codex", "Copilot", "prompt", "token", "context",
        "agent", "MCP", "LLM",
    ]
}
