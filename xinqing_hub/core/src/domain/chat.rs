//! AI 对话的纯函数部分（05 FR-CHT-02～05、FR-SAF-01/03，08 第 4、5 节）：会话标题、何时开新会话、
//! 提示词拼装、上下文截取、回复校验（V3 / V4 / V6）、今日状态摘要文字、危机双通道的合并。
//! 服务（流式、写库、事件）在 `crate::chat`，实现上的取舍见 ADR 0018。

use chrono::{DateTime, Local, Timelike};
use serde::{Deserialize, Serialize};
use xqp::MoodState;

use crate::domain::comfort::Style;
use crate::domain::safety::{self, CrisisLexicon};
use crate::domain::validate::{BannedWords, Scene};
use crate::infra::templates::{TemplateDirs, TemplateError, read_toml};

/// 距上一条消息超过 6 小时就开新会话（FR-CHT-02 第 1 条）。
pub const NEW_SESSION_GAP_MS: i64 = 6 * 60 * 60 * 1000;
/// 会话标题取第一条用户消息的前 12 个字（FR-CHT-02 第 2 条）。
pub const TITLE_CHARS: usize = 12;
/// 单条消息最多 2000 字（FR-CHT-09）。
pub const MAX_INPUT_CHARS: usize = 2000;
/// 对话单条回复 ≤ 600 字，超出截断并加“……”（08 第 5 节 V3）。
pub const MAX_REPLY_CHARS: usize = 600;
/// 最近对话：当前会话最近 20 轮（FR-CHT-05）。
pub const CONTEXT_TURNS: usize = 20;
/// 最近对话约 3000 token，超出时截掉最早的（FR-CHT-05）。中文按 1 字 ≈ 1 token 估算，偏保守。
pub const CONTEXT_TOKENS: usize = 3000;
/// 长期记忆：最近 10 条、合计 300 字（FR-CHT-05）。
pub const MEMORY_ITEMS: usize = 10;
pub const MEMORY_CHARS: usize = 300;
/// “晴晴记住的事”最多 50 条，每条 ≤ 100 字（FR-CHT-07 第 2 条；100 字也是 `memory` 表的 CHECK）。
pub const MEMORY_MAX: usize = 50;
pub const MEMORY_ENTRY_CHARS: usize = 100;
/// 今日状态摘要上限 200 字（FR-CHT-05）。
pub const SUMMARY_CHARS: usize = 200;
/// Q-CRISIS 阈值（08 第 3 节，偏向召回）。
pub const JEV_CRISIS_THRESHOLD: f64 = 0.5;

/// 会话的对话方式（FR-CHT-06 快捷指令里要调用 AI 的两个），存在 `chat_session.mode`（ADR 0022）。
/// “写成情绪日记”“陪我呼吸”是窗口里的本地动作，不经过这里。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ChatMode {
    /// 平常的对话
    #[default]
    Normal,
    /// 💬 我只是想吐槽：本会话只倾听，只共情和复述，不给建议
    Vent,
    /// 🧭 帮我理一理：引导说清“发生了什么 / 我的感受 / 我能做的一小步”
    Organize,
}

impl ChatMode {
    pub fn as_db(self) -> &'static str {
        match self {
            ChatMode::Normal => "normal",
            ChatMode::Vent => "vent",
            ChatMode::Organize => "organize",
        }
    }

    /// 库里不认识的值按平常对话处理。
    pub fn from_db(v: &str) -> Self {
        match v {
            "vent" => ChatMode::Vent,
            "organize" => ChatMode::Organize,
            _ => ChatMode::Normal,
        }
    }
}

/// 一条要记住的内容：去掉首尾空白后非空且不超过 100 字（FR-CHT-07 第 2 条），否则 `None`。
pub fn memory_entry(content: &str) -> Option<&str> {
    let content = content.trim();
    (!content.is_empty() && content.chars().count() <= MEMORY_ENTRY_CHARS).then_some(content)
}

