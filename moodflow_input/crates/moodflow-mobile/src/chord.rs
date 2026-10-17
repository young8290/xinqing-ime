//! 并击的移动端门面：键位方案 JSON → 结算器（`docs/design/xsime-feature-port-plan.md` §2.3）。
//!
//! 首批只接**物理键盘**并击：宿主把 keydown / keyup 连同时间戳交进来，结算出的拼音片段再经
//! [`crate::MobileCore::feed_text`] 送进拼音引擎（不另起候选路径）。触屏并击是 P3 末尾的事。
//!
//! 键名与 `moodflow-keys` 一致（`"a"`、`"semicolon"`…），宿主负责把平台键码换成键名。

use moodflow_chord::{ChordEvent, Keymap, OutputKind, Settler};

/// 一次结算的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordOutput {
    /// 送进拼音引擎的串。`fragment = true` 表示声母/韵母片段（与相邻输入拼合）。
    Emit { pinyin: String, fragment: bool },
    /// 这组键没有映射：不出字。
    NoMatch,
}

/// 已应用的并击方案 + 结算器。
#[derive(Debug, Clone)]
pub struct MobileChord {
    settler: Settler,
    name: String,
}

impl MobileChord {
    /// 解析并按**应用级**校验（空映射表可存草稿，不可应用）。错误串不含方案内容。
    pub fn from_json(json: &str) -> Result<Self, String> {
        let map = Keymap::from_json(json).map_err(|e| e.to_string())?;
        map.validate_for_apply().map_err(|e| e.to_string())?;
        Ok(Self {
            settler: Settler::new(&map),
            name: map.name,
        })
    }

    /// 草稿级校验（设置页保存草稿时用）：`Ok` 不代表可以应用。
    pub fn validate_draft(json: &str) -> Result<(), String> {
        Keymap::from_json(json)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// 该键是否属于并击键区；不属于的键宿主照常走普通路径。
    pub fn is_chord_key(&self, key: &str) -> bool {
        self.settler.is_chord_key(key)
    }

    pub fn is_busy(&self) -> bool {
        self.settler.is_busy()
    }

    pub fn key_down(&mut self, key: &str, now_ms: u64) -> Option<ChordOutput> {
        self.settler.key_down(key, now_ms).map(convert)
    }

    pub fn key_up(&mut self, key: &str, now_ms: u64) -> Option<ChordOutput> {
        self.settler.key_up(key, now_ms).map(convert)
    }

    /// 时间推进：等另一只手超时则单独结算。宿主在定时器或下一次按键前调用。
    pub fn tick(&mut self, now_ms: u64) -> Option<ChordOutput> {
        self.settler.tick(now_ms).map(convert)
    }

    /// 换焦点、切方案、取消：丢弃未结算的输入。
    pub fn reset(&mut self) {
        self.settler.reset();
    }
}

fn convert(e: ChordEvent) -> ChordOutput {
    match e {
        ChordEvent::Emit { output, kind } => ChordOutput::Emit {
            pinyin: output,
            fragment: kind == OutputKind::Fragment,
        },
        ChordEvent::NoMatch { .. } => ChordOutput::NoMatch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP: &str = r#"{
        "version": 1,
        "name": "测试",
        "left_keys": ["a", "s"],
        "right_keys": ["j", "k"],
        "interval_ms": 300,
        "mappings": [
            {"keys": ["s", "j"], "output": "zhang", "kind": "syllable"},
            {"keys": ["a"], "output": "zh", "kind": "fragment"}
        ]
    }"#;

    #[test]
    fn simultaneous_chord_settles_on_last_release() {
        let mut c = MobileChord::from_json(MAP).unwrap();
        assert!(c.is_chord_key("s"));
        assert!(!c.is_chord_key("q"));
        assert_eq!(c.key_down("s", 0), None);
        assert_eq!(c.key_down("j", 10), None);
        assert_eq!(c.key_up("s", 20), None, "只松开一只手不出字");
        assert_eq!(
            c.key_up("j", 30),
            Some(ChordOutput::Emit {
                pinyin: "zhang".into(),
                fragment: false
            })
        );
    }

    #[test]
    fn fragment_is_flagged() {
        let mut c = MobileChord::from_json(MAP).unwrap();
        c.key_down("a", 0);
        assert_eq!(
            c.key_up("a", 5),
            Some(ChordOutput::Emit {
                pinyin: "zh".into(),
                fragment: true
            })
        );
    }

    #[test]
    fn empty_mapping_is_draft_only() {
        let draft =
            r#"{"version":1,"name":"草稿","left_keys":["a"],"right_keys":["j"],"mappings":[]}"#;
        assert!(MobileChord::validate_draft(draft).is_ok());
        assert!(MobileChord::from_json(draft).is_err());
    }
}
