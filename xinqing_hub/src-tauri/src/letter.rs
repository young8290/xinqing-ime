//! 启动周信服务（C-10，FR-REV-02）并实现它用的 [`LetterPort`]：设置与“处理过哪一周”读写 `settings` 表，
//! 统计从只读连接读，信同步写进 `letter` 表，推 `letter:new`（ADR 0030）。

use std::sync::Arc;

use chrono::{DateTime, Local, NaiveDate, TimeZone};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use xinqing_hub_core::domain::comfort::Style;
use xinqing_hub_core::domain::consent::{ConsentItem, ConsentState};
use xinqing_hub_core::domain::evening;
use xinqing_hub_core::domain::letter::{self, DayCounts, LetterFallback, LetterPrompt, WeekFacts};
use xinqing_hub_core::domain::routine::{self, RoutineNight};
use xinqing_hub_core::domain::settings;
use xinqing_hub_core::domain::validate::BannedWords;
use xinqing_hub_core::infra::store::{Db, NewLetter, StoreError};
use xinqing_hub_core::letter::{Letter, LetterPort, LetterService};

use crate::events::LetterNew;
use crate::gateway::Ai;
use crate::paths;
use crate::state::AppState;

const DATE_FMT: &str = "%Y-%m-%d";

/// 须在 `AppState`、`Arc<Ai>` 都托管之后调用。模板加载失败时不启动（记日志），其余功能不受影响。
pub fn start(app: &AppHandle) {
    let loaded = paths::hub_template_dirs()
        .and_then(|dirs| {
            Ok((
                LetterPrompt::load(&dirs)?,
                LetterFallback::load(&dirs)?,
                BannedWords::load(&dirs)?,
            ))
        });
    let (prompt, fallback, banned) = match loaded {
        Ok(t) => t,
        Err(e) => {
            eprintln!("周信不可用：{e}");
            return;
        }
    };
    let service = LetterService::new(
        Arc::new(ShellPort { app: app.clone() }),
        app.state::<Arc<Ai>>().inner().clone(),
        crate::sim::clock(),
        prompt,
        fallback,
        Arc::new(banned),
    );
    tauri::async_runtime::spawn(service.run());
}

struct ShellPort {
    app: AppHandle,
}

impl ShellPort {
    fn setting(&self, key: &str) -> Option<settings::SettingValue> {
        settings::get(&self.app.state::<AppState>().db(), key).ok()
    }
}