/// 会话的安全状态，存在 `chat_session.safe_mode`（ADR 0018 第 3 条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum SafeMode {
    /// 普通模式
    Off,
    /// 触发过危机识别：用 P-CHAT-SAFE，求助卡片固定在窗口顶部（FR-SAF-02/03）
    On,
    /// 用户点了“我说的不是这个意思”：回到普通模式、词表阈值提高，求助信息仍折叠显示一行（FR-SAF-06）
    Dismissed,
}

impl SafeMode {
    pub fn from_db(v: i64) -> Self {
        match v {
            1 => SafeMode::On,
            2 => SafeMode::Dismissed,
            _ => SafeMode::Off,
        }
    }

    pub fn to_db(self) -> i64 {
        match self {
            SafeMode::Off => 0,
            SafeMode::On => 1,
            SafeMode::Dismissed => 2,
        }
    }

    /// 本会话用的词表阈值：点过“不是这个意思”后用 `threshold_after_dismiss`（17 第 2.7 节）。
    pub fn lexicon_threshold(self, lex: &CrisisLexicon) -> f64 {
        match self {
            SafeMode::Dismissed => lex.threshold_after_dismiss,
            SafeMode::Off | SafeMode::On => lex.threshold,
        }
    }
}

/// 危机识别命中的通道，即 `safety_log.channel`（FR-SAF-05）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrisisChannel {
    Lexicon,
    Jev,
    Both,
}

impl CrisisChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            CrisisChannel::Lexicon => "lexicon",
            CrisisChannel::Jev => "jev",
            CrisisChannel::Both => "both",
        }
    }

    /// 两个通道的结果合并：任一命中即触发（FR-SAF-01）。
    pub fn combine(lexicon: bool, jev: bool) -> Option<Self> {
        match (lexicon, jev) {
            (true, true) => Some(CrisisChannel::Both),
            (true, false) => Some(CrisisChannel::Lexicon),
            (false, true) => Some(CrisisChannel::Jev),
            (false, false) => None,
        }
    }
}

/// 本地词表通道（≤ 50 ms，先于大模型调用，FR-SAF-01）。
pub fn lexicon_hit(text: &str, lex: &CrisisLexicon, mode: SafeMode) -> bool {
    safety::check_local(text, lex, mode.lexicon_threshold(lex)).hit
}

/// 会话标题：去掉首尾空白、换行改为空格，取前 12 个字（FR-CHT-02 第 2 条，本地截取）。
pub fn title_of(first_message: &str) -> String {
    let flat: String = first_message
        .trim()
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    flat.chars().take(TITLE_CHARS).collect()
}

/// 是否沿用这个会话：上一条消息在 6 小时以内。
pub fn continues(last_ts: Option<i64>, now_ms: i64) -> bool {
    last_ts.is_some_and(|t| now_ms - t <= NEW_SESSION_GAP_MS)
}

/// 发给模型的一条历史消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub user: bool,
    pub content: String,
}

/// 最近 20 轮、估算不超过 3000 token，超出时从最早的开始丢（FR-CHT-05）。`history` 从旧到新，
/// 最后一条是这次要回复的用户消息，它总会保留。
pub fn trim_context(history: &[Turn]) -> &[Turn] {
    let user_turns = history.iter().filter(|t| t.user).count();
    // 先按轮数：从第 (用户消息数 - 20) 条用户消息开始
    let mut skip_users = user_turns.saturating_sub(CONTEXT_TURNS);
    let mut start = 0;
    while skip_users > 0 && start < history.len() {
        if history[start].user {
            skip_users -= 1;
        }
        start += 1;
    }
    // 再按长度：保留最后一条，往前累计
    let mut total = 0;
    let mut keep_from = history.len();
    for (i, t) in history.iter().enumerate().skip(start).rev() {
        total += t.content.chars().count();
        if total > CONTEXT_TOKENS && i + 1 < history.len() {
            break;
        }
        keep_from = i;
    }
    // 不以一条助手回复开头
    while keep_from + 1 < history.len() && !history[keep_from].user {
        keep_from += 1;
    }
    &history[keep_from..]
}

