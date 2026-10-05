//! 日程与待办的本地单句初筛（FR-SCH-01、FR-SCH-12）。
//!
//! 原句仅保留在返回值中供后续显式同意后的单句确认使用；调用方不得持久化它。

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use regex::Regex;
use serde::Deserialize;

use crate::domain::validate;
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

/// AI 抽取结果没通过校验的原因（08 第 5 节）。调用方按 08 重新生成 1 次，仍失败转本地抽取
/// （FR-SCH-03 第 2 条）。
#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum ExtractError {
    /// V1：去掉代码块后仍不是 JSON
    #[error("AI 抽取结果不是有效 JSON")]
    Json,
    /// V2：缺字段或类型不对
    #[error("AI 抽取结果字段类型或值不合法")]
    Shape,
    /// V2：日期或时刻不是合法的日历值
    #[error("AI 抽取结果日期或时间不合法")]
    DateTime,
    /// V3：标题为空或超长
    #[error("AI 抽取结果标题超长或为空")]
    Title,
}

/// 日程标题上限（FR-SCH-03 第 3 条、08 第 5 节 V3）。
const SCHEDULE_TITLE_MAX: usize = 12;
/// 待办标题上限（FR-SCH-12、08 第 5 节 V3）。
const TODO_TITLE_MAX: usize = 16;
/// 地点上限：产品书没有规定，超过这个长度多半是模型把整句抄了进来（FR-SCH-04 补充规则 7）。
const LOCATION_MAX: usize = 40;
/// 截止类没有时刻时的默认时刻（FR-SCH-04 校验第 5 条）。
const DEADLINE_TIME: NaiveTime = NaiveTime::from_hms_opt(23, 59, 0).unwrap();
/// 晚于这么多天以后标注“请确认日期”（FR-SCH-04 校验第 3 条）。
const CONFIRM_DATE_DAYS: i64 = 365;

/// `schedule.flags` 的取值（09 D-12）。
pub const FLAG_MAYBE_PAST: &str = "maybe_past";
pub const FLAG_CONFIRM_DATE: &str = "confirm_date";

