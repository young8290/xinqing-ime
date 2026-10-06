//! 情绪日记的纯函数部分（C-10 第三部分，05 FR-DIA-01～03、08 P-DIARY）：对话要点、提示词拼装、草稿校验、
//! 保存时的来源（`ai_draft` / `ai_edited` / `manual`）。生成、保存与危机识别在 [`crate::diary`]。实现说明见 docs/adr/0031。

use serde::Deserialize;

use super::comfort::ComfortPrompt;
use super::validate::{self, BannedWords, Scene};
use crate::infra::templates::{TemplateDirs, TemplateError, read_toml};

/// 草稿 ≤ 300 字（08 第 5 节 V3），提示词要 100–200 字；太短的多半是模型没按要求写，按没通过处理。
pub const DRAFT_MIN_CHARS: usize = 30;
pub const DRAFT_MAX_CHARS: usize = 300;
/// 一篇日记最多这么多字（手写的上限，防止误粘贴大段文字；ADR 0031 第 5 条）。
pub const MAX_CHARS: usize = 5000;
/// 对话要点：本次对话里用户最近的几条消息，每条截到这么多字，合计不超过 [`DIGEST_CHARS`]。
pub const DIGEST_MESSAGES: usize = 10;
pub const DIGEST_ITEM_CHARS: usize = 60;
pub const DIGEST_CHARS: usize = 500;

/// 日记的来源（FR-DIA-02），即 `diary.source`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    AiDraft,
    AiEdited,
    Manual,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::AiDraft => "ai_draft",
            Source::AiEdited => "ai_edited",
            Source::Manual => "manual",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "ai_draft" => Source::AiDraft,
            "ai_edited" => Source::AiEdited,
            _ => Source::Manual,
        }
    }

    /// 新写的一篇：从草稿来的看改没改过（不计首尾空白）；`draft` 为 `Some(None)` 表示是从草稿来的，
    /// 但 Hub 已经不记得草稿原文（重启过或过期），按“修改过”标，宁可多标 AI（C-LAW-05）。
    pub fn of_new(draft: Option<Option<&str>>, content: &str) -> Self {
        match draft {
            None => Source::Manual,
            Some(Some(d)) if d.trim() == content.trim() => Source::AiDraft,
            Some(_) => Source::AiEdited,
        }
    }

    /// 改已有的一篇：未修改的草稿改了内容就变成“修改过”，其余不变。
    pub fn after_edit(self, old: &str, new: &str) -> Self {
        match self {
            Source::AiDraft if old.trim() != new.trim() => Source::AiEdited,
            s => s,
        }
    }
}

/// 本次对话的要点：用户最近 [`DIGEST_MESSAGES`] 条消息，每条截短后用“；”连起来，从旧到新。
/// 只取用户自己说的话，不取晴晴的回复（草稿要像用户自己写的）。一条都没有时为 `None`。
pub fn digest(user_messages: &[String]) -> Option<String> {
    let mut items: Vec<String> = user_messages
        .iter()
        .rev()
        .map(|m| m.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|m| !m.is_empty())
        .take(DIGEST_MESSAGES)
        .map(|m| {
            if m.chars().count() > DIGEST_ITEM_CHARS {
                let cut: String = m.chars().take(DIGEST_ITEM_CHARS).collect();
                format!("{cut}…")
            } else {
                m
            }
        })
        .collect();
    // 超出总长时丢掉最早的
    while items.iter().map(|i| i.chars().count() + 1).sum::<usize>() > DIGEST_CHARS
        && items.len() > 1
    {
        items.pop();
    }
    items.reverse();
    (!items.is_empty()).then(|| items.join("；"))
}

/// P-DIARY（`prompts/diary.md`）。
#[derive(Debug, Clone)]
pub struct DiaryPrompt {
    pub version: u32,
    body: String,
}

/// 摘要或对话要点没有时填的字。
const NONE: &str = "（无）";