/// 提示词文件（首行 `<!-- version: N -->`，`<!--` 开头的行是注释，不发给模型）。只读出厂版本：
/// 提示词改动要评审并重跑评测（08 第 8 节）。
#[derive(Debug, Clone)]
struct PromptFile {
    version: u32,
    body: String,
}

impl PromptFile {
    fn load(dirs: &TemplateDirs, name: &str) -> Result<Self, TemplateError> {
        let path = dirs.factory_path(name);
        let text = std::fs::read_to_string(&path).map_err(|source| TemplateError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(Self::parse(&text))
    }

    fn parse(text: &str) -> Self {
        let version = text
            .lines()
            .next()
            .and_then(|l| l.trim().strip_prefix("<!-- version:"))
            .and_then(|l| l.trim_end_matches("-->").trim().parse().ok())
            .unwrap_or(0);
        let body = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("<!--"))
            .collect::<Vec<_>>()
            .join("\n");
        Self { version, body }
    }
}

/// P-CHAT 与 P-CHAT-SAFE（08 第 4 节）。
#[derive(Debug, Clone)]
pub struct ChatPrompts {
    chat: PromptFile,
    safe: PromptFile,
    /// 快捷指令的对话方式：接在 P-CHAT 后面（ADR 0022）
    vent: PromptFile,
    organize: PromptFile,
}

impl ChatPrompts {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        Ok(Self {
            chat: PromptFile::load(dirs, "prompts/chat.md")?,
            safe: PromptFile::load(dirs, "prompts/chat_safe.md")?,
            vent: PromptFile::load(dirs, "prompts/chat_vent.md")?,
            organize: PromptFile::load(dirs, "prompts/chat_organize.md")?,
        })
    }

    fn mode_file(&self, mode: ChatMode) -> Option<(&'static str, &PromptFile)> {
        match mode {
            ChatMode::Normal => None,
            ChatMode::Vent => Some(("P-CHAT-VENT", &self.vent)),
            ChatMode::Organize => Some(("P-CHAT-ORGANIZE", &self.organize)),
        }
    }

    /// 记在 `chat_message.prompt_ver` 与 `CompleteRequest.prompt_ver`。快捷指令的对话方式也记上版本，
    /// 例如 `P-CHAT v1 + P-CHAT-VENT v1`；安全模式不用对话方式。
    pub fn ver(&self, safe: bool, mode: ChatMode) -> String {
        if safe {
            return format!("P-CHAT-SAFE v{}", self.safe.version);
        }
        let base = format!("P-CHAT v{}", self.chat.version);
        match self.mode_file(mode) {
            Some((name, f)) => format!("{base} + {name} v{}", f.version),
            None => base,
        }
    }

    /// 系统提示词。安全模式固定温和语气，不用风格、摘要、记忆和对话方式（FR-CHT-10、ADR 0022）。
    pub fn system(
        &self,
        safe: bool,
        style: Style,
        mode: ChatMode,
        summary: Option<&str>,
        memories: &[String],
    ) -> String {
        if safe {
            return self.safe.body.trim().to_string();
        }
        let summary_block = summary
            .filter(|s| !s.is_empty())
            .map(|s| format!("用户今天的大致状态：{s}"))
            .unwrap_or_default();
        let memory_block = memory_block(memories);
        let mut out = self.chat.body.clone();
        for (slot, value) in [
            ("{style_block}", style.style_block()),
            ("{today_summary_block}", summary_block.as_str()),
            ("{memory_block}", memory_block.as_str()),
        ] {
            out = if value.is_empty() {
                // 空的变量连同所在的行一起去掉
                out.replace(&format!("{slot}\n"), "").replace(slot, "")
            } else {
                out.replace(slot, value)
            };
        }
        let mut out = out.trim().to_string();
        if let Some((_, f)) = self.mode_file(mode) {
            out.push('\n');
            out.push_str(f.body.trim());
        }
        out
    }
}

