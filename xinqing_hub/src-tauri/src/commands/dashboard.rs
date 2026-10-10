//! 看板与小组件用的只读命令（D-07、FR-WGT-04/05，docs/adr/0036）：今日概要与时间线、情绪日历、周报、当天的暖心话。
//! 统计在 `xinqing_hub_core::domain::dashboard`，这里只取参、读库、组装。都只读，不扣 AI 额度、不出网。

use std::collections::HashSet;

use chrono::{Datelike, Duration, NaiveDate};
use serde::Serialize;
use specta::Type;
use tauri::State;
use xinqing_hub_core::care::ComfortTrigger;
use xinqing_hub_core::domain::comfort_feedback::ComfortVerdict;
use xinqing_hub_core::domain::dashboard::{
    self, DayMood, DayStats, LineFacts, WeekDay, WeekReport, WeeklyLines,
};
use xinqing_hub_core::domain::evening;
use xinqing_hub_core::domain::letter;
use xinqing_hub_core::infra::store::Db;
use xqp::MoodState;

use crate::error::UiError;
use crate::paths;
use crate::sim;
use crate::state::AppState;

const DATE_FMT: &str = "%Y-%m-%d";

fn parse_date(s: &str) -> Result<NaiveDate, UiError> {
    NaiveDate::parse_from_str(s, DATE_FMT).map_err(|_| UiError::new("arg.date", "error.generic"))
}

/// 一句暖心话（看板“今日一句”、小组件重开时的“今日一句”）。时间戳是 Unix 毫秒。
#[derive(Debug, Clone, Serialize, Type)]
pub struct ComfortItem {
    pub id: u32,
    #[specta(type = specta_typescript::Number)]
    pub ts: f64,
    pub text: String,
    /// 为真时句尾显示 `AI 生成`（FR-CMF-04 第 2 条）
    pub ai_generated: bool,
    pub trigger: ComfortTrigger,
    /// 用户点过的反馈（FR-CMF-05）；没点过为空
    pub feedback: Option<ComfortVerdict>,
}

fn verdict(s: &str) -> Option<ComfortVerdict> {
    serde_json::from_value(serde_json::Value::String(s.into())).ok()
}

/// 某天（本地日期 `YYYY-MM-DD`）的暖心话，从早到晚（FR-DSH-02“今日一句”、FR-WGT-04“今日一句”）。
#[tauri::command]
#[specta::specta]
pub fn comfort_list(state: State<'_, AppState>, date: String) -> Result<Vec<ComfortItem>, UiError> {
    let (from, to) = evening::day_range_ms(parse_date(&date)?);
    Ok(state
        .db()
        .comforts_between(from, to)?
        .into_iter()
        .map(|c| ComfortItem {
            id: u32::try_from(c.id).unwrap_or(0),
            ts: c.ts as f64,
            ai_generated: c.source == "llm",
            trigger: if c.trigger == "self_report" {
                ComfortTrigger::SelfReport
            } else {
                ComfortTrigger::Auto
            },
            feedback: c.feedback.as_deref().and_then(verdict),
            text: c.text,
        })
        .collect())
}

/// 某天的概要（FR-DSH-02 概要卡片与状态时间线、FR-WGT-05 今日输入时长）。没有记录的天全是 0。
#[tauri::command]
#[specta::specta]
pub fn day_stats(state: State<'_, AppState>, date: String) -> Result<DayStats, UiError> {
    let day = parse_date(&date)?;
    Ok(read_day(&state.db(), day)?)
}

fn read_day(db: &Db, day: NaiveDate) -> Result<DayStats, xinqing_hub_core::infra::store::StoreError> {
    let date = day.format(DATE_FMT).to_string();
    let (typing_min, rests_due, rests_done, water) = db.summary_counts(&date)?.unwrap_or_default();
    let (from, to) = evening::day_range_ms(day);
    let points = db.mood_points_between(from, to)?;
    let states: Vec<MoodState> = points.iter().map(|(_, _, s)| *s).collect();
    Ok(DayStats {
        typing_min,
        rests_due,
        rests_done,
        water,
        comforts: db.comforts_between(from, to)?.len() as u32,
        dominant: dashboard::dominant(&states),
        timeline: dashboard::timeline(&points),
        date,
    })
}