impl DiaryPrompt {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        const FILE: &str = "prompts/diary.md";
        dirs.load_with_fallback(FILE, |path| {
            let text = std::fs::read_to_string(path).map_err(|source| TemplateError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            let version = text
                .lines()
                .next()
                .and_then(|l| l.trim().strip_prefix("<!-- version:"))
                .and_then(|l| l.strip_suffix("-->"))
                .and_then(|l| l.trim().parse::<u32>().ok());
            let p = ComfortPrompt::parse(&text);
            if version.is_none_or(|v| v == 0)
                || p.body().trim().is_empty()
                || ["{summary}", "{chat_digest}"]
                    .iter()
                    .any(|key| !p.body().contains(key))
            {
                return Err(TemplateError::Invalid {
                    file: FILE,
                    reason: "缺少正整数版本号、正文或必需占位符",
                });
            }
            Ok(Self {
                version: p.version,
                body: p.body().to_string(),
            })
        })
    }

    pub fn ver(&self) -> String {
        format!("P-DIARY v{}", self.version)
    }

    pub fn render(&self, summary: Option<&str>, digest: Option<&str>) -> String {
        self.body
            .replace("{summary}", summary.unwrap_or(NONE))
            .replace("{chat_digest}", digest.unwrap_or(NONE))
    }
}

/// 草稿为什么没通过（不含文字）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    Length,
    Banned,
    Language,
}

impl Reject {
    pub fn as_str(self) -> &'static str {
        match self {
            Reject::Length => "长度",
            Reject::Banned => "禁用词",
            Reject::Language => "语言",
        }
    }
}

/// 校验大模型写的草稿（08 第 5 节 V3 / V4 / V7），通过时返回去掉首尾空白的正文。
/// 草稿是用户的日记，V4 用通用口径（`chat_allow` 不放行，FR-DIA-01“不得出现诊断性措辞”）。
pub fn check_draft(text: &str, banned: &BannedWords) -> Result<String, Reject> {
    let text = text.trim();
    let n = text.chars().filter(|c| !c.is_whitespace()).count();
    if !(DRAFT_MIN_CHARS..=DRAFT_MAX_CHARS).contains(&n) {
        return Err(Reject::Length);
    }
    if banned.find(text, Scene::Other).is_some() {
        return Err(Reject::Banned);
    }
    if !validate::chinese_ratio_ok(text) {
        return Err(Reject::Language);
    }
    Ok(text.to_string())
}

/// 日记用到的固定文案（`ui_copy.toml` 的 `[diary]`）。
#[derive(Debug, Clone)]
pub struct DiaryCopy {
    /// 空白模板（FR-DIA-01 失败处理）
    pub blank: String,
    /// 日记里触发求助卡片时新开的对话的标题
    pub safety_session: String,
}

#[derive(Debug, Deserialize)]
struct RawCopy {
    diary: RawDiaryCopy,
}

#[derive(Debug, Deserialize)]
struct RawDiaryCopy {
    safety_session: String,
}