/// `{memory_block}`：最近 10 条，合计不超过 300 字（FR-CHT-05）；`memories` 从新到旧。
fn memory_block(memories: &[String]) -> String {
    let mut used = 0;
    let mut picked = Vec::new();
    for m in memories.iter().take(MEMORY_ITEMS) {
        let n = m.chars().count();
        if used + n > MEMORY_CHARS {
            break;
        }
        used += n;
        picked.push(m.as_str());
    }
    if picked.is_empty() {
        String::new()
    } else {
        format!("用户请你记住的事：{}", picked.join("；"))
    }
}

/// 对话要用的固定文案（`ui_copy.toml`，人工撰写，不标 `AI 生成`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatCopy {
    /// `safety.fallback_reply`：安全模式下大模型不可用、或回复命中 V6 时的固定回应（FR-SAF-03 第 2 条）
    pub fallback_reply: String,
    /// `chat.replaced`：回复命中禁用词时的替换句（ADR 0018 第 2 条）
    pub replaced: String,
    /// `chat.copy_suffix`：复制 AI 回复时附在末尾的标识（FR-CHT-04 第 5 条）
    pub copy_suffix: String,
}

#[derive(Debug, Deserialize)]
struct RawCopy {
    chat: RawChatCopy,
    safety: RawSafetyCopy,
}

#[derive(Debug, Deserialize)]
struct RawChatCopy {
    replaced: String,
    copy_suffix: String,
}

#[derive(Debug, Deserialize)]
struct RawSafetyCopy {
    fallback_reply: String,
}

impl ChatCopy {
    /// 只读出厂版本：固定回应属于危机安全文案（FR-SAF-04 随安装包分发）。
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        let raw: RawCopy = read_toml(&dirs.factory_path("ui_copy.toml"))?;
        Ok(Self {
            fallback_reply: raw.safety.fallback_reply.trim().to_string(),
            replaced: raw.chat.replaced.trim().to_string(),
            copy_suffix: raw.chat.copy_suffix,
        })
    }

    /// 复制一条消息时的文本：AI 生成的回复末尾附加标识（FR-CHT-04 第 5 条），其余原样。
    pub fn for_copy(&self, content: &str, ai_generated: bool) -> String {
        if ai_generated {
            format!("{content}{}", self.copy_suffix)
        } else {
            content.to_string()
        }
    }
}

/// 一条回复的校验结果（08 第 5 节，对话在流式结束后执行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyCheck {
    /// 通过；超过 600 字时已截断并加“……”（V3）
    Ok(String),
    /// V6：命中危机词表的“方法 / 手段”类条目，整条换成安全模式固定回应（FR-SAF-03）
    Unsafe,
    /// V4：命中禁用词（对话放行 `chat_allow`），换成 `chat.replaced`（ADR 0018 第 2 条）
    Banned,
}

pub fn check_reply(text: &str, banned: &BannedWords, lex: &CrisisLexicon) -> ReplyCheck {
    let text = text.trim();
    if safety::check_local(text, lex, lex.threshold).has_method() {
        return ReplyCheck::Unsafe;
    }
    if banned.find(text, Scene::Chat).is_some() {
        return ReplyCheck::Banned;
    }
    if text.chars().count() > MAX_REPLY_CHARS {
        let cut: String = text.chars().take(MAX_REPLY_CHARS).collect();
        return ReplyCheck::Ok(format!("{cut}……"));
    }
    ReplyCheck::Ok(text.to_string())
}

/// 今天某一时刻显示过的状态（`mood_state.shown_state`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatePoint {
    pub local: DateTime<Local>,
    pub state: MoodState,
}

