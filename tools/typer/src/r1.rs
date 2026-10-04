//! R1 打错字判定的离线复刻，只用来自检生成的脚本：标了 `typo` 的键都应命中，其余都不应命中。
//!
//! 规则与常量照抄 `xinqing_hub/core/src/domain/features/typo.rs`（FR-STA-04），那边改了这里要跟着改。
//! 不直接依赖 Hub core，是为了让 typer 保持轻量、能单独在测试机上编译。

use crate::script::{Key, Step};

pub const BACKSPACE_WITHIN_MS: u64 = 600;
pub const RETYPE_WITHIN_MS: u64 = 800;
pub const DEDUP_MS: u64 = 1_000;

const ROWS: [&[u8]; 3] = [b"qwertyuiop", b"asdfghjkl", b"zxcvbnm"];
const ROW_OFFSET_HALF: [i32; 3] = [0, 1, 3];

fn pos(c: u8) -> Option<(i32, i32)> {
    let c = c.to_ascii_lowercase();
    ROWS.iter().enumerate().find_map(|(r, row)| {
        row.iter()
            .position(|&k| k == c)
            .map(|i| (r as i32, i as i32 * 2 + ROW_OFFSET_HALF[r]))
    })
}

/// QWERTY 相邻（同行左右，或上下两行中水平距离不超过一个键宽，含对角）
pub fn adjacent(a: u8, b: u8) -> bool {
    let (Some((ra, xa)), Some((rb, xb))) = (pos(a), pos(b)) else {
        return false;
    };
    if a.eq_ignore_ascii_case(&b) {
        return false;
    }
    match (ra - rb).abs() {
        0 => (xa - xb).abs() == 2,
        1 => (xa - xb).abs() <= 2,
        _ => false,
    }
}

/// `c` 的全部相邻字母（小写）
pub fn neighbors(c: u8) -> Vec<u8> {
    (b'a'..=b'z').filter(|&o| adjacent(c, o)).collect()
}

/// 按计划时刻（毫秒）回放脚本，返回 R1 会命中的键的下标
pub fn hits(steps: &[Step], times: &[u64]) -> Vec<usize> {
    let mut last_letter: Option<(u64, u8)> = None;
    let mut backspace_after: Option<(u64, u8)> = None;
    let mut last_fire: Option<u64> = None;
    let mut out = Vec::new();
    for (i, (s, &ts)) in steps.iter().zip(times).enumerate() {
        match s.key {
            Key::Backspace => {
                backspace_after = match last_letter {
                    Some((t, x)) if ts.saturating_sub(t) <= BACKSPACE_WITHIN_MS => Some((ts, x)),
                    _ => None,
                };
                last_letter = None;
            }
            Key::Letter(y) => {
                let hit = match backspace_after.take() {
                    Some((tb, x)) => ts.saturating_sub(tb) <= RETYPE_WITHIN_MS && adjacent(x, y),
                    None => false,
                };
                last_letter = Some((ts, y));
                if hit && last_fire.is_none_or(|f| ts.saturating_sub(f) >= DEDUP_MS) {
                    last_fire = Some(ts);
                    out.push(i);
                }
            }
            _ => {
                last_letter = None;
                backspace_after = None;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacency_matches_hub_table() {
        // 与 Hub typo.rs 的 adjacency_table 测试同一组样例
        assert!(adjacent(b's', b'd'));
        assert!(adjacent(b's', b'w'));
        assert!(adjacent(b's', b'e'), "对角相邻");
        assert!(adjacent(b's', b'z'));
        assert!(adjacent(b's', b'x'));
        assert!(!adjacent(b's', b's'));
        assert!(!adjacent(b's', b'k'));
        assert!(!adjacent(b'q', b'z'));
        assert!(adjacent(b'g', b'b'));
        assert!(!adjacent(b'q', b'd'));
        assert_eq!(neighbors(b'q'), b"aw".to_vec());
    }
}
