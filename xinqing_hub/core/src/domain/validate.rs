//! 输出校验器（08 第 5 节），纯函数。当前实现 V1、V3 长度计数、V4 禁用词、V5 重复、V7 语言。
//! 与 `tools/check_templates.py` 使用同一份 `banned_words.toml`，规则保持一致。

use std::collections::HashSet;

use regex::Regex;
use serde::Deserialize;

use crate::infra::templates::{TemplateDirs, TemplateError, read_toml};

/// 输出场景：对话场景放行 `chat_allow` 中的词（FR-CHT-03）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    Chat,
    Other,
}

#[derive(Debug, Deserialize)]
struct RawBanned {
    version: u32,
    #[serde(default)]
    diagnosis: Vec<String>,
    #[serde(default)]
    surveillance: Vec<String>,
    #[serde(default)]
    preachy: Vec<String>,
    #[serde(default)]
    dependency: Vec<String>,
    #[serde(default)]
    routine: Vec<String>,
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    chat_allow: Vec<String>,
    #[serde(default)]
    abuse: Vec<String>,
    #[serde(default)]
    exempt_copy_ids: Vec<String>,
}

#[derive(Debug)]
pub struct BannedWords {
    pub version: u32,
    words: Vec<String>,
    patterns: Vec<Regex>,
    chat_allow: HashSet<String>,
    exempt: HashSet<String>,
    /// 辱骂词（第九类），只给温柔改写的 V8 用（ADR 0028）
    abuse: Vec<String>,
}

const FILE: &str = "banned_words.toml";

impl BannedWords {
    /// 只读出厂版本（15 第 5 节）。
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        let raw: RawBanned = read_toml(&dirs.factory_path(FILE))?;
        let patterns = raw
            .patterns
            .iter()
            .map(|p| {
                Regex::new(p).map_err(|source| TemplateError::Regex {
                    file: FILE,
                    pattern: p.clone(),
                    source,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let words = [
            raw.diagnosis,
            raw.surveillance,
            raw.preachy,
            raw.dependency,
            raw.routine,
        ]
        .concat();
        // 词表条目和待查文字走同一套归一化，否则 `PTSD` 这类大写条目永远匹配不上转成小写的文字
        let words = words.iter().map(|w| normalize(w)).collect();
        Ok(Self {
            version: raw.version,
            words,
            patterns,
            chat_allow: raw.chat_allow.iter().map(|w| normalize(w)).collect(),
            exempt: raw.exempt_copy_ids.into_iter().collect(),
            abuse: raw.abuse.iter().map(|w| normalize(w)).collect(),
        })
    }

    /// 返回第一个命中的禁用词或正则；`None` 表示通过。
    /// 匹配前做全角转半角、去空格（banned_words.toml 末尾说明）。
    pub fn find(&self, text: &str, scene: Scene) -> Option<String> {
        let t = normalize(text);
        for w in &self.words {
            if scene == Scene::Chat && self.chat_allow.contains(w) {
                continue;
            }
            if t.contains(w.as_str()) {
                return Some(w.clone());
            }
        }
        self.patterns
            .iter()
            .find(|p| p.is_match(&t))
            .map(|p| p.as_str().to_string())
    }

    /// 只找 `source` 里没有的禁用词或正则：从用户原句里抽出来的字段（日程标题等）用，
    /// 用户自己说的词照常保留，模型带进来的才算命中（ADR 0024）。
    pub fn find_new(&self, text: &str, source: &str, scene: Scene) -> Option<String> {
        let (t, src) = (normalize(text), normalize(source));
        for w in &self.words {
            if scene == Scene::Chat && self.chat_allow.contains(w) {
                continue;
            }
            if t.contains(w.as_str()) && !src.contains(w.as_str()) {
                return Some(w.clone());
            }
        }
        self.patterns
            .iter()
            .find(|p| p.is_match(&t) && !p.is_match(&src))
            .map(|p| p.as_str().to_string())
    }

    /// V8：`source` 里没有、`text` 里新出现的辱骂词（温柔改写，ADR 0028）。
    pub fn find_new_abuse(&self, text: &str, source: &str) -> Option<String> {
        let (t, src) = (normalize(text), normalize(source));
        self.abuse
            .iter()
            .find(|w| t.contains(w.as_str()) && !src.contains(w.as_str()))
            .cloned()
    }

    /// 固定文案校验：`exempt_copy_ids` 中的键跳过（AI 输出永不豁免）。
    pub fn find_in_copy(&self, key: &str, text: &str) -> Option<String> {
        if self.exempt.contains(key) {
            return None;
        }
        self.find(text, Scene::Other)
    }
}

/// 全角 ASCII 转半角、去空白、英文转小写。
pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{3000}')
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// 汉字个数（V3 长度口径，与 check_templates.py 的 `[一-鿿]` 一致）。
pub fn han_count(s: &str) -> usize {
    s.chars()
        .filter(|c| ('\u{4E00}'..='\u{9FFF}').contains(c))
        .count()
}

/// V7：中文字符占可读字符（汉字 + 字母 + 数字）的比例 ≥ 60%。
pub fn chinese_ratio_ok(s: &str) -> bool {
    let readable = s.chars().filter(|c| c.is_alphanumeric()).count();
    readable == 0 || han_count(s) as f64 / readable as f64 >= 0.6
}

/// V1：去掉代码块标记，截取第一个 `{` 到最后一个 `}` 解析 JSON。
pub fn extract_json(s: &str) -> Option<serde_json::Value> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str(&s[start..=end]).ok()
}

fn bigrams(s: &str) -> HashSet<(char, char)> {
    let c: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    c.windows(2).map(|w| (w[0], w[1])).collect()
}

/// V5：字符二元组 Jaccard 相似度。
pub fn jaccard(a: &str, b: &str) -> f64 {
    let (x, y) = (bigrams(a), bigrams(b));
    let union = x.union(&y).count();
    if union == 0 {
        return 0.0;
    }
    x.intersection(&y).count() as f64 / union as f64
}

/// V5：与最近 10 条暖心话相似度都 < 0.6。
pub fn not_repetitive(text: &str, recent: &[String]) -> bool {
    recent.iter().rev().take(10).all(|r| jaccard(text, r) < 0.6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_fullwidth_and_spaces() {
        assert_eq!(normalize("ＡＢＣ　检 测到"), "abc检测到");
    }

    #[test]
    fn json_extraction() {
        let v = extract_json("```json\n{\"text\": \"慢慢来\"}\n```").unwrap();
        assert_eq!(v["text"], "慢慢来");
        assert!(extract_json("没有 json").is_none());
    }

    #[test]
    fn ratio_and_han() {
        assert_eq!(han_count("慢慢来, ok"), 3);
        assert!(chinese_ratio_ok("今天辛苦了，早点休息"));
        assert!(!chinese_ratio_ok("Take a rest 休息"));
    }

    #[test]
    fn jaccard_similarity() {
        assert_eq!(jaccard("慢慢来", "慢慢来"), 1.0);
        assert!(jaccard("想说的话不急着说完", "今天辛苦了") < 0.1);
        assert!(!not_repetitive(
            "慢慢来不着急",
            &["慢慢来不着急哦".to_string()]
        ));
    }
}