/// 今日状态摘要（FR-CHT-05，例如“上午平稳，22 点后偏低落，今天已输入 3 小时 20 分”）：只有统计，没有任何原文，
/// 不超过 200 字。按时段取出现最多的状态，相邻时段相同就合并；没有任何数据时为 `None`。
pub fn today_summary(points: &[StatePoint], typing_min: u32) -> Option<String> {
    const PERIODS: [(&str, u32, u32); 4] = [
        ("早上和上午", 5, 12),
        ("下午", 12, 18),
        ("晚上", 18, 23),
        ("深夜", 23, 29),
    ];
    let mut parts: Vec<(String, &str)> = Vec::new();
    for (name, from, to) in PERIODS {
        let mut counts = [0u32; 5];
        for p in points {
            // 0–5 点算前一天的深夜
            let h = p.local.hour();
            let h = if h < 5 { h + 24 } else { h };
            if h < from || h >= to {
                continue;
            }
            if let Some(i) = state_index(p.state) {
                counts[i] += 1;
            }
        }
        let Some((best, _)) = counts
            .iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .max_by_key(|(i, n)| (**n, std::cmp::Reverse(*i)))
        else {
            continue;
        };
        let word = STATE_WORDS[best];
        match parts.last_mut() {
            Some((names, w)) if *w == word => {
                names.push('、');
                names.push_str(name);
            }
            _ => parts.push((name.to_string(), word)),
        }
    }
    let mut out: Vec<String> = parts.into_iter().map(|(n, w)| format!("{n}{w}")).collect();
    if typing_min > 0 {
        let (h, m) = (typing_min / 60, typing_min % 60);
        out.push(match (h, m) {
            (0, m) => format!("今天已输入 {m} 分钟"),
            (h, 0) => format!("今天已输入 {h} 小时"),
            (h, m) => format!("今天已输入 {h} 小时 {m} 分"),
        });
    }
    if out.is_empty() {
        return None;
    }
    let s = out.join("，");
    Some(s.chars().take(SUMMARY_CHARS).collect())
}

const STATE_WORDS: [&str; 5] = ["平稳", "有点犹豫", "偏低落", "有点烦躁", "有点累"];

