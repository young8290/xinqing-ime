//! 九宫格拼音的移动端门面（`docs/design/xsime-feature-port-plan.md` §2.6）。
//!
//! 切分逻辑在 `moodflow_engine::pinyin::t9`；这里只把结果整理成宿主好用的字符串形状：
//! 一种切分 = 用 `'` 连起来的全拼串（`ni'hao`），可以直接经
//! [`crate::MobileCore::replace_composition`] 交给拼音引擎出候选。
//!
//! 输入串的形状：数字 `2`–`9`、用户在「选拼音」行钉住的字母、显式分隔符 `'`，如 `ni'426`。

use moodflow_engine::pinyin::t9 as engine_t9;

/// 整串的可能切分，最多 `limit` 种；段数少的在前。
pub fn segmentations(input: &str, limit: usize) -> Vec<String> {
    engine_t9::segmentations(input, limit)
        .into_iter()
        .map(|seg| seg.join("'"))
        .collect()
}

/// 「选拼音」行：输入开头能匹配的音节（长的在前），用户点选后钉进输入串。
pub fn leading_syllables(input: &str) -> Vec<String> {
    engine_t9::leading_syllables(input)
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// 把输入开头 `syllable.len()` 个位置钉成该音节的字母，后面补一个分隔符。
///
/// 返回 `None` 表示该音节不匹配输入开头（宿主手里的「选拼音」行已过期）。
pub fn pin_leading(input: &str, syllable: &str) -> Option<String> {
    if !leading_syllables(input).iter().any(|s| s == syllable) {
        return None;
    }
    let rest = input.get(syllable.len()..).unwrap_or("");
    let rest = rest.strip_prefix('\'').unwrap_or(rest);
    if rest.is_empty() {
        Some(syllable.to_owned())
    } else {
        Some(format!("{syllable}'{rest}"))
    }
}

/// 字母所在的数字键（键面标注用）。
pub fn letter_digit(c: char) -> Option<char> {
    engine_t9::letter_digit(c).map(char::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nihao_is_among_segmentations() {
        let segs = segmentations("64426", 20);
        assert!(segs.iter().any(|s| s == "ni'hao"), "{segs:?}");
    }

    #[test]
    fn pinning_leading_syllable_constrains_rest() {
        let pinned = pin_leading("64426", "ni").unwrap();
        assert_eq!(pinned, "ni'426");
        let segs = segmentations(&pinned, 20);
        assert!(segs.iter().all(|s| s.starts_with("ni'")), "{segs:?}");
        assert!(segs.iter().any(|s| s == "ni'hao"), "{segs:?}");
    }

    #[test]
    fn stale_pin_is_rejected() {
        assert_eq!(pin_leading("64426", "zhang"), None);
    }

    #[test]
    fn letter_digit_matches_keypad() {
        assert_eq!(letter_digit('h'), Some('4'));
        assert_eq!(letter_digit('1'), None);
    }
}
