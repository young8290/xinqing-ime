//! 节奏脚本：每行一键，`<距上一键的毫秒数> <键> [标注]`，`#` 起到行尾是注释。
//!
//! 键：`a`–`z`、`0`–`9`、`space`、`bs`（退格）、`enter`、`esc`、`tab`、`comma`、`period`。
//! 标注是给评测对答案用的，发键时原样写进日志：`typo` 表示这一键应触发 R1，
//! `bs_*` 表示一次不该触发 R1 的退格修改（见 gen.rs）。

use std::fmt;

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// 小写 ASCII 字母
    Letter(u8),
    /// ASCII 数字
    Digit(u8),
    Space,
    Backspace,
    Enter,
    Esc,
    Tab,
    Comma,
    Period,
}

impl Key {
    pub fn parse(s: &str) -> Option<Key> {
        let b = s.as_bytes();
        Some(match s {
            "space" => Key::Space,
            "bs" | "backspace" => Key::Backspace,
            "enter" => Key::Enter,
            "esc" => Key::Esc,
            "tab" => Key::Tab,
            "comma" => Key::Comma,
            "period" => Key::Period,
            _ if b.len() == 1 && b[0].is_ascii_lowercase() => Key::Letter(b[0]),
            _ if b.len() == 1 && b[0].is_ascii_digit() => Key::Digit(b[0]),
            _ => return None,
        })
    }

    /// Windows 虚拟键码（字母、数字与 ASCII 大写字符同值）
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn vk(self) -> u16 {
        match self {
            Key::Letter(c) => c.to_ascii_uppercase() as u16,
            Key::Digit(c) => c as u16,
            Key::Space => 0x20,
            Key::Backspace => 0x08,
            Key::Enter => 0x0D,
            Key::Esc => 0x1B,
            Key::Tab => 0x09,
            Key::Comma => 0xBC,  // VK_OEM_COMMA
            Key::Period => 0xBE, // VK_OEM_PERIOD
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Letter(c) | Key::Digit(c) => write!(f, "{}", *c as char),
            Key::Space => f.write_str("space"),
            Key::Backspace => f.write_str("bs"),
            Key::Enter => f.write_str("enter"),
            Key::Esc => f.write_str("esc"),
            Key::Tab => f.write_str("tab"),
            Key::Comma => f.write_str("comma"),
            Key::Period => f.write_str("period"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// 距上一键（第一键：距开始）的毫秒数
    pub wait_ms: u32,
    pub key: Key,
    pub label: Option<String>,
}

pub fn parse(text: &str) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let ln = n + 1;
        let mut it = line.split_whitespace();
        let wait_ms: u32 = it
            .next()
            .unwrap_or_default()
            .parse()
            .with_context(|| format!("第 {ln} 行：间隔不是非负整数毫秒"))?;
        let Some(tok) = it.next() else {
            bail!("第 {ln} 行：缺少键");
        };
        let key = Key::parse(tok).with_context(|| format!("第 {ln} 行：不认识的键 `{tok}`"))?;
        let label = match it.next() {
            Some(l) if l.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') => Some(l.to_owned()),
            Some(l) => bail!("第 {ln} 行：标注 `{l}` 只能用小写字母和下划线"),
            None => None,
        };
        if it.next().is_some() {
            bail!("第 {ln} 行：多余的内容");
        }
        steps.push(Step {
            wait_ms,
            key,
            label,
        });
    }
    Ok(steps)
}

pub fn format(header: &str, steps: &[Step]) -> String {
    let mut out = String::new();
    for l in header.lines() {
        out.push_str("# ");
        out.push_str(l);
        out.push('\n');
    }
    for s in steps {
        out.push_str(&format!("{} {}", s.wait_ms, s.key));
        if let Some(l) = &s.label {
            out.push(' ');
            out.push_str(l);
        }
        out.push('\n');
    }
    out
}

/// 每一键相对开始的计划时刻（毫秒）
pub fn timeline(steps: &[Step]) -> Vec<u64> {
    let mut t = 0u64;
    steps
        .iter()
        .map(|s| {
            t += u64::from(s.wait_ms);
            t
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let text = "# 示例\n0 n\n125 i  # 行尾注释\n\n125 bs\n300 h typo\n125 space\n";
        let steps = parse(text).unwrap();
        assert_eq!(steps.len(), 5);
        assert_eq!(steps[2].key, Key::Backspace);
        assert_eq!(steps[3].label.as_deref(), Some("typo"));
        assert_eq!(timeline(&steps), vec![0, 125, 250, 550, 675]);
        assert_eq!(parse(&format("示例", &steps)).unwrap(), steps);
    }

    #[test]
    fn rejects_bad_lines() {
        assert!(parse("x a").is_err());
        assert!(parse("10").is_err());
        assert!(parse("10 shift").is_err());
        assert!(parse("10 a Typo").is_err());
        assert!(parse("10 a typo extra").is_err());
    }

    #[test]
    fn virtual_keys() {
        assert_eq!(Key::Letter(b'q').vk(), 0x51);
        assert_eq!(Key::Digit(b'1').vk(), 0x31);
        assert_eq!(Key::parse("bs"), Some(Key::Backspace));
    }
}