fn state_index(s: MoodState) -> Option<usize> {
    match s {
        MoodState::Fluent => Some(0),
        MoodState::Hesitant => Some(1),
        MoodState::Low => Some(2),
        MoodState::Agitated => Some(3),
        MoodState::Tired => Some(4),
        MoodState::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{NaiveDate, TimeZone};

    use super::*;

    fn dirs() -> TemplateDirs {
        TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        )
    }

    fn turn(user: bool, s: &str) -> Turn {
        Turn {
            user,
            content: s.into(),
        }
    }

    #[test]
    fn title_takes_first_12_chars() {
        assert_eq!(
            title_of("  今天考试没考好\n心里有点难受啊  "),
            "今天考试没考好 心里有点"
        );
        assert_eq!(title_of("嗨"), "嗨");
    }

    #[test]
    fn chat_mode_round_trips_through_db() {
        for m in [ChatMode::Normal, ChatMode::Vent, ChatMode::Organize] {
            assert_eq!(ChatMode::from_db(m.as_db()), m);
        }
        assert_eq!(ChatMode::from_db("??"), ChatMode::Normal);
    }

    #[test]
    fn memory_entry_trims_and_limits_length() {
        assert_eq!(memory_entry("  我喜欢晴天 "), Some("我喜欢晴天"));
        assert_eq!(memory_entry("   "), None);
        assert!(memory_entry(&"晴".repeat(MEMORY_ENTRY_CHARS)).is_some());
        assert_eq!(memory_entry(&"晴".repeat(MEMORY_ENTRY_CHARS + 1)), None);
    }

    #[test]
    fn new_session_after_six_hours() {
        let now = 100 * NEW_SESSION_GAP_MS;
        assert!(continues(Some(now - NEW_SESSION_GAP_MS), now));
        assert!(!continues(Some(now - NEW_SESSION_GAP_MS - 1), now));
        assert!(!continues(None, now));
    }

    #[test]
    fn context_keeps_last_20_turns() {
        let mut h = Vec::new();
        for i in 0..25 {
            h.push(turn(true, &format!("问{i}")));
            h.push(turn(false, &format!("答{i}")));
        }
        h.push(turn(true, "最后一问"));
        let kept = trim_context(&h);
        assert_eq!(kept.iter().filter(|t| t.user).count(), CONTEXT_TURNS);
        assert!(kept[0].user);
        assert_eq!(kept.last().unwrap().content, "最后一问");
    }

    #[test]
    fn context_drops_oldest_when_too_long() {
        let long = "字".repeat(1200);
        let h = vec![
            turn(true, &long),
            turn(false, &long),
            turn(true, &long),
            turn(false, "短"),
            turn(true, "现在"),
        ];
        let kept = trim_context(&h);
        // 1200 + 1 + 2 = 1203；再加一条 1200 = 2403；再加 1200 超过 3000
        assert_eq!(kept.len(), 3);
        assert!(kept[0].user);
        // 最后一条再长也保留
        let h = vec![turn(true, &"字".repeat(5000))];
        assert_eq!(trim_context(&h).len(), 1);
    }

    #[test]
    fn prompts_fill_and_drop_blocks() {
        let p = ChatPrompts::load(&dirs()).unwrap();
        assert_eq!(p.ver(false, ChatMode::Normal), "P-CHAT v1");
        assert_eq!(p.ver(true, ChatMode::Vent), "P-CHAT-SAFE v1");
        let plain = p.system(false, Style::Gentle, ChatMode::Normal, None, &[]);
        assert!(!plain.contains('{'), "没填的变量不能留在提示词里：{plain}");
        assert!(plain.ends_with("绝不提供任何方法相关的信息。"));
        let full = p.system(
            false,
            Style::Lively,
            ChatMode::Normal,
            Some("下午平稳，今天已输入 2 小时"),
            &["下周三考英语".into(), "养了一只猫叫团子".into()],
        );
        assert!(full.contains("语气轻快"));
        assert!(full.contains("用户今天的大致状态：下午平稳，今天已输入 2 小时"));
        assert!(full.contains("用户请你记住的事：下周三考英语；养了一只猫叫团子"));
        let safe = p.system(
            true,
            Style::Lively,
            ChatMode::Vent,
            Some("x"),
            &["y".into()],
        );
        assert!(safe.contains("你现在安全吗"));
        assert!(!safe.contains("语气轻快"), "安全模式忽略风格（FR-CHT-10）");
        assert!(!safe.contains("只倾听"), "安全模式忽略对话方式（ADR 0022）");
        assert!(!safe.contains("<!--"));
    }

    #[test]
    fn chat_modes_append_after_p_chat_and_carry_version() {
        let p = ChatPrompts::load(&dirs()).unwrap();
        assert_eq!(p.ver(false, ChatMode::Vent), "P-CHAT v1 + P-CHAT-VENT v1");
        assert_eq!(
            p.ver(false, ChatMode::Organize),
            "P-CHAT v1 + P-CHAT-ORGANIZE v1"
        );
        let plain = p.system(false, Style::Gentle, ChatMode::Normal, None, &[]);
        let vent = p.system(false, Style::Gentle, ChatMode::Vent, None, &[]);
        assert!(vent.starts_with(&plain), "边界部分原样保留");
        assert!(vent.contains("只倾听"));
        assert!(!vent.contains("<!--"));
        let organize = p.system(false, Style::Gentle, ChatMode::Organize, None, &[]);
        assert!(organize.contains("我能做的一小步"));
        let banned = BannedWords::load(&dirs()).unwrap();
        for text in [&vent, &organize] {
            assert_eq!(banned.find(&text[plain.len()..], Scene::Other), None);
        }
    }

    #[test]
    fn copy_loads() {
        let c = ChatCopy::load(&dirs()).unwrap();
        assert!(c.fallback_reply.contains("12356"));
        assert_eq!(c.for_copy("好的。", true), "好的。（内容由 AI 生成）");
        assert_eq!(c.for_copy("你好", false), "你好");
        let banned = BannedWords::load(&dirs()).unwrap();
        assert_eq!(banned.find(&c.replaced, Scene::Chat), None);
    }

    #[test]
    fn memory_block_caps_at_300_chars() {
        let m: Vec<String> = (0..12).map(|_| "记".repeat(40)).collect();
        let b = memory_block(&m);
        assert_eq!(
            b.matches('；').count(),
            6,
            "7 条 280 字，第 8 条超过 300 字"
        );
    }

    #[test]
    fn reply_checks() {
        let banned = BannedWords::load(&dirs()).unwrap();
        let lex = CrisisLexicon::load(&dirs()).unwrap();
        assert_eq!(
            check_reply(" 听起来你今天挺累的。 ", &banned, &lex),
            ReplyCheck::Ok("听起来你今天挺累的。".into())
        );
        let long = "好".repeat(700);
        let ReplyCheck::Ok(cut) = check_reply(&long, &banned, &lex) else {
            panic!()
        };
        assert_eq!(cut.chars().count(), MAX_REPLY_CHARS + 2);
        assert!(cut.ends_with("……"));
        assert_eq!(
            check_reply("你可以试试割腕", &banned, &lex),
            ReplyCheck::Unsafe
        );
        assert_eq!(
            check_reply("你这是抑郁症的表现", &banned, &lex),
            ReplyCheck::Banned
        );
    }

    #[test]
    fn crisis_channels_and_dismissed_threshold() {
        let lex = CrisisLexicon::load(&dirs()).unwrap();
        assert!(lexicon_hit("我真的不想活了", &lex, SafeMode::Off));
        assert!(!lexicon_hit("今天的作业好多啊", &lex, SafeMode::Off));
        assert_eq!(SafeMode::Dismissed.lexicon_threshold(&lex), 1.6);
        assert_eq!(
            CrisisChannel::combine(true, false),
            Some(CrisisChannel::Lexicon)
        );
        assert_eq!(CrisisChannel::combine(true, true).unwrap().as_str(), "both");
        assert_eq!(CrisisChannel::combine(false, false), None);
        for m in [SafeMode::Off, SafeMode::On, SafeMode::Dismissed] {
            assert_eq!(SafeMode::from_db(m.to_db()), m);
        }
    }

    #[test]
    fn summary_groups_periods() {
        let at = |h: u32, s: MoodState| StatePoint {
            local: Local
                .from_local_datetime(
                    &NaiveDate::from_ymd_opt(2026, 3, 10)
                        .unwrap()
                        .and_hms_opt(h, 0, 0)
                        .unwrap(),
                )
                .earliest()
                .unwrap(),
            state: s,
        };
        let pts = [
            at(9, MoodState::Fluent),
            at(10, MoodState::Fluent),
            at(14, MoodState::Fluent),
            at(22, MoodState::Low),
            at(22, MoodState::Low),
            at(21, MoodState::Fluent),
            at(23, MoodState::Unknown),
        ];
        assert_eq!(
            today_summary(&pts, 200).unwrap(),
            "早上和上午、下午平稳，晚上偏低落，今天已输入 3 小时 20 分"
        );
        assert_eq!(today_summary(&[], 45).unwrap(), "今天已输入 45 分钟");
        assert_eq!(today_summary(&[], 0), None);
        let banned = BannedWords::load(&dirs()).unwrap();
        let all = [
            at(9, MoodState::Hesitant),
            at(13, MoodState::Agitated),
            at(19, MoodState::Tired),
            at(1, MoodState::Low),
        ];
        let s = today_summary(&all, 120).unwrap();
        assert_eq!(banned.find(&s, Scene::Other), None, "{s}");
    }
}