impl DiaryCopy {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        let raw: RawCopy = read_toml(&dirs.factory_path("ui_copy.toml"))?;
        // 整份用户文件只提取非安全的空白模板；求助会话标题仍来自出厂文件。
        let banned = BannedWords::load(dirs)?;
        let blank = dirs.load_with_fallback("ui_copy.toml", |path| {
            #[derive(Deserialize)]
            struct BlankFile { version: u32, diary: BlankCopy }
            #[derive(Deserialize)]
            struct BlankCopy { blank: String }
            let copy: BlankFile = read_toml(path)?;
            let blank = copy.diary.blank.trim();
            if copy.version == 0 || blank.is_empty() || banned.find(blank, Scene::Other).is_some() {
                return Err(TemplateError::Invalid { file: "ui_copy.toml", reason: "日记空白模板版本、正文或禁用词校验失败" });
            }
            Ok(blank.to_string())
        })?;
        Ok(Self {
            blank,
            safety_session: raw.diary.safety_session.trim().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn dirs() -> TemplateDirs {
        TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        )
    }

    #[test]
    fn source_follows_fr_dia_02() {
        assert_eq!(Source::of_new(None, "手写的"), Source::Manual);
        assert_eq!(
            Source::of_new(Some(Some("草稿")), " 草稿\n"),
            Source::AiDraft
        );
        assert_eq!(
            Source::of_new(Some(Some("草稿")), "草稿，改了"),
            Source::AiEdited
        );
        assert_eq!(
            Source::of_new(Some(None), "草稿"),
            Source::AiEdited,
            "不记得草稿时按修改过"
        );
        assert_eq!(Source::AiDraft.after_edit("a", "a "), Source::AiDraft);
        assert_eq!(Source::AiDraft.after_edit("a", "b"), Source::AiEdited);
        assert_eq!(Source::Manual.after_edit("a", "b"), Source::Manual);
        assert_eq!(Source::AiEdited.after_edit("a", "a"), Source::AiEdited);
        for s in [Source::AiDraft, Source::AiEdited, Source::Manual] {
            assert_eq!(Source::parse(s.as_str()), s);
        }
    }

    #[test]
    fn digest_keeps_recent_user_words() {
        assert_eq!(digest(&[]), None);
        assert_eq!(digest(&["  ".into()]), None);
        let msgs: Vec<String> = (1..=12).map(|i| format!("第{i}句")).collect();
        let d = digest(&msgs).unwrap();
        assert!(d.starts_with("第3句；"), "只取最近 10 条，从旧到新：{d}");
        assert!(d.ends_with("第12句"));
        let long = "很".repeat(100);
        let d = digest(&[long]).unwrap();
        assert_eq!(d.chars().count(), DIGEST_ITEM_CHARS + 1);
        let many: Vec<String> = (0..10).map(|_| "长".repeat(80)).collect();
        assert!(digest(&many).unwrap().chars().count() <= DIGEST_CHARS);
    }

    #[test]
    fn prompt_fills_summary_and_digest() {
        let p = DiaryPrompt::load(&dirs()).unwrap();
        assert_eq!(p.ver(), "P-DIARY v1");
        let s = p.render(Some("上午平稳"), None);
        assert!(s.contains("今日状态摘要：上午平稳"));
        assert!(s.contains("对话要点：（无）"));
        assert!(!s.contains('{'));
    }

    #[test]
    fn drafts_are_checked() {
        let banned = BannedWords::load(&dirs()).unwrap();
        let good = "今天上午还挺顺的，下午开始有点累，晚上和晴晴聊了几句，说了说考试的事，心里松快了一些。明天想早点起来，先把最难的那一科看一遍。";
        assert_eq!(
            check_draft(&format!("\n{good}\n"), &banned),
            Ok(good.to_string())
        );
        assert_eq!(check_draft("太短", &banned), Err(Reject::Length));
        assert_eq!(check_draft(&"长".repeat(301), &banned), Err(Reject::Length));
        let diag = good.replace("有点累", "可能是抑郁");
        assert_eq!(check_draft(&diag, &banned), Err(Reject::Banned));
        let english =
            "Today was fine, I talked with Qingqing about the exam and felt a bit better.";
        assert_eq!(check_draft(english, &banned), Err(Reject::Language));
    }

    #[test]
    fn copy_loads_and_passes_banned_words() {
        let dirs = dirs();
        let c = DiaryCopy::load(&dirs).unwrap();
        assert!(c.blank.starts_with("今天发生了"));
        assert!(c.blank.contains("明天想"));
        let banned = BannedWords::load(&dirs).unwrap();
        for t in [&c.blank, &c.safety_session] {
            assert_eq!(banned.find(t, Scene::Other), None);
        }
    }
}