#[derive(Debug, Deserialize)]
struct ScheduleOutput {
    has_event: bool,
    title: Option<String>,
    date: Option<String>,
    time: Option<String>,
    end_time: Option<String>,
    /// 只校验类型；是否全天由有没有时刻决定（FR-SCH-04 时刻规则、补充规则 2、6）
    #[allow(dead_code)]
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

/// V1 + V2：按 08 第 5 节去掉代码块、截取 `{…}` 后再按字段反序列化。
fn parse_output<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, ExtractError> {
    let value = validate::extract_json(raw).ok_or(ExtractError::Json)?;
    serde_json::from_value(value).map_err(|_| ExtractError::Shape)
}

/// V3：去掉首尾空白后非空且不超过 `max` 个字。
fn checked_title(title: Option<String>, max: usize) -> Result<String, ExtractError> {
    let title = title.as_deref().map(str::trim).unwrap_or_default();
    if title.is_empty() || title.chars().count() > max {
        return Err(ExtractError::Title);
    }
    Ok(title.to_owned())
}

fn optional_date(value: Option<String>) -> Result<Option<NaiveDate>, ExtractError> {
    value
        .map(|date| {
            NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").map_err(|_| ExtractError::DateTime)
        })
        .transpose()
}

fn optional_time(value: Option<String>) -> Result<Option<NaiveTime>, ExtractError> {
    value
        .map(|time| {
            NaiveTime::parse_from_str(time.trim(), "%H:%M").map_err(|_| ExtractError::DateTime)
        })
        .transpose()
}

fn format_date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn format_time(time: NaiveTime) -> String {
    time.format("%H:%M").to_string()
}

/// 校验并规范化 P-SCHEDULE 的输出（08 第 5 节、FR-SCH-03、FR-SCH-04 校验第 2～5 条）。
/// `now` 是本地时间；`has_event=false` 返回 `Ok(None)`，不写入数据库。
///
/// 日期、时刻统一写成 `YYYY-MM-DD`、`HH:MM`，去重哈希（FR-SCH-06）才不会因为 `9:30` 和
/// `09:30` 算成两条。FR-SCH-04 校验第 1 条（模型日期与代码日期不一致时采用代码结果并标
/// `adjusted`）要用原句，由接 L3 的调用方做。
pub fn validate_schedule_json(
    raw: &str,
    now: NaiveDateTime,
) -> Result<Option<ScheduleDraft>, ExtractError> {
    let output: ScheduleOutput = parse_output(raw)?;
    if !output.has_event {
        return Ok(None);
    }
    let title = checked_title(output.title, SCHEDULE_TITLE_MAX)?;
    let location = output
        .location
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if location
        .as_ref()
        .is_some_and(|value| value.chars().count() > LOCATION_MAX)
    {
        return Err(ExtractError::Shape);
    }
    let mut date = optional_date(output.date)?;
    let mut time = optional_time(output.time)?;
    let end_time = optional_time(output.end_time)?;

    // 校验第 5 条：截止类没有时刻默认 23:59。连日期也没有时不补，留给卡片让用户补充
    if output.is_deadline && date.is_some() && time.is_none() {
        time = Some(DEADLINE_TIME);
    }
    // 校验第 4 条：有时刻没有日期默认今天，时刻已过顺延到明天
    if let (None, Some(start)) = (date, time) {
        let today = now.date();
        date = Some(if start <= now.time() {
            today.succ_opt().unwrap_or(today)
        } else {
            today
        });
    }
    // 结束时刻只在有开始时刻且晚于它时才有意义，否则丢掉，不连累其余字段
    let end_time = end_time.filter(|end| time.is_some_and(|start| *end > start));

    let mut flags = Vec::new();
    if let Some(day) = date {
        // 校验第 2 条：全天日程按日期比，带时刻的按时刻比
        let past = match time {
            Some(start) => day.and_time(start) < now,
            None => day < now.date(),
        };
        if past {
            flags.push(FLAG_MAYBE_PAST.to_owned());
        }
        // 校验第 3 条
        if (day - now.date()).num_days() > CONFIRM_DATE_DAYS {
            flags.push(FLAG_CONFIRM_DATE.to_owned());
        }
    }

    Ok(Some(ScheduleDraft {
        title,
        date: date.map(format_date),
        all_day: time.is_none(),
        time: time.map(format_time),
        end_time: end_time.map(format_time),
        location,
        is_deadline: output.is_deadline,
        remind_offsets: Vec::new(),
        source: "ai".into(),
        flags,
    }))
}

/// 校验并规范化 P-TODO 的输出（08 第 5 节、FR-SCH-12）。`is_todo=false` 返回 `Ok(None)`，
/// 不写入数据库。
pub fn validate_todo_json(raw: &str) -> Result<Option<TodoDraft>, ExtractError> {
    let output: TodoOutput = parse_output(raw)?;
    if !output.is_todo {
        return Ok(None);
    }
    Ok(Some(TodoDraft {
        title: checked_title(output.title, TODO_TITLE_MAX)?,
        due_date: optional_date(output.due_date)?.map(format_date),
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

    /// FR-SCH-04 验收用的“今天”：2026-10-03（周六）上午 10 点。
    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 3)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
    }

    /// 以 P-SCHEDULE 的一条合法输出为底，用 `patch` 改其中几个字段。
    fn schedule_output(patch: serde_json::Value) -> String {
        let mut output = serde_json::json!({
            "has_event": true, "title": "组会", "date": "2026-10-09", "time": "15:00",
            "end_time": null, "all_day": false, "location": "实验楼", "is_deadline": false,
        });
        for (key, value) in patch.as_object().unwrap() {
            output[key] = value.clone();
        }
        output.to_string()
    }

    fn schedule(patch: serde_json::Value) -> Result<Option<ScheduleDraft>, ExtractError> {
        validate_schedule_json(&schedule_output(patch), now())
    }

    #[test]
    fn schedule_strips_code_fence_and_normalizes_fields() {
        let raw = format!(
            "```json\n{}\n```",
            schedule_output(serde_json::json!({
                "title": " 组会 ", "time": "9:30", "end_time": "10:00", "location": " 实验楼 ",
            }))
        );
        let draft = validate_schedule_json(&raw, now()).unwrap().unwrap();
        assert_eq!(draft.title, "组会");
        assert_eq!(draft.date.as_deref(), Some("2026-10-09"));
        assert_eq!(draft.time.as_deref(), Some("09:30"));
        assert_eq!(draft.end_time.as_deref(), Some("10:00"));
        assert_eq!(draft.location.as_deref(), Some("实验楼"));
        assert!(!draft.all_day);
        assert_eq!(draft.source, "ai");
        assert!(draft.flags.is_empty());
        assert_eq!(
            schedule(serde_json::json!({ "has_event": false })),
            Ok(None)
        );
    }

    #[test]
    fn schedule_follows_fr_sch_04_examples() {
        // 明晚八点前交数据库作业 → 2026-10-04 20:00，截止
        let draft = schedule(serde_json::json!({
            "title": "交数据库作业", "date": "2026-10-04", "time": "20:00",
            "location": null, "is_deadline": true,
        }))
        .unwrap()
        .unwrap();
        assert_eq!(
            (draft.time.as_deref(), draft.is_deadline, draft.flags.len()),
            (Some("20:00"), true, 0)
        );
        // 下周二和室友去看电影 → 全天；模型把 all_day 填成 false 也按全天
        let draft = schedule(serde_json::json!({
            "title": "看电影", "date": "2026-10-06", "time": null, "location": null,
        }))
        .unwrap()
        .unwrap();
        assert!(draft.all_day);
        assert_eq!(draft.time, None);
        // 这周五交报告 → 2026-10-02，截止类默认 23:59，标注时间可能已过
        let draft = schedule(serde_json::json!({
            "title": "交报告", "date": "2026-10-02", "time": null,
            "all_day": true, "location": null, "is_deadline": true,
        }))
        .unwrap()
        .unwrap();
        assert_eq!(draft.time.as_deref(), Some("23:59"));
        assert!(!draft.all_day);
        assert_eq!(draft.flags, [FLAG_MAYBE_PAST]);
    }

    #[test]
    fn schedule_fills_missing_date_and_flags_far_dates() {
        let date_of = |time: &str| {
            schedule(serde_json::json!({ "date": null, "time": time }))
                .unwrap()
                .unwrap()
                .date
        };
        assert_eq!(date_of("15:00").as_deref(), Some("2026-10-03"));
        assert_eq!(date_of("09:00").as_deref(), Some("2026-10-04"));
        let far = schedule(serde_json::json!({ "date": "2027-10-05" }))
            .unwrap()
            .unwrap();
        assert_eq!(far.flags, [FLAG_CONFIRM_DATE]);
        let today_all_day = schedule(serde_json::json!({ "date": "2026-10-03", "time": null }))
            .unwrap()
            .unwrap();
        assert!(today_all_day.flags.is_empty());
    }

    #[test]
    fn schedule_drops_end_time_without_valid_start() {
        let end_of = |time: serde_json::Value| {
            schedule(serde_json::json!({ "time": time, "end_time": "15:00" }))
                .unwrap()
                .unwrap()
                .end_time
        };
        assert_eq!(end_of(serde_json::json!("16:00")), None);
        assert_eq!(end_of(serde_json::json!("15:00")), None);
        assert_eq!(end_of(serde_json::Value::Null), None);
        assert_eq!(end_of(serde_json::json!("14:00")).as_deref(), Some("15:00"));
    }

    #[test]
    fn schedule_rejects_invalid_output() {
        assert_eq!(
            validate_schedule_json("好的，我来抽取", now()),
            Err(ExtractError::Json)
        );
        assert_eq!(
            validate_schedule_json(r#"{"has_event":true,"title":"组会"}"#, now()),
            Err(ExtractError::Shape)
        );
        assert_eq!(
            schedule(serde_json::json!({ "has_event": "true" })),
            Err(ExtractError::Shape)
        );
        assert_eq!(
            schedule(serde_json::json!({ "date": "2026-02-30" })),
            Err(ExtractError::DateTime)
        );
        assert_eq!(
            schedule(serde_json::json!({ "time": "25:00" })),
            Err(ExtractError::DateTime)
        );
        assert_eq!(
            schedule(serde_json::json!({ "title": "  " })),
            Err(ExtractError::Title)
        );
        assert_eq!(
            schedule(serde_json::json!({ "title": "一二三四五六七八九十一二三" })),
            Err(ExtractError::Title)
        );
        assert!(
            schedule(serde_json::json!({ "title": " 一二三四五六七八九十一二 " }))
                .unwrap()
                .is_some()
        );
        assert_eq!(
            schedule(serde_json::json!({ "location": "很".repeat(LOCATION_MAX + 1) })),
            Err(ExtractError::Shape)
        );
    }

    #[test]
    fn validates_todo_route_and_title_limit() {
        assert!(
            validate_todo_json(r#"{"is_todo":false,"title":null,"due_date":null}"#)
                .unwrap()
                .is_none()
        );
        let todo = validate_todo_json(
            "```json\n{\"is_todo\":true,\"title\":\" 打印简历 \",\"due_date\":\"2026-10-09\"}\n```",
        )
        .unwrap()
        .unwrap();
        assert_eq!(todo.title, "打印简历");
        assert_eq!(todo.due_date.as_deref(), Some("2026-10-09"));
        assert_eq!(todo.source, "ai");
        assert_eq!(
            validate_todo_json(
                r#"{"is_todo":true,"title":"这是一条超过十六个汉字长度的待办事项标题","due_date":null}"#
            ),
            Err(ExtractError::Title)
        );
        assert_eq!(
            validate_todo_json(r#"{"is_todo":true,"title":"打印简历","due_date":"10月9日"}"#),
            Err(ExtractError::DateTime)
        );
    }
}
