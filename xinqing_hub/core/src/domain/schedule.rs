//! 日程与待办的本地单句初筛（FR-SCH-01、FR-SCH-12）。
//!
//! 原句仅保留在返回值中供后续显式同意后的单句确认使用；调用方不得持久化它。

use chrono::NaiveDate;
use regex::Regex;
use serde::Deserialize;

use crate::infra::templates::{TemplateDirs, TemplateError};

const FILE: &str = "schedule_patterns.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateKind {
    Schedule,
    Todo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSentence {
    pub kind: CandidateKind,
    pub text: String,
}

/// 不包含识别原句的日程数据，可安全写入 D-12。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleDraft {
    pub title: String,
    pub date: Option<String>,
    pub time: Option<String>,
    pub end_time: Option<String>,
    pub all_day: bool,
    pub location: Option<String>,
    pub is_deadline: bool,
    pub remind_offsets: Vec<i64>,
    pub source: String,
    pub flags: Vec<String>,
}

/// 不包含识别原句的待办数据，可安全写入 D-25。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoDraft {
    pub title: String,
    pub due_date: Option<String>,
    pub source: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExtractError {
    #[error("AI 抽取结果不是有效 JSON")]
    Json,
    #[error("AI 抽取结果字段类型或值不合法")]
    Shape,
    #[error("AI 抽取结果日期或时间不合法")]
    DateTime,
    #[error("AI 抽取结果标题超长或为空")]
    Title,
}

#[derive(Debug, Deserialize)]
struct ScheduleOutput {
    has_event: bool,
    title: Option<String>,
    date: Option<String>,
    time: Option<String>,
    end_time: Option<String>,
    all_day: bool,
    location: Option<String>,
    is_deadline: bool,
}

#[derive(Debug, Deserialize)]
struct TodoOutput {
    is_todo: bool,
    title: Option<String>,
    due_date: Option<String>,
}

fn optional_date(value: Option<String>) -> Result<Option<String>, ExtractError> {
    value
        .map(|date| {
            NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .map(|_| date)
                .map_err(|_| ExtractError::DateTime)
        })
        .transpose()
}

fn optional_time(value: Option<String>) -> Result<Option<String>, ExtractError> {
    value
        .map(|time| {
            chrono::NaiveTime::parse_from_str(&time, "%H:%M")
                .map(|_| time)
                .map_err(|_| ExtractError::DateTime)
        })
        .transpose()
}

/// 校验 P-SCHEDULE 的 JSON。`has_event=false` 返回 `Ok(None)`，不写入数据库。
pub fn validate_schedule_json(raw: &str) -> Result<Option<ScheduleDraft>, ExtractError> {
    let output: ScheduleOutput = serde_json::from_str(raw).map_err(|_| ExtractError::Json)?;
    if !output.has_event {
        return Ok(None);
    }
    let title = output
        .title
        .filter(|value| !value.trim().is_empty())
        .ok_or(ExtractError::Title)?;
    if title.chars().count() > 12 {
        return Err(ExtractError::Title);
    }
    let date = optional_date(output.date)?;
    let time = optional_time(output.time)?;
    let end_time = optional_time(output.end_time)?;
    if output.all_day && time.is_some() || end_time.is_some() && time.is_none() {
        return Err(ExtractError::Shape);
    }
    if let (Some(start), Some(end)) = (&time, &end_time)
        && start >= end
    {
        return Err(ExtractError::DateTime);
    }
    if output
        .location
        .as_ref()
        .is_some_and(|value| value.chars().count() > 40)
    {
        return Err(ExtractError::Shape);
    }
    Ok(Some(ScheduleDraft {
        title: title.trim().to_owned(),
        date,
        time,
        end_time,
        all_day: output.all_day,
        location: output.location.filter(|value| !value.trim().is_empty()),
        is_deadline: output.is_deadline,
        remind_offsets: Vec::new(),
        source: "ai".into(),
        flags: Vec::new(),
    }))
}

