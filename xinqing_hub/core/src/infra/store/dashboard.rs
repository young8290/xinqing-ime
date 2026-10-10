//! 看板与小组件底栏的只读查询（07 FR-DSH-02～04、FR-WGT-04/05，docs/adr/0036）。统计本身在 `domain::dashboard`。

use rusqlite::{OptionalExtension, params};
use xqp::MoodState;

use super::{Db, StoreError, parse_state};

/// `comfort_log` 的一行，给看板“今日一句”和小组件的“今日一句”用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComfortRow {
    pub id: i64,
    pub ts: i64,
    pub text: String,
    /// `llm` / `template`
    pub source: String,
    /// `auto` / `self_report`
    pub trigger: String,
    /// `useful` / `unfit` / `mute`
    pub feedback: Option<String>,
}

impl Db {
    /// `[from, to)` 之间的状态记录 `(行号, Unix 毫秒, 显示状态)`，从旧到新；认不出的状态跳过。
    pub fn mood_points_between(
        &self,
        from: i64,
        to: i64,
    ) -> Result<Vec<(i64, i64, MoodState)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ts, shown_state FROM mood_state WHERE ts >= ?1 AND ts < ?2 ORDER BY ts, id",
        )?;
        let rows = stmt.query_map([from, to], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, ts, s) = row?;
            if let Some(state) = parse_state(&s) {
                out.push((id, ts, state));
            }
        }
        Ok(out)
    }

    /// `[from, to)` 之间的暖心话，从旧到新。
    pub fn comforts_between(&self, from: i64, to: i64) -> Result<Vec<ComfortRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ts, text, source, trigger, feedback FROM comfort_log
             WHERE ts >= ?1 AND ts < ?2 ORDER BY ts, id",
        )?;
        let rows = stmt.query_map([from, to], |r| {
            Ok(ComfortRow {
                id: r.get(0)?,
                ts: r.get(1)?,
                text: r.get(2)?,
                source: r.get(3)?,
                trigger: r.get(4)?,
                feedback: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// `[from, to]` 两个本地日期（`YYYY-MM-DD`，含两端）之间写过日记的日期，去重、按先后。
    pub fn diary_dates_between(&self, from: &str, to: &str) -> Result<Vec<String>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT date FROM diary WHERE date BETWEEN ?1 AND ?2 ORDER BY date",
        )?;
        let rows = stmt.query_map([from, to], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// `[from, to)` 之间新建、且现在是“已添加”的日程数（周报“本周新增日程”）。
    pub fn schedules_added_between(&self, from: i64, to: i64) -> Result<u32, StoreError> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM schedule WHERE status = 'added' AND created_ts >= ?1 AND created_ts < ?2",
            params![from, to],
            |r| r.get(0),
        )?)
    }

    /// 一天的专注分钟数（`daily_summary.focus_min`，FR-RST-10）；没有这一天时为 0。
    pub fn summary_focus_min(&self, date: &str) -> Result<u32, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT focus_min FROM daily_summary WHERE date = ?1",
                [date],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::fusion::Source;
    use crate::infra::store::ComfortRecord;

    #[test]
    fn mood_points_are_bounded_and_ordered() {
        let db = Db::open_in_memory().unwrap();
        for (ts, s) in [
            (30, MoodState::Low),
            (10, MoodState::Fluent),
            (50, MoodState::Tired),
        ] {
            db.insert_mood_state(ts, None, s, s, Source::Rule).unwrap();
        }
        let got: Vec<_> = db
            .mood_points_between(10, 50)
            .unwrap()
            .into_iter()
            .map(|(_, ts, s)| (ts, s))
            .collect();
        assert_eq!(got, [(10, MoodState::Fluent), (30, MoodState::Low)]);
    }

    #[test]
    fn comforts_carry_trigger_and_feedback() {
        let db = Db::open_in_memory().unwrap();
        let rec = |ts, trigger| ComfortRecord {
            ts,
            state: "low",
            text: "慢慢来。",
            source: "template",
            template_id: Some("gl01"),
            model: None,
            prompt_ver: None,
            trigger,
        };
        let a = db.comfort_insert(&rec(5, "auto")).unwrap();
        db.comfort_insert(&rec(9, "self_report")).unwrap();
        db.comfort_insert(&rec(100, "auto")).unwrap();
        db.comfort_set_feedback(a, "useful").unwrap();
        let got = db.comforts_between(0, 100).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].feedback.as_deref(), Some("useful"));
        assert_eq!(got[1].trigger, "self_report");
        assert_eq!(got[1].feedback, None);
    }

    #[test]
    fn diary_dates_schedules_and_focus() {
        let db = Db::open_in_memory().unwrap();
        db.diary_insert("2026-10-05", "a", "manual", 1).unwrap();
        db.diary_insert("2026-10-05", "b", "manual", 2).unwrap();
        db.diary_insert("2026-11-01", "c", "manual", 3).unwrap();
        assert_eq!(
            db.diary_dates_between("2026-10-01", "2026-10-31").unwrap(),
            ["2026-10-05"]
        );
        assert_eq!(db.schedules_added_between(0, i64::MAX).unwrap(), 0);
        assert_eq!(db.summary_focus_min("2026-10-05").unwrap(), 0);
    }
}
