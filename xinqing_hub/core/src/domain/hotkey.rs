//! Hub 的全局快捷键（07 FR-ENT-04，docs/adr/0036）：设置键 `hotkey.*` 的格式与校验。注册在外壳（`src-tauri/src/hotkeys.rs`）。
//!
//! 写法与输入法配置的 `xinqing.pause_hotkey` 一致：小写、`+` 连接，修饰键在前，如 `ctrl+alt+q`；空串表示不设。
//! 只收“至少一个 Ctrl / Alt / Win + 一个字母、数字或 F1–F24”：单独的字母或只加 Shift 会吞掉正常打字。

/// 一组快捷键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// 小写：`a`–`z`、`0`–`9`、`f1`–`f24`
    pub key: String,
}

impl Hotkey {
    /// 解析并校验；不合法时为 `None`。修饰键顺序随意、大小写不敏感，不许重复。
    pub fn parse(s: &str) -> Option<Self> {
        let mut hk = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            key: String::new(),
        };
        let parts: Vec<String> = s
            .split('+')
            .map(|p| p.trim().to_ascii_lowercase())
            .collect();
        let (key, mods) = parts.split_last()?;
        for m in mods {
            let flag = match m.as_str() {
                "ctrl" | "control" => &mut hk.ctrl,
                "alt" => &mut hk.alt,
                "shift" => &mut hk.shift,
                "win" | "super" | "meta" => &mut hk.win,
                _ => return None,
            };
            if *flag {
                return None;
            }
            *flag = true;
        }
        if !(hk.ctrl || hk.alt || hk.win) || !valid_key(key) {
            return None;
        }
        hk.key = key.clone();
        Some(hk)
    }

    /// 规范写法：`ctrl+alt+shift+win+键`。
    pub fn normalized(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for (on, name) in [
            (self.ctrl, "ctrl"),
            (self.alt, "alt"),
            (self.shift, "shift"),
            (self.win, "win"),
        ] {
            if on {
                parts.push(name);
            }
        }
        parts.push(&self.key);
        parts.join("+")
    }
}

fn valid_key(k: &str) -> bool {
    let b = k.as_bytes();
    match b {
        [c] => c.is_ascii_lowercase() || c.is_ascii_digit(),
        [b'f', rest @ ..] if !rest.is_empty() && rest.len() <= 2 => std::str::from_utf8(rest)
            .ok()
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (1..=24).contains(&n) && !rest.starts_with(b"0")),
        _ => false,
    }
}

/// 设置值是否可以存：空串（不设）或合法的快捷键。
pub fn accepts(s: &str) -> bool {
    s.is_empty() || Hotkey::parse(s).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifiers_in_any_order_and_normalizes() {
        let hk = Hotkey::parse("Alt+CTRL+Q").unwrap();
        assert!(hk.ctrl && hk.alt && !hk.shift && !hk.win);
        assert_eq!(hk.normalized(), "ctrl+alt+q");
        assert_eq!(
            Hotkey::parse("win+shift+f12").unwrap().normalized(),
            "shift+win+f12"
        );
        assert_eq!(Hotkey::parse("ctrl+alt+7").unwrap().key, "7");
    }

    #[test]
    fn rejects_shortcuts_that_would_eat_typing_or_are_malformed() {
        for bad in [
            "q",
            "shift+q",
            "ctrl+alt",
            "ctrl+ctrl+q",
            "ctrl+alt+qq",
            "ctrl+alt+f0",
            "ctrl+alt+f25",
            "ctrl+alt+f01",
            "hyper+q",
            "ctrl+alt+;",
            "",
        ] {
            assert!(Hotkey::parse(bad).is_none(), "{bad}");
        }
        assert!(accepts(""), "空串表示不设");
        assert!(accepts("ctrl+alt+w"));
        assert!(!accepts("w"));
    }
}
