//! 跨 JNI 的扁平编码：结构化结果一律编成 `String[]`，Kotlin 侧 `Wire.kt` 解码。
//!
//! # 为什么不在 Rust 侧 new Java 对象
//!
//! 逐字段 `NewObject` 要在两边各维护一份类名 + 构造签名，写错只在运行期炸（`NoSuchMethodError`），
//! 而且 CI 里没有设备跑不到。字符串数组只有一种 JNI 形状，解码逻辑是纯 Kotlin，能在 JVM
//! 单测里跑——跨边界的契约测试落在能自动执行的一侧。
//!
//! # 格式
//!
//! 字段分隔符 [`US`]（U+001F，单元分隔符）。**正文永远放最后一个字段**，解码时按「最多切 N 段」
//! 切，正文里即便出现 U+001F 也不会错位。
//!
//! 按键结果（[`outcome`]）：第 0 项是标志串（`c` = 已消费，`m` = 模式变化），其后每项一条编辑指令：
//!
//! | 指令 | 编码 |
//! |---|---|
//! | 上屏 | `C<文本>` |
//! | 组字 | `S<光标 UTF-16 偏移><US><文本>` |
//! | 删除光标前 | `D<字符数>` |
//! | 替换光标前 | `R<字符数><US><文本>` |
//! | 移动光标 | `M<偏移>` |
//!
//! 时序提示（`TimingHint`）是 TSF 专属编排，薄宿主忽略（`ops` 里已含等价降级），不编码。

use moodflow_mobile::{EditOp, KeyOutcome};

/// 字段分隔符。
pub const US: char = '\u{1F}';

/// 按键结果 → 字符串数组。
pub fn outcome(o: &KeyOutcome) -> Vec<String> {
    let mut flags = String::new();
    if o.consumed {
        flags.push('c');
    }
    if o.mode_changed {
        flags.push('m');
    }
    let mut out = Vec::with_capacity(o.ops.len() + 1);
    out.push(flags);
    for op in &o.ops {
        out.push(match op {
            EditOp::Commit(text) => format!("C{text}"),
            EditOp::SetComposition { text, caret } => {
                format!("S{}{US}{text}", utf8_to_utf16_offset(text, *caret))
            }
            EditOp::DeleteBackward { count } => format!("D{count}"),
            EditOp::ReplaceBackward { count, text } => format!("R{count}{US}{text}"),
            EditOp::MoveCursor { delta } => format!("M{delta}"),
        });
    }
    out
}

/// 核心给的是 UTF-8 **字节**偏移，Android 的文本 API 吃 UTF-16 码元偏移。
/// 偏移不在字符边界或越界时取整串末尾（组字光标落在末尾是最安全的降级）。
pub fn utf8_to_utf16_offset(text: &str, byte_offset: usize) -> usize {
    match text.get(..byte_offset) {
        Some(prefix) => prefix.encode_utf16().count(),
        None => text.encode_utf16().count(),
    }
}

/// 若干字段连成一项（正文字段放最后）。
pub fn join(fields: &[&str]) -> String {
    let mut s = String::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            s.push(US);
        }
        s.push_str(f);
    }
    s
}

pub fn flag(b: bool) -> &'static str {
    if b { "1" } else { "0" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_encodes_flags_and_ops_in_order() {
        let o = KeyOutcome {
            consumed: true,
            ops: vec![
                EditOp::Commit("你好".into()),
                EditOp::SetComposition {
                    text: "ni'hao".into(),
                    caret: 6,
                },
                EditOp::DeleteBackward { count: 2 },
                EditOp::ReplaceBackward {
                    count: 1,
                    text: ".".into(),
                },
                EditOp::MoveCursor { delta: -1 },
            ],
            hint: None,
            mode_changed: true,
        };
        let us = US.to_string();
        assert_eq!(
            outcome(&o),
            vec![
                "cm".to_string(),
                "C你好".to_string(),
                format!("S6{us}ni'hao"),
                "D2".to_string(),
                format!("R1{us}."),
                "M-1".to_string(),
            ]
        );
    }

    #[test]
    fn passthrough_has_empty_flags_and_no_ops() {
        assert_eq!(outcome(&KeyOutcome::passthrough()), vec![String::new()]);
    }

    #[test]
    fn caret_converts_bytes_to_utf16_units() {
        // 「你」3 字节 / 1 码元；😀 4 字节 / 2 码元
        assert_eq!(utf8_to_utf16_offset("你a", 3), 1);
        assert_eq!(utf8_to_utf16_offset("😀a", 4), 2);
        assert_eq!(utf8_to_utf16_offset("你", 1), 1, "不在字符边界取末尾");
        assert_eq!(utf8_to_utf16_offset("ab", 99), 2);
    }

    #[test]
    fn join_puts_separator_between_fields() {
        assert_eq!(join(&["a", "b", "c"]), "a\u{1F}b\u{1F}c");
        assert_eq!(join(&[]), "");
    }
}
