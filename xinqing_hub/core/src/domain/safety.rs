//! 危机表达本地检测（FR-SAF-01，17 第 2.7 节）。
//!
//! 算法与 `eval/tools/check_crisis.py` 逐步一致，单元测试用 E-CRISIS 数据集和
//! `eval/datasets/e_crisis.golden.json`（由 Python 参考实现生成）对拍。
//! 纯函数，只读词表；结果只给出分数与命中类别，不返回匹配到的原文。

use std::collections::{BTreeMap, HashMap};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::infra::templates::{read_toml, TemplateDirs, TemplateError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Ideation,
    Method,
    Farewell,
}

#[derive(Debug, Deserialize)]
struct RawLexicon {
    version: u32,
    threshold: f64,
    threshold_after_dismiss: f64,
    negation_factor: f64,
    quote_factor: f64,
    negation: Vec<String>,
    #[serde(default)]
    self_ref: Vec<String>,
    quote: Vec<String>,
    entry: Vec<RawEntry>,
    #[serde(default)]
    benign: Vec<RawBenign>,
}

#[derive(Debug, Deserialize)]
struct RawEntry {
    category: Category,
    pattern: String,
    weight: f64,
}

#[derive(Debug, Deserialize)]
struct RawBenign {
    pattern: String,
}

#[derive(Debug)]
struct Entry {
    category: Category,
    re: Regex,
    weight: f64,
}

#[derive(Debug)]
pub struct CrisisLexicon {
    pub version: u32,
    pub threshold: f64,
    pub threshold_after_dismiss: f64,
    negation_factor: f64,
    quote_factor: f64,
    negation: Vec<String>,
    self_ref: Vec<String>,
    quote: Vec<String>,
    entries: Vec<Entry>,
    benign: Vec<Regex>,
    punct: Regex,
    t2s: HashMap<char, char>,
}

#[derive(Debug, Deserialize)]
struct RawT2s {
    map: HashMap<char, char>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LocalVerdict {
    pub score: f64,
    pub hit: bool,
    /// 各类别最高分（不含原文）。
    pub categories: BTreeMap<Category, f64>,
}

impl LocalVerdict {
    /// V6：对话输出是否命中“方法 / 手段”类条目。
    pub fn has_method(&self) -> bool {
        self.categories.contains_key(&Category::Method)
    }
}

const FILE: &str = "crisis_lexicon.toml";
const T2S_FILE: &str = "crisis_t2s.toml";

impl CrisisLexicon {
    /// 只读出厂版本（15 第 5 节：危机词表不允许用户覆盖）。
    /// 同时读取繁转简字表 `crisis_t2s.toml`（由 tools/gen_crisis_t2s.py 生成）。
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        let raw: RawLexicon = read_toml(&dirs.factory_path(FILE))?;
        let t2s: RawT2s = read_toml(&dirs.factory_path(T2S_FILE))?;
        Self::from_raw(raw, t2s.map)
    }

    fn from_raw(raw: RawLexicon, t2s: HashMap<char, char>) -> Result<Self, TemplateError> {
        let compile = |p: &str| {
            Regex::new(p).map_err(|source| TemplateError::Regex {
                file: FILE,
                pattern: p.to_string(),
                source,
            })
        };
        let entries = raw
            .entry
            .iter()
            .map(|e| {
                Ok(Entry {
                    category: e.category,
                    re: compile(&e.pattern)?,
                    weight: e.weight,
                })
            })
            .collect::<Result<Vec<_>, TemplateError>>()?;
        let benign = raw
            .benign
            .iter()
            .map(|b| compile(&b.pattern))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            version: raw.version,
            threshold: raw.threshold,
            threshold_after_dismiss: raw.threshold_after_dismiss,
            negation_factor: raw.negation_factor,
            quote_factor: raw.quote_factor,
            negation: raw.negation,
            self_ref: raw.self_ref,
            quote: raw.quote,
            entries,
            benign,
            punct: Regex::new(r"[\s，。！？、,.!?…~；;：:]").expect("固定正则"),
            t2s,
        })
    }

    /// 判定算法第 1 步：全角转半角、繁转简，再去空格和标点（与 Python 参考实现相同）。
    fn normalize(&self, text: &str) -> String {
        let t: String = text
            .chars()
            .map(|c| match c {
                '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
                '\u{3000}' => ' ',
                _ => c,
            })
            .map(|c| self.t2s.get(&c).copied().unwrap_or(c))
            .collect();
        self.punct.replace_all(&t, "").into_owned()
    }
}

/// 取 `t` 中字节位置 `end` 之前最多 `n` 个字符（Python `t[max(0, s-6):s]`）。
fn chars_before(t: &str, end: usize, n: usize) -> &str {
    let head = &t[..end];
    let start = head
        .char_indices()
        .rev()
        .nth(n - 1)
        .map(|(i, _)| i)
        .unwrap_or(0);
    &head[start..]
}

/// 本地判定。`threshold` 传 `lex.threshold`，用户点过“我说的不是这个意思”后传 `lex.threshold_after_dismiss`。
pub fn check_local(text: &str, lex: &CrisisLexicon, threshold: f64) -> LocalVerdict {
    let t = lex.normalize(text);
    let benign: Vec<(usize, usize)> = lex
        .benign
        .iter()
        .flat_map(|re| re.find_iter(&t).map(|m| (m.start(), m.end())))
        .collect();
    let quoted = lex.quote.iter().any(|q| t.contains(q.as_str()))
        && !lex.self_ref.iter().any(|r| t.contains(r.as_str()));
    let mut best: BTreeMap<Category, f64> = BTreeMap::new();
    for e in &lex.entries {
        for m in e.re.find_iter(&t) {
            let (s, en) = (m.start(), m.end());
            if benign.iter().any(|&(bs, be)| bs < en && s < be) {
                continue;
            }
            let mut w = e.weight;
            let before = chars_before(&t, s, 6);
            if lex.negation.iter().any(|n| before.contains(n.as_str())) {
                w *= lex.negation_factor;
            }
            if quoted {
                w *= lex.quote_factor;
            }
            let slot = best.entry(e.category).or_insert(0.0);
            if w > *slot {
                *slot = w;
            }
        }
    }
    let score: f64 = best.values().sum();
    LocalVerdict {
        score,
        hit: score >= threshold,
        categories: best,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chars_before_counts_characters_not_bytes() {
        let t = "我其实不会想死";
        let s = t.find("想死").unwrap();
        assert_eq!(chars_before(t, s, 6), "我其实不会");
        assert_eq!(
            chars_before("一二三四五六七八", "七".len() * 6, 6),
            "一二三四五六"
        );
        assert_eq!(chars_before("abc", 0, 6), "");
    }
}