/// 情绪日历的一个月（FR-DSH-03）：`month` 是 `YYYY-MM`，返回这个月每天的主导天气与有没有日记。
#[tauri::command]
#[specta::specta]
pub fn month_moods(state: State<'_, AppState>, month: String) -> Result<Vec<DayMood>, UiError> {
    let first = parse_date(&format!("{month}-01"))?;
    let next = first
        .checked_add_months(chrono::Months::new(1))
        .ok_or_else(|| UiError::new("arg.date", "error.generic"))?;
    let last = next - Duration::days(1);
    let db = state.db();
    let (from, _) = evening::day_range_ms(first);
    let (to, _) = evening::day_range_ms(next);
    let points: Vec<(i64, MoodState)> = db
        .mood_points_between(from, to)?
        .into_iter()
        .map(|(_, ts, s)| (ts, s))
        .collect();
    let diaries: HashSet<String> = db
        .diary_dates_between(&first.format(DATE_FMT).to_string(), &last.format(DATE_FMT).to_string())?
        .into_iter()
        .collect();
    Ok(dashboard::month(first, &points, &diaries))
}

/// 周报（FR-DSH-04）：`week_start` 是那周里的任意一天，按所在周的周一算。作息洞察另用 `get_routine`。
/// 一句话总结来自本地模板 `weekly_line.toml`，不调用 AI；模板读不出来时用一句兜底。
#[tauri::command]
#[specta::specta]
pub fn week_stats(state: State<'_, AppState>, week_start: String) -> Result<WeekReport, UiError> {
    let week = letter::week_start(parse_date(&week_start)?);
    let now = sim::clock().now();
    let db = state.db();
    let facts = crate::letter::read_week(&db, week, now)?;
    let stats = letter::stats(&facts);
    let mut days = Vec::with_capacity(7);
    let mut focus_min = 0;
    for (d, c) in week.iter_days().zip(&facts.days) {
        let date = d.format(DATE_FMT).to_string();
        let focus = db.summary_focus_min(&date)?;
        focus_min += focus;
        days.push(WeekDay {
            date,
            typing_min: c.typing_min,
            rests_due: c.rests_due,
            rests_done: c.rests_done,
            water: c.water,
            focus_min: focus,
        });
    }
    let states: Vec<MoodState> = facts.states.iter().map(|(_, s)| *s).collect();
    let heat = dashboard::heat(&facts.states);
    let line_facts = LineFacts {
        hard_hour: dashboard::hard_hour(&heat),
        hard_slot: stats.hard_slot.clone(),
        rest_rate: stats.rest_rate,
        fluent_share: stats.state_share.get("fluent").copied(),
    };
    let seed = u64::try_from(week.num_days_from_ce()).unwrap_or(0);
    let line = match paths::hub_template_dirs().and_then(|dirs| Ok(WeeklyLines::load(&dirs)?)) {
        Ok(lines) => lines.pick(&line_facts, seed),
        Err(e) => {
            eprintln!("周报一句话模板不可用：{e}");
            "又过完了一周，辛苦了。".into()
        }
    };
    let (from, _) = evening::day_range_ms(week);
    let (to, _) = evening::day_range_ms(week + Duration::days(7));
    Ok(WeekReport {
        week_start: week.format(DATE_FMT).to_string(),
        days,
        states: dashboard::state_counts(&states),
        heat,
        rest_rate: stats.rest_rate,
        rests_done: stats.rests_done,
        rests_due: stats.rests_due,
        water: stats.water,
        schedules_added: db.schedules_added_between(from, to)?,
        todos_done: stats.todos_done,
        focus_min,
        line,
    })
}

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone};
    use xinqing_hub_core::domain::fusion::Source;

    use super::*;

    #[test]
    fn day_reads_counts_states_and_comforts_of_that_day_only() {
        let db = Db::open_in_memory().unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let ts = |d: u32, h: u32, m: u32| {
            Local
                .with_ymd_and_hms(2026, 10, d, h, m, 0)
                .unwrap()
                .timestamp_millis()
        };
        db.summary_active_minute("2026-10-05", None, ts(5, 9, 0))
            .unwrap();
        for m in 0..10 {
            db.insert_mood_state(ts(5, 9, m), None, MoodState::Low, MoodState::Low, Source::Rule)
                .unwrap();
        }
        db.insert_mood_state(ts(6, 9, 0), None, MoodState::Tired, MoodState::Tired, Source::Rule)
            .unwrap();
        db.comfort_insert(&xinqing_hub_core::infra::store::ComfortRecord {
            ts: ts(5, 9, 5),
            state: "low",
            text: "慢慢来。",
            source: "template",
            template_id: None,
            model: None,
            prompt_ver: None,
            trigger: "auto",
        })
        .unwrap();
        let s = read_day(&db, day).unwrap();
        assert_eq!(s.date, "2026-10-05");
        assert_eq!((s.typing_min, s.comforts), (1, 1));
        assert_eq!(s.dominant, Some(MoodState::Low));
        assert_eq!(s.timeline.len(), 1);
        assert_eq!(s.timeline[0].state, MoodState::Low);
    }

    #[test]
    fn verdict_parses_stored_feedback() {
        assert_eq!(verdict("mute"), Some(ComfortVerdict::Mute));
        assert_eq!(verdict("nope"), None);
    }
}
