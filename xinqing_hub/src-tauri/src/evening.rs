//! 启动晚间小结服务（C-10，FR-REV-01）并实现它用的 [`EveningPort`]：设置与“今天出过了”读写 `settings` 表，
//! 统计从只读连接读，显示推 `review:evening`（ADR 0029）。

use std::sync::Arc;

use chrono::{DateTime, Local, NaiveDate, NaiveTime};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use xinqing_hub_core::domain::evening::{self, DayFacts, EveningSummary, EveningTemplates};
use xinqing_hub_core::domain::settings;
use xinqing_hub_core::evening::{EveningPort, EveningService};
use xinqing_hub_core::infra::store::Db;

use crate::events::ReviewEvening;
use crate::paths;
use crate::sensing::Sensing;
use crate::state::AppState;

/// 须在 `AppState`、`Sensing` 都托管之后调用。模板加载失败时不启动（记日志），其余功能不受影响。
pub fn start(app: &AppHandle) {
    let templates =
        match paths::hub_template_dirs().and_then(|dirs| Ok(EveningTemplates::load(&dirs)?)) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("晚间小结不可用：{e}");
                return;
            }
        };
    let bus = app.state::<Sensing>().bus.subscribe();
    let service = EveningService::new(
        Arc::new(ShellPort { app: app.clone() }),
        crate::sim::clock(),
        templates,
    );
    tauri::async_runtime::spawn(service.run(bus));
}

struct ShellPort {
    app: AppHandle,
}

impl ShellPort {
    fn setting(&self, key: &str) -> Option<settings::SettingValue> {
        settings::get(&self.app.state::<AppState>().db(), key).ok()
    }
}

impl EveningPort for ShellPort {
    fn enabled(&self) -> bool {
        self.setting("review.evening.enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }

    fn at(&self) -> NaiveTime {
        evening::parse_time(
            self.setting("review.evening.time")
                .as_ref()
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        )
    }

    fn shown_on(&self) -> Option<NaiveDate> {
        let s = self
            .app
            .state::<AppState>()
            .db()
            .settings_get(evening::SHOWN_ON_KEY)
            .ok()??;
        NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()
    }

    fn mark_shown(&self, day: NaiveDate) {
        let v = day.format("%Y-%m-%d").to_string();
        if let Err(e) = self
            .app
            .state::<AppState>()
            .writer()
            .write_sync(|db| db.settings_set(evening::SHOWN_ON_KEY, &v))
        {
            eprintln!("记录晚间小结失败：{e}");
        }
    }

    fn facts(&self, day: NaiveDate, now: DateTime<Local>) -> Option<DayFacts> {
        let state = self.app.state::<AppState>();
        read_facts(&state.db(), day, now)
            .inspect_err(|e| eprintln!("读取当天统计失败：{e}"))
            .ok()
    }

    fn show(&self, s: &EveningSummary) {
        if let Err(e) = ReviewEvening(s.clone()).emit(&self.app) {
            eprintln!("推送 review:evening 失败：{e}");
        }
    }

    fn note(&self, msg: &str) {
        eprintln!("{msg}");
    }
}

/// 一天到 `now` 为止的统计（ADR 0029 第 3 条）。过了午夜才出时，那天的日程全部算到点。
fn read_facts(
    db: &Db,
    day: NaiveDate,
    now: DateTime<Local>,
) -> Result<DayFacts, xinqing_hub_core::infra::store::StoreError> {
    let date = day.format("%Y-%m-%d").to_string();
    let (typing_min, rests_due, rests_done, water) = db.summary_counts(&date)?.unwrap_or_default();
    let until = if now.date_naive() > day {
        "24:00".to_string()
    } else {
        now.format("%H:%M").to_string()
    };
    let (from, to) = evening::day_range_ms(day);
    let states = db
        .shown_states_since(from)?
        .into_iter()
        .filter(|(ts, _)| *ts < to)
        .collect();
    Ok(DayFacts {
        typing_min,
        water,
        rests_done,
        rests_due,
        schedules_done: db.schedules_done_on(&date, &until)?,
        todos_done: db.todos_done_between(from, to)?,
        states,
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use xqp::MoodState;

    use super::*;

    #[test]
    fn facts_come_from_the_right_day() {
        let db = Db::open_in_memory().unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let ts = |h: u32| {
            Local
                .with_ymd_and_hms(2026, 10, 5, h, 0, 0)
                .unwrap()
                .timestamp_millis()
        };
        db.summary_active_minute("2026-10-05", None, ts(9)).unwrap();
        db.summary_rest("2026-10-05", true, false).unwrap();
        db.summary_rest("2026-10-05", false, true).unwrap();
        db.insert_mood_state(
            ts(9),
            None,
            MoodState::Low,
            MoodState::Low,
            xinqing_hub_core::domain::fusion::Source::Rule,
        )
        .unwrap();
        // 第二天的记录不算
        db.insert_mood_state(
            ts(9) + 86_400_000,
            None,
            MoodState::Tired,
            MoodState::Tired,
            xinqing_hub_core::domain::fusion::Source::Rule,
        )
        .unwrap();
        let now = Local.with_ymd_and_hms(2026, 10, 5, 22, 30, 0).unwrap();
        let f = read_facts(&db, day, now).unwrap();
        assert_eq!(
            (f.typing_min, f.rests_due, f.rests_done, f.water),
            (1, 1, 1, 1)
        );
        assert_eq!(f.states, [(ts(9), MoodState::Low)]);
        assert_eq!((f.schedules_done, f.todos_done), (0, 0));
    }
}
