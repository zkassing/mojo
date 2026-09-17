//! 实时上屏的差异计算（对应 Swift `LiveTyper`）。
//!
//! 流式识别会不断修正已经说过的词（「你好这是」→「你好这是一个」，也可能
//! 「提交」→「commit」），所以不能只做追加：用公共前缀比对找出分歧点，
//! 删掉分歧点之后的旧字再补新字，把敲退格的次数降到最低。
//!
//! 本模块是纯计算；真正敲退格 / 打字由平台 Emitter 执行。

/// 按字符（而非字节）算公共前缀长度 —— 退格删的是字符，中文一个字一下
pub fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

/// 由上屏文本和最新识别全文，算出追赶动作：(敲退格次数, 要补打的文本)
pub fn diff_plan(on_screen: &str, target: &str) -> (usize, String) {
    if on_screen == target {
        return (0, String::new());
    }
    let common = common_prefix_len(on_screen, target);
    let backspaces = on_screen.chars().count() - common;
    let suffix: String = target.chars().skip(common).collect();
    (backspaces, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_append() {
        let (bs, suffix) = diff_plan("你好这是", "你好这是一个");
        assert_eq!(bs, 0);
        assert_eq!(suffix, "一个");
    }

    #[test]
    fn correction_rewrites_tail() {
        // 「提交」被修正成「commit」：公共前缀只有「请」，删后 4 字再补打
        let (bs, suffix) = diff_plan("请提交代码", "请commit代码");
        assert_eq!(bs, 4);
        assert_eq!(suffix, "commit代码");
    }

    #[test]
    fn identical_is_noop() {
        assert_eq!(diff_plan("abc", "abc"), (0, String::new()));
    }

    #[test]
    fn full_rollback() {
        // 识别完全改口：全删重打
        let (bs, suffix) = diff_plan("完全错误", "recognition");
        assert_eq!(bs, 4);
        assert_eq!(suffix, "recognition");
    }

    #[test]
    fn empty_target_deletes_all() {
        let (bs, suffix) = diff_plan("三个字", "");
        assert_eq!(bs, 3);
        assert_eq!(suffix, "");
    }

    #[test]
    fn emoji_counts_as_one_char() {
        // char 维度：emoji 一次退格（与 Swift Character 语义一致；
        // 注意：复合 emoji（ZWJ 序列）在 Swift 里是一个 Character、
        // 在 Rust chars() 里是多个 —— ASR 中文场景不涉及，如有需要再换 unicode-segmentation）
        let (bs, suffix) = diff_plan("好👍", "好👌");
        assert_eq!(bs, 1);
        assert_eq!(suffix, "👌");
    }
}
