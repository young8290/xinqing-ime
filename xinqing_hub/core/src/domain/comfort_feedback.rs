//! 暖心话的反馈（C-04 第二部分：05 FR-CMF-05、FR-CMF-06 第 2 条，docs/adr/0015 第 9–12 条）。
//!
//! - 👍 有用 / 👎 不合适 写 `comfort_log.feedback`；模板句被 👎 后不再选它（`Db::comfort_blocked_templates`）；
//! - 🔕 今天先别说了：今天剩余时间不再主动关怀（休息提醒、自评回应不受影响），到期时间存内部键 [`MUTED_UNTIL_KEY`]；
//! - 连续 3 天的反馈都是 👎 或 🔕：主动关怀频率自动降一档（最低降到“少一些”），界面告知“我会少打扰你一些”。
//!   降档后只看降档之后的反馈，避免同一批反馈连降几档（降档时间存内部键 [`REDUCED_TS_KEY`]）。

use std::collections::BTreeMap;

use chrono::{DateTime, Days, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

use super::comfort::CareLevel;
use super::settings::{self, SettingsError};
use crate::infra::store::Db;

/// 🔕 的到期时间（Unix 毫秒）。内部键，不在设置键注册表里（`settings::INTERNAL_KEYS`）。
pub const MUTED_UNTIL_KEY: &str = "care.muted_until";
/// 上一次自动降档的时间（Unix 毫秒）。内部键。
pub const REDUCED_TS_KEY: &str = "care.reduced_ts";
/// 连续几天负反馈后降档（FR-CMF-06 第 2 条）。
pub const REDUCE_DAYS: u64 = 3;

/// 一句暖心话上的反馈（`comfort_log.feedback`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ComfortVerdict {
    /// 👍 有用
    Useful,
    /// 👎 不合适
    Unfit,
    /// 🔕 今天先别说了
    Mute,
}

impl ComfortVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            ComfortVerdict::Useful => "useful",
            ComfortVerdict::Unfit => "unfit",
            ComfortVerdict::Mute => "mute",
        }
    }

    pub fn negative(self) -> bool {
        self != ComfortVerdict::Useful
    }
}

/// 记一次反馈的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    /// 这句暖心话还在（没有被清理）
    pub found: bool,
    /// 这次反馈让主动关怀频率自动降到了哪一档
    pub reduced_to: Option<CareLevel>,
}

/// 记一条反馈；必要时设置 🔕 和自动降档。`now` 是反馈的时刻。
pub fn record(
    db: &Db,
    comfort_id: i64,
    verdict: ComfortVerdict,
    now: DateTime<Local>,
) -> Result<Outcome, SettingsError> {
    if !db.comfort_set_feedback(comfort_id, verdict.as_str())? {
        return Ok(Outcome {
            found: false,
            reduced_to: None,
        });
    }
    if verdict == ComfortVerdict::Mute {
        db.settings_set(MUTED_UNTIL_KEY, &end_of_day_ms(now).to_string())?;
    }
    let reduced_to = if verdict.negative() {
        maybe_reduce(db, now)?
    } else {
        None
    };
    Ok(Outcome {
        found: true,
        reduced_to,
    })
}

/// 🔕 生效到什么时候；没有设置或已过期时为 `None`。
pub fn muted_until(db: &Db, now_ms: i64) -> Option<i64> {
    read_ms(db, MUTED_UNTIL_KEY).filter(|until| *until > now_ms)
}

fn read_ms(db: &Db, key: &str) -> Option<i64> {
    db.settings_get(key).ok().flatten()?.parse().ok()
}

/// 今天、昨天、前天每天都有反馈，且每天的反馈全是 👎 / 🔕 时降一档。
fn maybe_reduce(db: &Db, now: DateTime<Local>) -> Result<Option<CareLevel>, SettingsError> {
    let today = now.date_naive();
    let first = today
        .checked_sub_days(Days::new(REDUCE_DAYS - 1))
        .unwrap_or(today);
    let since = local_midnight(first).max(read_ms(db, REDUCED_TS_KEY).map_or(i64::MIN, |t| t + 1));
    let rows = db.comfort_feedback_since(since)?;
    if !all_negative_days(&rows, first, today) {
        return Ok(None);
    }
    let level = settings::get(db, "care.level")?;
    let Some(lower) = level
        .as_str()
        .map(CareLevel::parse)
        .and_then(CareLevel::reduced)
    else {
        return Ok(None);
    };
    settings::set(db, "care.level", &lower.as_str().into())?;
    db.settings_set(REDUCED_TS_KEY, &now.timestamp_millis().to_string())?;
    Ok(Some(lower))
}

/// `rows` 是 `(暖心话的时间, 反馈)`：`first..=last` 的每一天都要有反馈，且全是负面。
fn all_negative_days(rows: &[(i64, String)], first: NaiveDate, last: NaiveDate) -> bool {
    let mut days: BTreeMap<NaiveDate, bool> = BTreeMap::new();
    for (ts, verdict) in rows {
        let Some(day) = Local
            .timestamp_millis_opt(*ts)
            .single()
            .map(|t| t.date_naive())
        else {
            continue;
        };
        let negative = verdict != "useful";
        days.entry(day)
            .and_modify(|all| *all &= negative)
            .or_insert(negative);
    }
    first
        .iter_days()
        .take_while(|d| *d <= last)
        .all(|d| days.get(&d) == Some(&true))
}

