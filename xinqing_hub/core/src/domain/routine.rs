//! 作息洞察（05 FR-REV-03，B-09）与 `daily_summary` 中使用计时、休息提醒那几列的写入（FR-RST-08）。
//!
//! 每晚的**停止打字时间** = 当天 18:00 至次日 06:00 之间最后一个活跃输入分钟（FR-RST-01），
//! 存在“当晚”日期那一行的 `last_active_ts`（09 D-31）；晚于 00:00 记一次熬夜。
//! 界面一律称“停止打字时间”，并注明只统计这台电脑上的打字、不等于入睡时间（DS-COPY-09）。

use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

use crate::infra::store::{Db, StoreError};

/// 当晚时段的起点（18:00）与终点（次日 06:00），以当晚日期 0 点起算的分钟数。
pub const EVENING_START_MIN: u32 = 18 * 60;
pub const EVENING_END_MIN: u32 = 30 * 60;
/// 停止打字时间不早于这个时刻（次日 00:00）就算熬夜。
pub const LATE_FROM_MIN: u32 = 24 * 60;
/// `get_routine` 最多统计的晚数（看板用 7 / 30 天）。
pub const MAX_DAYS: u32 = 90;

const DATE_FMT: &str = "%Y-%m-%d";

/// `local` 落在 18:00–次日 06:00 时，返回它所属的“当晚”日期（凌晨算前一天）。
pub fn evening_of(local: DateTime<Local>) -> Option<NaiveDate> {
    let minute = local.hour() * 60 + local.minute();
    if minute >= EVENING_START_MIN {
        Some(local.date_naive())
    } else if minute + 24 * 60 < EVENING_END_MIN {
        local.date_naive().pred_opt()
    } else {
        None
    }
}

/// `local` 距当晚日期 `evening` 0 点的分钟数（18:00 = 1080，次日 01:30 = 1530）；不在当晚时段内时为 `None`。
pub fn stop_minute(evening: NaiveDate, local: DateTime<Local>) -> Option<u32> {
    let days = (local.date_naive() - evening).num_days();
    let minute = u32::try_from(days).ok()? * 24 * 60 + local.hour() * 60 + local.minute();
    (EVENING_START_MIN..EVENING_END_MIN)
        .contains(&minute)
        .then_some(minute)
}

/// 最近一个已经结束（过了次日 06:00）的当晚日期。
pub fn last_finished_evening(now: DateTime<Local>) -> NaiveDate {
    let today_night = (now - Duration::minutes(i64::from(EVENING_END_MIN - 24 * 60))).date_naive();
    today_night.pred_opt().unwrap_or(today_night)
}

/// 记一个活跃分钟（FR-RST-01，由休息提醒服务的计时调用）：当天的使用分钟加 1；在当晚时段内时更新停止打字时间。
pub fn record_active_minute(db: &Db, local: DateTime<Local>) -> Result<(), StoreError> {
    let date = local.date_naive().format(DATE_FMT).to_string();
    let evening = evening_of(local).map(|d| d.format(DATE_FMT).to_string());
    db.summary_active_minute(&date, evening.as_deref(), local.timestamp_millis())
}

/// 记一次休息提醒的显示（`shown`）或完成（`!shown`，喝水提醒的完成另记饮水次数），按本地日期（FR-RST-08）。
pub fn record_rest(
    db: &Db,
    local: DateTime<Local>,
    shown: bool,
    water: bool,
) -> Result<(), StoreError> {
    db.summary_rest(
        &local.date_naive().format(DATE_FMT).to_string(),
        shown,
        water,
    )
}

/// 一晚的停止打字时间（`get_routine` 的折线点）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RoutineNight {
    /// 当晚日期，本地 `YYYY-MM-DD`
    pub date: String,
    /// 停止打字时间，Unix 毫秒；这晚 18:00 后没有在这台电脑上打字时为 `null`
    pub stop_ts: Option<f64>,
    /// 停止打字时间距当晚日期 0 点的分钟数（18:00 = 1080，次日 01:30 = 1530），折线的纵轴
    pub stop_min: Option<u32>,
    /// 晚于 00:00
    pub late: bool,
}

/// 作息洞察（FR-REV-03）：最近若干晚的停止打字时间、平均停止时间、熬夜天数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Routine {
    /// 从早到晚，每晚一项（含没有记录的晚上），最后一项是最近一个已经结束的晚上
    pub nights: Vec<RoutineNight>,
    /// 有记录的晚上的平均停止时间，单位同 `stop_min`，四舍五入到分钟；一晚都没有时为 `null`
    pub avg_stop_min: Option<u32>,
    /// 熬夜天数
    pub late_nights: u32,
    /// 有记录的晚数
    pub counted_nights: u32,
}

/// 由各晚的记录汇总（纯计算，TC-REV-04 对它手算核对）。
pub fn summarize(nights: Vec<RoutineNight>) -> Routine {
    let mins: Vec<u32> = nights.iter().filter_map(|n| n.stop_min).collect();
    let counted_nights = mins.len() as u32;
    let avg_stop_min = (!mins.is_empty()).then(|| {
        let sum: u64 = mins.iter().map(|&m| u64::from(m)).sum();
        ((sum as f64) / f64::from(counted_nights)).round() as u32
    });
    let late_nights = nights.iter().filter(|n| n.late).count() as u32;
    Routine {
        nights,
        avg_stop_min,
        late_nights,
        counted_nights,
    }
}