/// 校验 P-TODO 的 JSON。`is_todo=false` 返回 `Ok(None)`，不写入数据库。
pub fn validate_todo_json(raw: &str) -> Result<Option<TodoDraft>, ExtractError> {
    let output: TodoOutput = serde_json::from_str(raw).map_err(|_| ExtractError::Json)?;
    if !output.is_todo {
        return Ok(None);
    }
    let title = output
        .title
        .filter(|value| !value.trim().is_empty())
        .ok_or(ExtractError::Title)?;
    if title.chars().count() > 16 {
        return Err(ExtractError::Title);
    }
    Ok(Some(TodoDraft {
        title: title.trim().to_owned(),
        due_date: optional_date(output.due_date)?,
        source: "ai".into(),
    }))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanResult {
    pub candidates: Vec<CandidateSentence>,
    pub skipped_by_daily_cap: usize,
}

#[derive(Debug, Deserialize)]
struct RawPatterns {
    min_len: usize,
    max_len: usize,
    daily_cap: usize,
    date_relative: RelativePattern,
    date_absolute: AbsolutePattern,
    time_of_day: Pattern,
    deadline: Pattern,
    event_verb: Pattern,
    exclude: ExcludePatterns,
    todo: TodoPatterns,
}

#[derive(Debug, Deserialize)]
struct Pattern {
    pattern: String,
}

#[derive(Debug, Deserialize)]
struct RelativePattern {
    pattern: String,
    #[serde(default)]
    not_preceded_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AbsolutePattern {
    pattern: String,
    not_preceded_by_char: String,
    not_followed_by_char: String,
    hao_not_followed_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExcludePatterns {
    past: String,
    question_end: String,
    confirm: String,
}

#[derive(Debug, Deserialize)]
struct TodoPatterns {
    hint: String,
    action: String,
    huan_not_followed_by: Vec<String>,
    second_person: String,
    second_person_allow: String,
    remind_others: String,
    remind_others_not_followed_by: Vec<String>,
}

#[derive(Debug)]
pub struct ScheduleRecognizer {
    min_len: usize,
    max_len: usize,
    daily_cap: usize,
    relative_date: Regex,
    relative_not_preceded_by: Vec<String>,
    absolute_date: Regex,
    time: Regex,
    deadline: Regex,
    event_verb: Regex,
    past: Regex,
    question_end: Regex,
    confirm: Regex,
    todo_hint: Regex,
    todo_action: Regex,
    second_person: Regex,
    second_person_allow: Regex,
    remind_others: Regex,
    not_followed_by: Vec<String>,
    absolute_not_followed_by: Regex,
    absolute_not_preceded_by: Regex,
    hao_not_followed_by: Vec<String>,
    remind_not_followed_by: Vec<String>,
}

impl ScheduleRecognizer {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        let path = dirs.resolve(FILE);
        let raw: RawPatterns = crate::infra::templates::read_toml(&path)?;
        let compile = |pattern: &str| {
            Regex::new(pattern).map_err(|source| TemplateError::Regex {
                file: FILE,
                pattern: pattern.to_string(),
                source,
            })
        };
        Ok(Self {
            min_len: raw.min_len,
            max_len: raw.max_len,
            daily_cap: raw.daily_cap,
            relative_date: compile(&raw.date_relative.pattern)?,
            relative_not_preceded_by: raw.date_relative.not_preceded_by,
            absolute_date: compile(&raw.date_absolute.pattern)?,
            time: compile(&raw.time_of_day.pattern)?,
            deadline: compile(&raw.deadline.pattern)?,
            event_verb: compile(&raw.event_verb.pattern)?,
            past: compile(&raw.exclude.past)?,
            question_end: compile(&raw.exclude.question_end)?,
            confirm: compile(&raw.exclude.confirm)?,
            todo_hint: compile(&raw.todo.hint)?,
            todo_action: compile(&raw.todo.action)?,
            second_person: compile(&raw.todo.second_person)?,
            second_person_allow: compile(&raw.todo.second_person_allow)?,
            remind_others: compile(&raw.todo.remind_others)?,
            not_followed_by: raw.todo.huan_not_followed_by,
            absolute_not_followed_by: compile(&raw.date_absolute.not_followed_by_char)?,
            absolute_not_preceded_by: compile(&raw.date_absolute.not_preceded_by_char)?,
            hao_not_followed_by: raw.date_absolute.hao_not_followed_by,
            remind_not_followed_by: raw.todo.remind_others_not_followed_by,
        })
    }

    /// 扫描新上屏文本。`accepted_today` 是当天此前已命中的日程与待办总数。
    pub fn scan(&self, committed_text: &str, accepted_today: usize) -> ScanResult {
        let mut result = ScanResult::default();
        let mut used = accepted_today;
        for raw_sentence in
            committed_text.split_inclusive(['。', '！', '？', '；', '\n', '!', '?', ';'])
        {
            let raw_sentence = raw_sentence.trim();
            if self.excluded(raw_sentence) {
                continue;
            }
            let sentence = raw_sentence
                .trim_end_matches(['。', '！', '？', '；', '\n', '!', '?', ';'])
                .trim();
            let len = sentence.chars().count();
            if len < self.min_len || len > self.max_len {
                continue;
            }
            let has_time = self.time.is_match(sentence);
            let todo = self.is_todo(sentence) && !has_time;
            let schedule = self.has_date(sentence)
                || self.deadline.is_match(sentence)
                || (has_time && self.event_verb.is_match(sentence));
            if !todo && !schedule {
                continue;
            }
            if used >= self.daily_cap {
                result.skipped_by_daily_cap += 1;
                continue;
            }
            used += 1;
            result.candidates.push(CandidateSentence {
                kind: if todo {
                    CandidateKind::Todo
                } else {
                    CandidateKind::Schedule
                },
                text: sentence.to_owned(),
            });
        }
        result
    }

    fn excluded(&self, sentence: &str) -> bool {
        let has_future_date = self.has_date(sentence);
        if self.past.is_match(sentence) && !has_future_date {
            return true;
        }
        self.question_end.is_match(sentence) && !self.confirm.is_match(sentence)
    }

    fn has_date(&self, sentence: &str) -> bool {
        let relative = self.relative_date.find_iter(sentence).any(|m| {
            let before = &sentence[..m.start()];
            !self
                .relative_not_preceded_by
                .iter()
                .any(|prefix| before.ends_with(prefix))
        });
        relative || self.valid_absolute_date(sentence)
    }

    fn valid_absolute_date(&self, sentence: &str) -> bool {
        self.absolute_date.find_iter(sentence).any(|m| {
            let before = &sentence[..m.start()];
            if m.as_str().contains('.') && before.ends_with('点') {
                return false;
            }
            if self
                .absolute_not_preceded_by
                .find(before)
                .is_some_and(|previous| previous.end() == before.len())
            {
                return false;
            }
            let after = &sentence[m.end()..];
            if self
                .absolute_not_followed_by
                .find(after)
                .is_some_and(|next| next.start() == 0)
            {
                return false;
            }
            if m.as_str().ends_with('号')
                && self
                    .hao_not_followed_by
                    .iter()
                    .any(|suffix| after.starts_with(suffix))
            {
                return false;
            }
            if let Some((month, day)) = parse_numeric_date(m.as_str()) {
                return (1..=12).contains(&month) && (1..=31).contains(&day);
            }
            if let Some((month, day)) = parse_chinese_date(m.as_str()) {
                return (1..=12).contains(&month) && (1..=31).contains(&day);
            }
            true
        })
    }

    fn is_todo(&self, sentence: &str) -> bool {
        let Some(hint) = self.todo_hint.find(sentence) else {
            return false;
        };
        let remainder = format!("{}{}", &sentence[..hint.start()], &sentence[hint.end()..]);
        if self.second_person.is_match(sentence) && !self.second_person_allow.is_match(sentence) {
            return false;
        }
        if let Some(remind) = self.remind_others.find(&remainder) {
            let next = remainder[remind.end()..].chars().next();
            if next.is_some_and(|c| self.remind_not_followed_by.iter().any(|s| s.starts_with(c))) {
                return false;
            }
        }
        self.todo_action.find_iter(&remainder).any(|action| {
            action.as_str() != "还"
                || remainder[action.end()..]
                    .chars()
                    .next()
                    .is_none_or(|next| !self.not_followed_by.iter().any(|s| s.starts_with(next)))
        })
    }
}

fn parse_numeric_date(raw: &str) -> Option<(u32, u32)> {
    let normalized = raw.replace(['.', '/', '-'], "-");
    let (month, day) = normalized.split_once('-')?;
    Some((month.parse().ok()?, day.parse().ok()?))
}

fn parse_chinese_date(raw: &str) -> Option<(u32, u32)> {
    let (month, day) = raw.split_once('月')?;
    let day = day.trim_end_matches(['日', '号']);
    Some((month.parse().ok()?, day.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recognizer() -> ScheduleRecognizer {
        let dirs = TemplateDirs::factory_only(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        );
        ScheduleRecognizer::load(&dirs).unwrap()
    }

    #[test]
    fn routes_deadline_reminder_to_todo_and_timed_event_to_schedule() {
        let result = recognizer().scan("记得周五前交材料。明天下午两点在实验楼开会", 0);
        assert_eq!(result.candidates.len(), 2);
        assert_eq!(result.candidates[0].kind, CandidateKind::Todo);
        assert_eq!(result.candidates[1].kind, CandidateKind::Schedule);
    }

    #[test]
    fn filters_past_questions_and_non_date_numeric_context() {
        let result = recognizer().scan("上周五我们开会。明天要开会吗？绩点3.5真的很高", 0);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn does_not_exceed_daily_budget() {
        let result = recognizer().scan("明天开会。后天开会", 49);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.skipped_by_daily_cap, 1);
    }

    #[test]
    fn rejects_out_of_range_calendar_dates() {
        assert!(!recognizer().has_date("19月99号要面试"));
        assert!(recognizer().has_date("10月15号上午面试"));
    }

    #[test]
    fn validates_schedule_shape_and_time_order() {
        let draft = validate_schedule_json(r#"{"has_event":true,"title":"组会","date":"2026-10-09","time":"15:00","end_time":"16:00","all_day":false,"location":"实验楼","is_deadline":false}"#).unwrap().unwrap();
        assert_eq!(draft.source, "ai");
        assert_eq!(draft.time.as_deref(), Some("15:00"));
        assert_eq!(
            validate_schedule_json(
                r#"{"has_event":true,"title":"组会","date":"2026-10-09","time":"16:00","end_time":"15:00","all_day":false,"location":null,"is_deadline":false}"#
            ),
            Err(ExtractError::DateTime)
        );
        assert_eq!(
            validate_schedule_json(
                r#"{"has_event":true,"title":"组会","date":"2026-02-30","time":null,"end_time":null,"all_day":true,"location":null,"is_deadline":false}"#
            ),
            Err(ExtractError::DateTime)
        );
    }

    #[test]
    fn validates_todo_route_and_title_limit() {
        assert!(
            validate_todo_json(r#"{"is_todo":false,"title":null,"due_date":null}"#)
                .unwrap()
                .is_none()
        );
        let todo =
            validate_todo_json(r#"{"is_todo":true,"title":"打印简历","due_date":"2026-10-09"}"#)
                .unwrap()
                .unwrap();
        assert_eq!(todo.source, "ai");
        assert_eq!(
            validate_todo_json(
                r#"{"is_todo":true,"title":"这是一条超过十六个汉字长度的待办事项标题","due_date":null}"#
            ),
            Err(ExtractError::Title)
        );
    }
}