fn local_midnight(d: NaiveDate) -> i64 {
    d.and_hms_opt(0, 0, 0)
        .and_then(|t| Local.from_local_datetime(&t).earliest())
        .map_or(i64::MIN, |t| t.timestamp_millis())
}

/// 当天 24:00（即次日 00:00）的 Unix 毫秒。
fn end_of_day_ms(now: DateTime<Local>) -> i64 {
    now.date_naive()
        .succ_opt()
        .map(local_midnight)
        .unwrap_or_else(|| now.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::store::ComfortRecord;

    fn at(d: u32, h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, d, h, 0, 0).unwrap()
    }

    fn say(db: &Db, t: DateTime<Local>, template: Option<&str>) -> i64 {
        db.comfort_insert(&ComfortRecord {
            ts: t.timestamp_millis(),
            state: "low",
            text: "累了就歇一下，我在这儿。",
            source: if template.is_some() {
                "template"
            } else {
                "llm"
            },
            template_id: template,
            model: None,
            prompt_ver: None,
            trigger: "auto",
        })
        .unwrap()
    }

    fn level(db: &Db) -> String {
        settings::get(db, "care.level")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn tc_cmf_07_feedback_is_written_and_unfit_templates_are_blocked() {
        let db = Db::open_in_memory().unwrap();
        let a = say(&db, at(5, 10), Some("gl02"));
        let b = say(&db, at(5, 11), Some("gl03"));
        record(&db, a, ComfortVerdict::Useful, at(5, 10)).unwrap();
        record(&db, b, ComfortVerdict::Unfit, at(5, 11)).unwrap();
        assert_eq!(db.comfort_blocked_templates().unwrap(), ["gl03"]);
        let got: Vec<Option<String>> = db
            .conn()
            .prepare("SELECT feedback FROM comfort_log ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(got, [Some("useful".into()), Some("unfit".into())]);
        assert!(
            !record(&db, 999, ComfortVerdict::Useful, at(5, 12))
                .unwrap()
                .found
        );
    }

    #[test]
    fn mute_lasts_until_midnight() {
        let db = Db::open_in_memory().unwrap();
        let id = say(&db, at(5, 23), None);
        record(&db, id, ComfortVerdict::Mute, at(5, 23)).unwrap();
        let midnight = at(6, 0).timestamp_millis();
        assert_eq!(
            muted_until(&db, at(5, 23).timestamp_millis()),
            Some(midnight)
        );
        assert_eq!(muted_until(&db, midnight), None, "过了零点就恢复");
    }

    #[test]
    fn three_negative_days_reduce_one_level_once() {
        let db = Db::open_in_memory().unwrap();
        for d in [3, 4] {
            let id = say(&db, at(d, 15), None);
            record(&db, id, ComfortVerdict::Unfit, at(d, 15)).unwrap();
        }
        assert_eq!(level(&db), "normal", "两天不够");
        let id = say(&db, at(5, 15), None);
        let out = record(&db, id, ComfortVerdict::Mute, at(5, 15)).unwrap();
        assert_eq!(out.reduced_to, Some(CareLevel::Less));
        assert_eq!(level(&db), "less");
        // 同一天再点：降档之后还没有新的 3 天，不再降
        let id = say(&db, at(5, 16), None);
        assert_eq!(
            record(&db, id, ComfortVerdict::Unfit, at(5, 16))
                .unwrap()
                .reduced_to,
            None
        );
        assert_eq!(level(&db), "less");
    }

    #[test]
    fn a_useful_day_or_a_gap_breaks_the_streak() {
        let db = Db::open_in_memory().unwrap();
        // 3 号有一条 👍：那天不算全负面
        let a = say(&db, at(3, 10), None);
        record(&db, a, ComfortVerdict::Unfit, at(3, 10)).unwrap();
        let b = say(&db, at(3, 11), None);
        record(&db, b, ComfortVerdict::Useful, at(3, 11)).unwrap();
        for d in [4, 5] {
            let id = say(&db, at(d, 15), None);
            record(&db, id, ComfortVerdict::Unfit, at(d, 15)).unwrap();
        }
        assert_eq!(level(&db), "normal");
        // 7 号的负反馈：6 号没有反馈，中断
        let id = say(&db, at(7, 15), None);
        assert_eq!(
            record(&db, id, ComfortVerdict::Unfit, at(7, 15))
                .unwrap()
                .reduced_to,
            None
        );
    }

    #[test]
    fn never_reduces_below_less() {
        let db = Db::open_in_memory().unwrap();
        settings::set(&db, "care.level", &"less".into()).unwrap();
        for d in [3, 4, 5] {
            let id = say(&db, at(d, 15), None);
            assert_eq!(
                record(&db, id, ComfortVerdict::Unfit, at(d, 15))
                    .unwrap()
                    .reduced_to,
                None
            );
        }
        assert_eq!(level(&db), "less");
    }
}