/// `get_routine(days)`：截至最近一个已经结束的晚上，共 `days` 晚（1–[`MAX_DAYS`]）。
pub fn get(db: &Db, days: u32, now: DateTime<Local>) -> Result<Routine, StoreError> {
    let days = days.clamp(1, MAX_DAYS);
    let to = last_finished_evening(now);
    let from = to - Duration::days(i64::from(days - 1));
    let rows = db.summary_stop_times(
        &from.format(DATE_FMT).to_string(),
        &to.format(DATE_FMT).to_string(),
    )?;
    let nights = from
        .iter_days()
        .take(days as usize)
        .map(|d| {
            let date = d.format(DATE_FMT).to_string();
            let ts = rows.iter().find(|(r, _)| *r == date).map(|(_, ts)| *ts);
            let stop_min = ts
                .and_then(|ts| Local.timestamp_millis_opt(ts).single())
                .and_then(|t| stop_minute(d, t));
            RoutineNight {
                date,
                stop_ts: stop_min.and(ts).map(|ts| ts as f64),
                stop_min,
                late: stop_min.is_some_and(|m| m >= LATE_FROM_MIN),
            }
        })
        .collect();
    Ok(summarize(nights))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32, h: u32, m: u32) -> DateTime<Local> {
        let naive = NaiveDate::from_ymd_opt(2026, 10, day)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap();
        Local.from_local_datetime(&naive).earliest().unwrap()
    }

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    #[test]
    fn evening_window_bounds() {
        assert_eq!(evening_of(at(5, 17, 59)), None);
        assert_eq!(evening_of(at(5, 18, 0)), Some(date(5)));
        assert_eq!(evening_of(at(6, 0, 0)), Some(date(5)));
        assert_eq!(evening_of(at(6, 5, 59)), Some(date(5)));
        assert_eq!(evening_of(at(6, 6, 0)), None);
        assert_eq!(stop_minute(date(5), at(5, 18, 0)), Some(1080));
        assert_eq!(stop_minute(date(5), at(6, 1, 30)), Some(1530));
        assert_eq!(stop_minute(date(5), at(6, 6, 0)), None);
        assert_eq!(stop_minute(date(5), at(5, 12, 0)), None);
    }

    #[test]
    fn last_finished_evening_switches_at_six() {
        assert_eq!(last_finished_evening(at(5, 5, 59)), date(3));
        assert_eq!(last_finished_evening(at(5, 6, 0)), date(4));
        assert_eq!(last_finished_evening(at(5, 23, 0)), date(4));
    }

    /// TC-REV-04：注入一周停止打字时间，统计与手算一致。
    /// 10-01 22:40、10-02 23:55、10-03 00:20（次日）、10-04 无、10-05 01:10（次日）、10-06 21:00、10-07 00:00（次日）。
    /// 平均 = (1360 + 1435 + 1460 + 1510 + 1260 + 1440) / 6 = 8465 / 6 = 1410.83 → 1411（23:31）；
    /// 熬夜 3 晚（10-03、10-05、10-07；00:00 那一分钟已在午夜之后）。
    #[test]
    fn one_week_matches_hand_calculation() {
        let db = Db::open_in_memory().unwrap();
        let stops = [
            at(1, 22, 40),
            at(2, 23, 55),
            at(4, 0, 20),
            at(6, 1, 10),
            at(6, 21, 0),
            at(8, 0, 0),
        ];
        // 每晚停止前半小时也在打字，另加白天的一分钟
        for stop in stops {
            record_active_minute(&db, stop - Duration::minutes(30)).unwrap();
            record_active_minute(&db, stop).unwrap();
        }
        record_active_minute(&db, at(4, 10, 0)).unwrap();

        let r = get(&db, 7, at(8, 9, 0)).unwrap();
        let mins: Vec<_> = r
            .nights
            .iter()
            .map(|n| (n.date.as_str(), n.stop_min))
            .collect();
        assert_eq!(
            mins,
            vec![
                ("2026-10-01", Some(1360)),
                ("2026-10-02", Some(1435)),
                ("2026-10-03", Some(1460)),
                ("2026-10-04", None),
                ("2026-10-05", Some(1510)),
                ("2026-10-06", Some(1260)),
                ("2026-10-07", Some(1440)),
            ]
        );
        assert_eq!(r.avg_stop_min, Some(1411));
        assert_eq!(r.late_nights, 3);
        assert_eq!(r.counted_nights, 6);
        assert_eq!(
            r.nights[0].stop_ts,
            Some(at(1, 22, 40).timestamp_millis() as f64)
        );
        // 使用时长按自然日：10-04 有凌晨 00:20 和白天 10:00 两分钟，前一晚 23:50 那分钟算 10-03
        assert_eq!(db.summary_counts("2026-10-04").unwrap().unwrap().0, 2);
    }

    #[test]
    fn days_are_clamped_and_empty_is_null() {
        let db = Db::open_in_memory().unwrap();
        let r = get(&db, 0, at(8, 9, 0)).unwrap();
        assert_eq!(r.nights.len(), 1);
        assert_eq!(r.avg_stop_min, None);
        assert_eq!(get(&db, 1000, at(8, 9, 0)).unwrap().nights.len(), 90);
    }
}
