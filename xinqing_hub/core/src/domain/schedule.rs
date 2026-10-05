//! 日程与待办的本地单句初筛（FR-SCH-01、FR-SCH-12）。
//!
//! 原句仅保留在返回值中供后续显式同意后的单句确认使用；调用方不得持久化它。

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use regex::Regex;
use serde::Deserialize;

use crate::domain::validate::{self, BannedWords, Scene};
use crate::domain::when;
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
    /// V3：标题为空（超长的截断，ADR 0023）
    #[error("AI 抽取结果标题为空")]
    Title,
    /// V4：标题或地点里有原句没有的禁用词（ADR 0023）
    #[error("AI 抽取结果含禁用词")]
    Banned,
}

/// 日程标题上限，超出截断（FR-SCH-03 第 3 条、08 第 5 节 V3，ADR 0023）。
const SCHEDULE_TITLE_MAX: usize = 12;
/// 待办标题上限，超出截断（FR-SCH-12、08 第 5 节 V3，ADR 0023）。
const TODO_TITLE_MAX: usize = 16;
/// 地点上限：产品书没有规定，超过这个长度多半是模型把整句抄了进来（FR-SCH-04 补充规则 7）。
const LOCATION_MAX: usize = 40;
/// 截止类没有时刻时的默认时刻（FR-SCH-04 校验第 5 条）。
const DEADLINE_TIME: NaiveTime = NaiveTime::from_hms_opt(23, 59, 0).unwrap();
/// 晚于这么多天以后标注“请确认日期”（FR-SCH-04 校验第 3 条）。
const CONFIRM_DATE_DAYS: i64 = 365;

/// `schedule.flags` 的取值（09 D-12；`need_time` 与 E-EXTRACT 数据集一致）。
/// 模型的日期或时刻与代码按原句算出的不一致，已改用代码结果（FR-SCH-04 校验第 1 条）
pub const FLAG_ADJUSTED: &str = "adjusted";
/// 结果早于当前时刻，卡片标注“时间可能已过”（校验第 2 条）
pub const FLAG_MAYBE_PAST: &str = "maybe_past";
/// 晚于 365 天以后，卡片标注“请确认日期”（校验第 3 条）
pub const FLAG_CONFIRM_DATE: &str = "confirm_date";
/// 非截止类的全天日程，卡片提示补充时刻（时刻规则、补充规则 2、6）
pub const FLAG_NEED_TIME: &str = "need_time";

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

/// V3：去掉首尾空白后不能为空，超过 `max` 个字截断（FR-SCH-03 第 3 条，ADR 0023）。
fn checked_title(title: Option<String>, max: usize) -> Result<String, ExtractError> {
    let title = title.as_deref().map(str::trim).unwrap_or_default();
    if title.is_empty() {
        return Err(ExtractError::Title);
    }
    Ok(title.chars().take(max).collect::<String>().trim_end().to_owned())
}

