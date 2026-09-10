//! 技术术语谐音误识纠正（对应 Swift `TermFixer`）。
//!
//! ASR 对中文口语很准，但技术黑话经常按谐音写成中文词：
//! 「main 分支」→「闷分之」、「diff」→「地府」、「commit」→「康米特」。
//! 规则按长度降序匹配，避免「git push」被「git」先吃掉。

/// 谐音 → 正确术语
const TABLE: &[(&str, &str)] = &[
    // Git
    ("闷分之", "main 分支"),
    ("闷分支", "main 分支"),
    ("麦分支", "main 分支"),
    ("马斯特", "master"),
    ("马斯特分支", "master 分支"),
    ("地府", "diff"),
    ("低阜", "diff"),
    ("底部", "diff"),
    ("康米特", "commit"),
    ("开米特", "commit"),
    ("科密特", "commit"),
    ("普希", "push"),
    ("扑西", "push"),
    ("铺设", "push"),
    ("普尔", "pull"),
    ("扑尔", "pull"),
    ("瑞贝斯", "rebase"),
    ("瑞贝", "rebase"),
    ("麦哲", "merge"),
    ("莫吉", "merge"),
    ("么吉", "merge"),
    ("切克奥特", "checkout"),
    ("checkout 到", "checkout "),
    ("斯塔什", "stash"),
    ("克隆", "clone"),
    ("布兰奇", "branch"),
    ("布朗奇", "branch"),
    ("瑞莫特", "remote"),
    ("瑞破", "repo"),
    ("瑞泡", "repo"),
    ("康弗利克特", "conflict"),
    ("冲突解决", "conflict"),
    ("黑客", "hunk"),
    ("亨克", "hunk"),
    // 常见命令
    ("cd 到", "cd "),
    ("艾尔艾斯", "ls"),
    ("格瑞普", "grep"),
    ("卡特", "cat"),
    ("麦克", "make"),
    ("赛德", "sed"),
    ("艾瓦克", "awk"),
    ("肯", "cat"),
    ("恩皮恩", "npm"),
    ("阳", "yarn"),
    ("偏", "pnpm"),
    ("派森", "python"),
    ("派", "pip"),
    ("斗客", "docker"),
    ("多克", "docker"),
    ("库伯耐踢死", "kubernetes"),
    ("k8s", "k8s"),
    // 编程概念
    ("放克身", "function"),
    ("放克宪", "function"),
    ("方克身", "function"),
    ("瑞特恩", "return"),
    ("润", "return"),
    ("阿辛克", "async"),
    ("阿维特", "await"),
    ("康波内特", "component"),
    ("康朋内特", "component"),
    ("普罗普斯", "props"),
    ("普罗普", "prop"),
    ("斯泰特", "state"),
    ("斯坦特", "state"),
    ("胡克", "hook"),
    ("虎克", "hook"),
    ("泰普斯克瑞普特", "TypeScript"),
    ("太谱", "type"),
    ("因特菲斯", "interface"),
    ("英特菲斯", "interface"),
    ("瑞菲克特", "refactor"),
    ("瑞法克特", "refactor"),
    ("迪巴格", "debug"),
    ("地八哥", "debug"),
    ("特斯特", "test"),
    ("泰斯特", "test"),
    ("林特", "lint"),
    ("领特", "lint"),
    ("拜偶德", "build"),
    ("比欧德", "build"),
    ("艾皮艾", "API"),
    ("诶皮诶", "API"),
    ("恩德泡因特", "endpoint"),
    ("瑞快斯特", "request"),
    ("瑞斯庞斯", "response"),
    ("阿瑞", "array"),
    ("奥布杰克特", "object"),
    ("斯特灵", "string"),
    ("布尔", "bool"),
    ("艾瑞尔", "error"),
    ("诶勒", "error"),
    ("洛格", "log"),
    ("康所", "console"),
    ("英波特", "import"),
    ("艾克斯波特", "export"),
    ("康斯特", "const"),
    ("累特", "let"),
    ("克拉斯", "class"),
    ("买色的", "method"),
    ("帕拉米特", "parameter"),
    ("阿规门特", "argument"),
    ("瓦瑞博", "variable"),
    // 常见句式修正
    ("和平笔记", "合并"),
    ("和平", "合并"),
    // Rust：中英文模型常听成英文近音词或中文谐音。
    // 注意英文规则只整词替换（见 fix()），避免误伤 restaurant/raspberry 等正常单词；
    // "rest" 故意不收 —— 说 REST API 时不能误改。
    ("russ", "Rust"),
    ("ras", "Rust"),
    ("拉斯特", "Rust"),
    ("拉丝特", "Rust"),
    ("若斯特", "Rust"),
    ("罗斯特", "Rust"),
    ("如斯特", "Rust"),
];

/// 按长度降序排好的规则（长的先匹配）
fn sorted_rules() -> &'static [(&'static str, &'static str)] {
    use std::sync::OnceLock;
    static SORTED: OnceLock<Vec<(&'static str, &'static str)>> = OnceLock::new();
    SORTED.get_or_init(|| {
        let mut v: Vec<_> = TABLE.to_vec();
        v.sort_by_key(|(k, _)| std::cmp::Reverse(k.chars().count()));
        v
    })
}