impl LetterPort for ShellPort {
    fn enabled(&self) -> bool {
        self.setting("review.letter.enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }

    fn llm_allowed(&self) -> bool {
        ConsentState::load(&self.app.state::<AppState>().db())
            .is_ok_and(|c| c.granted(ConsentItem::LlmSummary))
    }

    fn style(&self) -> Style {
        Style::parse(
            self.setting("care.style")
                .as_ref()
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        )
    }

    fn last_week(&self) -> Option<NaiveDate> {
        let s = self
            .app
            .state::<AppState>()
            .db()
            .settings_get(letter::LAST_WEEK_KEY)
            .ok()??;
        NaiveDate::parse_from_str(&s, DATE_FMT).ok()
    }

    fn mark_week(&self, week: NaiveDate) {
        let v = week.format(DATE_FMT).to_string();
        if let Err(e) = self
            .app
            .state::<AppState>()
            .writer()
            .write_sync(|db| db.settings_set(letter::LAST_WEEK_KEY, &v))
        {
            eprintln!("记录周信进度失败：{e}");
        }
    }

    fn facts(&self, week: NaiveDate, now: DateTime<Local>) -> Option<WeekFacts> {
        read_week(&self.app.state::<AppState>().db(), week, now)
            .inspect_err(|e| eprintln!("读取一周统计失败：{e}"))
            .ok()
    }

    fn save(&self, l: &Letter) -> Option<i64> {
        let week = l.week_start.format(DATE_FMT).to_string();
        let row = NewLetter {
            week_start: &week,
            content: &l.content,
            source: if l.ai_generated { "llm" } else { "template" },
            model: l.model.as_deref(),
            prompt_ver: l.prompt_ver.as_deref(),
            ts: l.ts,
        };
        self.app
            .state::<AppState>()
            .writer()
            .write_sync(|db| db.letter_insert(&row))
            .inspect_err(|e| eprintln!("保存周信失败：{e}"))
            .ok()
    }

    fn show(&self, id: i64, l: &Letter) {
        let ev = LetterNew {
            id: id as u32,
            week_start: l.week_start.format(DATE_FMT).to_string(),
            ai_generated: l.ai_generated,
        };
        if let Err(e) = ev.emit(&self.app) {
            eprintln!("推送 letter:new 失败：{e}");
        }
    }

    fn note(&self, msg: &str) {
        eprintln!("{msg}");
    }
}

/// 那周（周一 `week` 起 7 天）到 `now` 为止的统计（ADR 0030 第 3 条）。
fn read_week(db: &Db, week: NaiveDate, now: DateTime<Local>) -> Result<WeekFacts, StoreError> {
    let days: Vec<NaiveDate> = week.iter_days().take(7).collect();
    let from = evening::day_range_ms(week).0;
    let to = evening::day_range_ms(days[6]).1.min(now.timestamp_millis());
    let mut counts = Vec::with_capacity(7);
    let mut schedules_done = 0;
    for d in &days {
        let date = d.format(DATE_FMT).to_string();
        let (typing_min, rests_due, rests_done, water) =
            db.summary_counts(&date)?.unwrap_or_default();
        counts.push(DayCounts {
            typing_min,
            rests_due,
            rests_done,
            water,
        });
        let until = match now.date_naive() {
            today if *d < today => "24:00".to_string(),
            today if *d == today => now.format("%H:%M").to_string(),
            _ => continue,
        };
        schedules_done += db.schedules_done_on(&date, &until)?;
    }
    let states = db
        .shown_states_since(from)?
        .into_iter()
        .filter(|(ts, _)| *ts < to)
        .collect();
    // 作息只看周一到周六晚：周日 20:00 写信时周日晚还没结束
    let nights = &days[..6];
    let rows = db.summary_stop_times(
        &nights[0].format(DATE_FMT).to_string(),
        &nights[5].format(DATE_FMT).to_string(),
    )?;
    let nights = nights
        .iter()
        .map(|d| {
            let date = d.format(DATE_FMT).to_string();
            let stop_min = rows
                .iter()
                .find(|(r, _)| *r == date)
                .and_then(|(_, ts)| Local.timestamp_millis_opt(*ts).single())
                .and_then(|t| routine::stop_minute(*d, t));
            RoutineNight {
                date,
                stop_ts: None,
                stop_min,
                late: stop_min.is_some_and(|m| m >= routine::LATE_FROM_MIN),
            }
        })
        .collect();
    let r = routine::summarize(nights);
    Ok(WeekFacts {
        days: counts,
        states,
        schedules_done,
        todos_done: db.todos_done_between(from, to)?,
        self_reports: db.self_reports_between(from, to)?.len() as u32,
        avg_stop_min: r.avg_stop_min,
        late_nights: r.late_nights,
    })
}

#[cfg(test)]
mod tests {
    use xqp::MoodState;

    use super::*;

    #[test]
    fn a_week_of_facts() {
        let db = Db::open_in_memory().unwrap();
        let week = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let at = |d: u32, h: u32, m: u32| Local.with_ymd_and_hms(2026, 10, d, h, m, 0).unwrap();
        // 周一白天打字、周二晚上 00:30 才停（算熬夜），下一周一的不算
        routine::record_active_minute(&db, at(5, 9, 0)).unwrap();
        routine::record_active_minute(&db, at(7, 0, 30)).unwrap();
        db.summary_rest("2026-10-05", true, false).unwrap();
        db.insert_mood_state(
            at(5, 9, 0).timestamp_millis(),
            None,
            MoodState::Low,
            MoodState::Low,
            xinqing_hub_core::domain::fusion::Source::Rule,
        )
        .unwrap();
        db.insert_mood_state(
            at(12, 9, 0).timestamp_millis(),
            None,
            MoodState::Tired,
            MoodState::Tired,
            xinqing_hub_core::domain::fusion::Source::Rule,
        )
        .unwrap();
        let f = read_week(&db, week, at(11, 20, 0)).unwrap();
        assert_eq!(f.days.len(), 7);
        assert_eq!(f.days[0].typing_min, 1);
        assert_eq!(f.days[0].rests_due, 1);
        assert_eq!(f.states.len(), 1, "下一周的状态不算");
        assert_eq!(f.late_nights, 1);
        assert_eq!(f.avg_stop_min, Some(24 * 60 + 30));
        assert_eq!((f.schedules_done, f.todos_done, f.self_reports), (0, 0, 0));
    }
}