/// V4：标题和地点取自用户原句，原句里本来就有的词（“按时吃药”）不拦，只拦模型自己带进来的（ADR 0023）。
fn check_banned(fields: &[&str], sentence: &str, banned: &BannedWords) -> Result<(), ExtractError> {
    if fields
        .iter()
        .any(|f| banned.find_new(f, sentence, Scene::Other).is_some())
    {
        return Err(ExtractError::Banned);
    }
    Ok(())
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


/// 校验并规范化 P-SCHEDULE 的输出（08 第 5 节 V1～V4、FR-SCH-03、FR-SCH-04 校验第 1～5 条）。
/// `sentence` 是送去抽取的那一句（只在内存里用，不落库），`now` 是本地时间；`has_event=false` 返回 `Ok(None)`。
///
/// 原句里能算出的日期和时刻一律以代码为准（[`when::parse`]），与模型不一致时标 `adjusted`。
/// 日期、时刻统一写成 `YYYY-MM-DD`、`HH:MM`，去重哈希（FR-SCH-06）才不会因为 `9:30` 和 `09:30` 算成两条。
pub fn validate_schedule_json(
    raw: &str,
    sentence: &str,
    now: NaiveDateTime,
    banned: &BannedWords,
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
    check_banned(
        &[title.as_str(), location.as_deref().unwrap_or_default()],
        sentence,
        banned,
    )?;
    let mut date = optional_date(output.date)?;
    let mut time = optional_time(output.time)?;
    let mut end_time = optional_time(output.end_time)?;

    // 校验第 1 条：原句能算出的以代码为准
    let (model_date, model_time) = (date, time);
    let code = when::parse(sentence, now);
    let is_deadline = output.is_deadline || code.deadline;
    date = code.date.or(date);
    if code.time.is_some() {
        (time, end_time) = (code.time, code.end_time.or(end_time));
        if code.date.is_none() {
            // 原句有钟点没说日子：按校验第 4 条取今天或明天，模型给的日期不算数
            date = None;
        }
    } else if code.date.is_some() || code.period_only {
        // 原句说了日子或时段却没有钟点：时刻为空、按全天处理（时刻规则、补充规则 2、6），模型补的钟点不算数
        (time, end_time) = (None, None);
    }

    // 校验第 5 条：截止类没有时刻默认 23:59。连日期也没有时不补，留给卡片让用户补充
    if is_deadline && date.is_some() && time.is_none() {
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
    // 只在模型给了值、而最后用的不是它时标 `adjusted`；模型留空由代码补上的不算
    let adjusted = model_date.is_some_and(|m| date != Some(m))
        || model_time.is_some_and(|m| time != Some(m));
    if adjusted {
        flags.push(FLAG_ADJUSTED.to_owned());
    }
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
    if time.is_none() {
        flags.push(FLAG_NEED_TIME.to_owned());
    }

    Ok(Some(ScheduleDraft {
        title,
        date: date.map(format_date),
        all_day: time.is_none(),
        time: time.map(format_time),
        end_time: end_time.map(format_time),
        location,
        is_deadline,
        remind_offsets: Vec::new(),
        source: "ai".into(),
        flags,
    }))
}

/// 校验并规范化 P-TODO 的输出（08 第 5 节 V1～V4、FR-SCH-12）。截止日期按 FR-SCH-04 的规则由代码从原句算，
/// 算得出时以代码为准。`is_todo=false` 返回 `Ok(None)`，不写入数据库。
pub fn validate_todo_json(
    raw: &str,
    sentence: &str,
    now: NaiveDateTime,
    banned: &BannedWords,
) -> Result<Option<TodoDraft>, ExtractError> {
    let output: TodoOutput = parse_output(raw)?;
    if !output.is_todo {
        return Ok(None);
    }
    let title = checked_title(output.title, TODO_TITLE_MAX)?;
    check_banned(&[title.as_str()], sentence, banned)?;
    let due_date = when::parse(sentence, now)
        .date
        .or(optional_date(output.due_date)?);
    Ok(Some(TodoDraft {
        title,
        due_date: due_date.map(format_date),
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

    fn banned() -> BannedWords {
        let dirs = TemplateDirs::factory_only(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        );
        BannedWords::load(&dirs).unwrap()
    }

    /// 没有日期和钟点的句子：代码算不出东西，结果全看模型的输出。
    const PLAIN: &str = "在实验楼开组会";

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

    fn schedule_for(
        sentence: &str,
        patch: serde_json::Value,
    ) -> Result<Option<ScheduleDraft>, ExtractError> {
        validate_schedule_json(&schedule_output(patch), sentence, now(), &banned())
    }

    fn schedule(patch: serde_json::Value) -> Result<Option<ScheduleDraft>, ExtractError> {
        schedule_for(PLAIN, patch)
    }

    #[test]
    fn schedule_strips_code_fence_and_normalizes_fields() {
        let raw = format!(
            "```json\n{}\n```",
            schedule_output(serde_json::json!({
                "title": " 组会 ", "time": "9:30", "end_time": "10:00", "location": " 实验楼 ",
            }))
        );
        let draft = validate_schedule_json(&raw, PLAIN, now(), &banned())
            .unwrap()
            .unwrap();
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
    fn code_date_and_time_win_over_model_and_mark_adjusted() {
        // 模型把“周五下午三点”算成了周四两点
        let draft = schedule_for(
            "好的，周五下午三点在实验楼开组会",
            serde_json::json!({ "date": "2026-10-08", "time": "14:00" }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            (draft.date.as_deref(), draft.time.as_deref()),
            (Some("2026-10-09"), Some("15:00"))
        );
        assert_eq!(draft.flags, [FLAG_ADJUSTED]);
        // 模型没给日期，代码补上：不算改了模型的结果
        let draft = schedule_for(
            "好的，周五下午三点在实验楼开组会",
            serde_json::json!({ "date": null }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(draft.date.as_deref(), Some("2026-10-09"));
        assert!(draft.flags.is_empty());
        // 只说了“明早”，模型自己补的钟点不算数
        let draft = schedule_for(
            "明早去体检，记得空腹",
            serde_json::json!({ "title": "体检", "date": "2026-10-04", "time": "08:00", "location": null }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(draft.time, None);
        assert_eq!(draft.flags, [FLAG_ADJUSTED, FLAG_NEED_TIME]);
        // 原句有截止说法，模型漏标也按截止处理
        let draft = schedule_for(
            "数模比赛报名明天截止",
            serde_json::json!({ "title": "数模报名", "date": null, "time": null, "location": null }),
        )
        .unwrap()
        .unwrap();
        assert!(draft.is_deadline);
        assert_eq!(draft.time.as_deref(), Some("23:59"));
    }

    /// E-EXTRACT 的 60 条（`eval/datasets/e_extract.jsonl`，今天 2026-10-03 10:00）：模型只给出标题和地点、
    /// 日期时刻全错时，代码校验后的日期、时刻、结束时刻、截止、全天、标记都要与标注一致（标记里的 `adjusted` 不比）。
    #[test]
    fn e_extract_dates_and_times_come_out_right_after_code_validation() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../eval/datasets/e_extract.jsonl");
        let text = std::fs::read_to_string(path).unwrap();
        let banned = banned();
        let mut wrong = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let row: serde_json::Value = serde_json::from_str(line).unwrap();
            let exp = &row["expected"];
            let model = serde_json::json!({
                "has_event": true, "title": exp["title"], "date": "2026-01-01", "time": "03:33",
                "end_time": null, "all_day": false, "location": exp["location"],
                "is_deadline": exp["is_deadline"],
            });
            let sentence = row["text"].as_str().unwrap();
            let got = validate_schedule_json(&model.to_string(), sentence, now(), &banned)
                .unwrap()
                .unwrap();
            let mut flags: Vec<&str> = got
                .flags
                .iter()
                .map(String::as_str)
                .filter(|f| *f != FLAG_ADJUSTED)
                .collect();
            flags.sort_unstable();
            let mut want: Vec<&str> = exp["flags"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_str().unwrap())
                .collect();
            want.sort_unstable();
            let ok = got.date.as_deref() == exp["date"].as_str()
                && got.time.as_deref() == exp["time"].as_str()
                && got.end_time.as_deref() == exp["end_time"].as_str()
                && got.all_day == exp["all_day"].as_bool().unwrap()
                && got.is_deadline == exp["is_deadline"].as_bool().unwrap()
                && flags == want;
            if !ok {
                wrong.push(format!(
                    "{} {sentence}: {:?} {:?} {:?} {flags:?}",
                    row["id"], got.date, got.time, got.end_time
                ));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn schedule_follows_fr_sch_04_examples() {
        // 明晚八点前交数据库作业 → 2026-10-04 20:00，截止
        let draft = schedule_for(
            "明晚八点前交数据库作业",
            serde_json::json!({
                "title": "交数据库作业", "date": "2026-10-04", "time": "20:00",
                "location": null, "is_deadline": true,
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            (draft.time.as_deref(), draft.is_deadline, draft.flags.len()),
            (Some("20:00"), true, 0)
        );
        // 下周二和室友去看电影 → 全天、提示补充时刻；模型把 all_day 填成 false 也按全天
        let draft = schedule_for(
            "下周二和室友去看电影",
            serde_json::json!({
                "title": "看电影", "date": "2026-10-06", "time": null, "location": null,
            }),
        )
        .unwrap()
        .unwrap();
        assert!(draft.all_day);
        assert_eq!(draft.time, None);
        assert_eq!(draft.flags, [FLAG_NEED_TIME]);
        // 这周五交报告 → 2026-10-02，截止类默认 23:59，标注时间可能已过
        let draft = schedule_for(
            "这周五交报告",
            serde_json::json!({
                "title": "交报告", "date": "2026-10-02", "time": null,
                "all_day": true, "location": null, "is_deadline": true,
            }),
        )
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
        assert_eq!(today_all_day.flags, [FLAG_NEED_TIME]);
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
    fn titles_are_truncated_and_banned_words_only_count_when_model_adds_them() {
        // 超长截断（FR-SCH-03 第 3 条，ADR 0023）
        let draft = schedule(serde_json::json!({ "title": "一二三四五六七八九十一二三" }))
            .unwrap()
            .unwrap();
        assert_eq!(draft.title, "一二三四五六七八九十一二");
        // 原句里就有的词照常保留
        let draft = schedule_for(
            "明天上午九点按时吃药",
            serde_json::json!({ "title": "按时吃药", "location": null }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(draft.title, "按时吃药");
        // 模型自己带进来的禁用词拦下
        assert_eq!(
            schedule_for(
                "明天上午九点去医院",
                serde_json::json!({ "title": "去医院治疗", "location": null }),
            ),
            Err(ExtractError::Banned)
        );
    }

    #[test]
    fn schedule_rejects_invalid_output() {
        let banned = banned();
        assert_eq!(
            validate_schedule_json("好的，我来抽取", PLAIN, now(), &banned),
            Err(ExtractError::Json)
        );
        assert_eq!(
            validate_schedule_json(r#"{"has_event":true,"title":"组会"}"#, PLAIN, now(), &banned),
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
            schedule(serde_json::json!({ "location": "很".repeat(LOCATION_MAX + 1) })),
            Err(ExtractError::Shape)
        );
    }

    #[test]
    fn validates_todo_route_title_and_due_date() {
        let banned = banned();
        let todo = |raw: &str, sentence: &str| validate_todo_json(raw, sentence, now(), &banned);
        assert!(
            todo(r#"{"is_todo":false,"title":null,"due_date":null}"#, "记得打印简历")
                .unwrap()
                .is_none()
        );
        let t = todo(
            "```json\n{\"is_todo\":true,\"title\":\" 打印简历 \",\"due_date\":\"2026-10-09\"}\n```",
            "记得打印简历",
        )
        .unwrap()
        .unwrap();
        assert_eq!(t.title, "打印简历");
        assert_eq!(t.due_date.as_deref(), Some("2026-10-09"));
        assert_eq!(t.source, "ai");
        // 截止日期以原句算出的为准（FR-SCH-12）
        let t = todo(
            r#"{"is_todo":true,"title":"交材料","due_date":"2026-10-08"}"#,
            "记得周五前交材料",
        )
        .unwrap()
        .unwrap();
        assert_eq!(t.due_date.as_deref(), Some("2026-10-09"));
        let t = todo(
            r#"{"is_todo":true,"title":"这是一条超过十六个汉字长度的待办事项标题","due_date":null}"#,
            "记得",
        )
        .unwrap()
        .unwrap();
        assert_eq!(t.title.chars().count(), TODO_TITLE_MAX);
        assert_eq!(
            todo(
                r#"{"is_todo":true,"title":"打印简历","due_date":"10月9日"}"#,
                "记得打印简历"
            ),
            Err(ExtractError::DateTime)
        );
    }
}