/// 纠正一段识别文本
pub fn fix(text: &str) -> String {
    if text.is_empty() {
        return text.to_string();
    }
    let mut out = text.to_string();
    for (wrong, right) in sorted_rules() {
        if wrong.is_ascii() {
            // 英文短词只做整词替换（词边界），否则 "ras" 会误伤 raspberry
            out = replace_ascii_word(&out, wrong, right);
        } else if out.to_lowercase().contains(&wrong.to_lowercase()) {
            out = replace_case_insensitive(&out, wrong, right);
        }
    }
    out
}

/// ASCII 规则专用：仅当 needle 两侧不是字母/数字（整词）时才替换
fn replace_ascii_word(hay: &str, needle: &str, rep: &str) -> String {
    let h: Vec<char> = hay.chars().collect();
    let n: Vec<char> = needle.chars().map(|c| c.to_ascii_lowercase()).collect();
    if n.is_empty() || h.len() < n.len() {
        return hay.to_string();
    }
    let mut res = String::with_capacity(hay.len());
    let mut i = 0;
    while i < h.len() {
        let matched = i + n.len() <= h.len()
            && h[i..i + n.len()]
                .iter()
                .map(|c| c.to_ascii_lowercase())
                .eq(n.iter().copied());
        let left_ok = i == 0 || !h[i - 1].is_ascii_alphanumeric();
        let right_ok = i + n.len() >= h.len() || !h[i + n.len()].is_ascii_alphanumeric();
        if matched && left_ok && right_ok {
            res.push_str(rep);
            i += n.len();
        } else {
            res.push(h[i]);
            i += 1;
        }
    }
    res
}

/// 不区分大小写替换所有出现（按字符对齐，中文一个字算一个）
fn replace_case_insensitive(hay: &str, needle: &str, rep: &str) -> String {
    let h: Vec<char> = hay.chars().collect();
    let n: Vec<char> = needle.chars().map(|c| c.to_ascii_lowercase()).collect();
    if n.is_empty() || h.len() < n.len() {
        return hay.to_string();
    }
    let mut res = String::with_capacity(hay.len());
    let mut i = 0;
    while i < h.len() {
        if i + n.len() <= h.len()
            && h[i..i + n.len()]
                .iter()
                .map(|c| c.to_ascii_lowercase())
                .eq(n.iter().copied())
        {
            res.push_str(rep);
            i += n.len();
        } else {
            res.push(h[i]);
            i += 1;
        }
    }
    res
}

/// 热词表：喂给 ASR 让它优先往这些词上靠（预留给引擎 hotwords 能力）
pub const HOT_WORDS: &[&str] = &[
    // Git
    "git", "commit", "push", "pull", "merge", "rebase", "branch", "checkout", "stash", "clone",
    "remote", "origin", "main", "master", "diff", "hunk", "conflict", "cherry-pick", "reset",
    "revert", "log",
    // Shell
    "cd", "ls", "grep", "cat", "sed", "awk", "find", "chmod", "sudo", "npm", "yarn", "pnpm",
    "node", "python", "pip", "cargo", "make", "docker", "kubectl", "kubernetes", "ssh", "curl",
    "brew", "swift",
    // 编程
    "function", "return", "async", "await", "const", "let", "var", "class", "interface", "type",
    "TypeScript", "JavaScript", "Swift", "React", "component", "props", "state", "hook",
    "useEffect", "import", "export", "default", "null", "undefined", "boolean", "string",
    "number", "array", "object", "promise", "callback", "API", "endpoint", "request", "response",
    "JSON", "HTTP", "refactor", "debug", "test", "lint", "build", "deploy", "error", "console",
    "log", "warning", "exception", "stack", "trace", "database", "query", "schema", "migration",
    "index",
    // AI coding
    "Claude", "Codex", "Copilot", "prompt", "token", "context", "agent", "MCP", "LLM",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_common_terms() {
        assert_eq!(fix("把代码康米特到闷分之"), "把代码commit到main 分支");
        assert_eq!(fix("看一下地府"), "看一下diff");
        assert_eq!(fix("先普希再普尔"), "先push再pull");
    }

    #[test]
    fn longer_rule_wins() {
        // 「马斯特分支」整体规则优先于「马斯特」
        assert_eq!(fix("切到马斯特分支"), "切到master 分支");
    }

    #[test]
    fn case_insensitive_ascii() {
        assert_eq!(fix("K8S 部署"), "k8s 部署");
    }

    #[test]
    fn empty_and_no_match() {
        assert_eq!(fix(""), "");
        assert_eq!(fix("今天天气不错"), "今天天气不错");
    }

    #[test]
    fn rust_homophones() {
        // 英文近音词：整词替换（识别原文混合大小写都能命中）
        assert_eq!(fix("开启 Russ 的服务"), "开启 Rust 的服务");
        assert_eq!(fix("用 ras 写个工具"), "用 Rust 写个工具");
        // 中文谐音
        assert_eq!(fix("拉斯特语言"), "Rust语言");
        assert_eq!(fix("学一下若斯特"), "学一下Rust");
    }

    #[test]
    fn ascii_rules_need_word_boundary() {
        // 词边界保护：ras 不能误伤 raspberry，k8s 不能误伤 k8shell
        assert_eq!(fix("吃个 raspberry"), "吃个 raspberry");
        assert_eq!(fix("restaurant 不动"), "restaurant 不动");
        // REST API 故意不收进规则表：保持原样
        assert_eq!(fix("调 rest 接口"), "调 rest 接口");
    }

    #[test]
    fn rules_sorted_by_len_desc() {
        let r = sorted_rules();
        for w in r.windows(2) {
            assert!(w[0].0.chars().count() >= w[1].0.chars().count());
        }
    }
}
