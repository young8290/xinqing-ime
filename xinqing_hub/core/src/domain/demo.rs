//! 演示模式（07 FR-DMO-03，B-10，ADR 0023）：独立演示数据库的预置数据与加速倍数。
//!
//! 每次以演示模式启动，外壳都删掉旧的演示库、建新库并调用 [`seed`]，所以每场演示看到的都是同一份“过去一周”：
//! - 7 天 × 45 个特征窗口（09:00–23:40 每 20 分钟一个），足够越过冷启动，个人基线照常由启动时的重算得出；
//! - 每个窗口一条状态记录，大部分流畅，每天穿插一段犹豫 / 低落 / 烦躁 / 疲惫，供情绪日历和周报显示；
//! - 每天的使用时长、休息提醒次数、饮水次数、当晚停止打字时间（作息洞察 FR-REV-03）；
//! - 3 条自评天气；两条待确认日程（明天 14:00、后天截止）。
//!
//! 数值都是编造的，不来自任何真实用户；不写任何文本内容（自评不带备注）。

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, TimeZone, Timelike};
use xqp::MoodState;

use crate::domain::features::WindowFeatures;
use crate::domain::fusion::Source;
use crate::domain::rules::Hints;
use crate::domain::schedule::ScheduleDraft;
use crate::infra::store::demo::SummarySeed;
use crate::infra::store::{Db, StoreError};
use crate::infra::templates::{AppCat, BaselineDefault};

/// 演示模式的时间加速倍数（FR-DMO-03 第 1 条）。
pub const SPEED: u32 = 60;
/// 演示库文件名，与真实库 `xinqing.db` 放在同一个数据目录。
pub const DB_FILE: &str = "xinqing_demo.db";
/// 预置多少天。
pub const DAYS: u32 = 7;

const WINDOW_EVERY_MIN: u32 = 20;
const FIRST_MIN: u32 = 9 * 60;
const LAST_MIN: u32 = 23 * 60 + 40;

/// 预置结果，写日志用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Seeded {
    pub windows: u32,
    pub schedules: u32,
}

/// 第 k 天（0 = 最早的一天）的特殊时段：(起始分钟, 结束分钟, 状态)。
fn episodes(k: u32) -> &'static [(u32, u32, MoodState)] {
    use MoodState::*;
    match k {
        0 => &[(14 * 60, 15 * 60, Hesitant)],
        1 => &[(22 * 60 + 40, 24 * 60, Tired)],
        2 => &[(10 * 60, 11 * 60, Agitated)],
        3 => &[(20 * 60, 21 * 60 + 30, Low)],
        5 => &[(15 * 60, 16 * 60, Hesitant), (23 * 60, 24 * 60, Tired)],
        6 => &[(9 * 60, 10 * 60, Low)],
        _ => &[],
    }
}

/// 每天的 (使用分钟, 提醒次数, 完成次数, 饮水次数, 当晚停止打字时间距当晚 0 点的分钟数)。
const DAILY: [(u32, u32, u32, u32, u32); DAYS as usize] = [
    (185, 6, 4, 2, 22 * 60 + 40),
    (210, 7, 5, 3, 23 * 60 + 40),
    (160, 5, 2, 1, 23 * 60 + 10),
    (140, 4, 4, 2, 21 * 60 + 30),
    (230, 8, 6, 3, 22 * 60 + 50),
    (250, 7, 3, 2, 24 * 60 + 30),
    (170, 5, 4, 2, 23 * 60 + 40),
];

/// 可复现的伪随机数，落在 [-1, 1]。
fn jitter(day: u32, i: u32, salt: u32) -> f64 {
    let mut x = (day.wrapping_mul(7919) ^ i.wrapping_mul(104_729) ^ salt.wrapping_mul(1_299_709))
        .wrapping_add(0x9e37_79b9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85eb_ca6b);
    x ^= x >> 13;
    f64::from(x % 2001) / 1000.0 - 1.0
}

fn local(date: NaiveDate, minute: u32) -> Option<DateTime<Local>> {
    let naive = date.and_time(NaiveTime::MIN) + Duration::minutes(i64::from(minute));
    Local.from_local_datetime(&naive).earliest()
}

fn features(
    base: &BaselineDefault,
    t: DateTime<Local>,
    state: MoodState,
    k: u32,
    i: u32,
) -> WindowFeatures {
    let bucket = if (6..22).contains(&t.hour()) {
        &base.day
    } else {
        &base.night
    };
    let med = |name: &str, fallback: f64| bucket.get(name).map_or(fallback, |m| m.med);
    let (kpm_f, iki_f, bs_f) = match state {
        MoodState::Hesitant => (0.7, 1.5, 1.2),
        MoodState::Low => (0.7, 1.35, 1.0),
        MoodState::Agitated => (1.3, 0.85, 1.8),
        MoodState::Tired => (0.65, 1.3, 1.6),
        _ => (1.0, 1.0, 1.0),
    };
    let wobble = |salt: u32| 1.0 + 0.12 * jitter(k, i, salt);
    let kpm = med("kpm", 220.0) * kpm_f * wobble(1);
    WindowFeatures {
        n_keys: kpm.round() as u32,
        active_ms: 50_000,
        kpm: Some(kpm),
        iki_med: Some(med("iki_med", 190.0) * iki_f * wobble(2)),
        iki_iqr: Some(med("iki_iqr", 160.0) * wobble(3)),
        dwell_med: Some(med("dwell_med", 95.0) * wobble(4)),
        bs_rate: med("bs_rate", 0.08) * bs_f * wobble(5),
        session_min: 20.0,
        hour: t.hour(),
        minute_of_day: t.hour() * 60 + t.minute(),
        ..Default::default()
    }
}

/// 往空的演示库里写入“过去一周”（截至 `now` 的前一天）和两条待确认日程。在一个事务里完成。
pub fn seed(db: &Db, now: DateTime<Local>, base: &BaselineDefault) -> Result<Seeded, StoreError> {
    db.seed_tx(|db| {
        let mut out = Seeded::default();
        let today = now.date_naive();
        for k in 0..DAYS {
            let date = today - Duration::days(i64::from(DAYS - k));
            for (i, minute) in (FIRST_MIN..=LAST_MIN)
                .step_by(WINDOW_EVERY_MIN as usize)
                .enumerate()
            {
                let Some(end) = local(date, minute) else {
                    continue;
                };
                let state = episodes(k)
                    .iter()
                    .find(|(from, to, _)| (*from..*to).contains(&minute))
                    .map_or(MoodState::Fluent, |e| e.2);
                let f = features(base, end, state, k, i as u32);
                let end_ms = end.timestamp_millis();
                let cat = [AppCat::Chat, AppCat::Doc, AppCat::Browser][i % 3];
                let id = db.insert_window(end_ms - 60_000, end_ms, cat, &f, &Hints::default())?;
                db.insert_mood_state(end_ms, Some(id), state, state, Source::Rule)?;
                out.windows += 1;
            }
            let (typing_min, due, done, water, stop) = DAILY[k as usize];
            let date_s = date.format("%Y-%m-%d").to_string();
            db.summary_seed(&SummarySeed {
                date: &date_s,
                typing_min,
                rests_due: due,
                rests_done: done,
                water,
                last_active_ts: local(date, stop).map(|t| t.timestamp_millis()),
            })?;
        }
        for (k, minute, weather, auto) in [
            (1, 23 * 60 + 20, "night", "tired"),
            (3, 20 * 60 + 30, "rain", "low"),
            (5, 15 * 60 + 30, "cloudy", "hesitant"),
        ] {
            let date = today - Duration::days(i64::from(DAYS - k));
            if let Some(t) = local(date, minute) {
                db.insert_self_report(t.timestamp_millis(), weather, None, Some(auto), "user")?;
            }
        }
        let created = (now - Duration::hours(1)).timestamp_millis();
        let day = |n: i64| (today + Duration::days(n)).format("%Y-%m-%d").to_string();
        let drafts = [
            ScheduleDraft {
                title: "小组讨论".into(),
                date: Some(day(1)),
                time: Some("14:00".into()),
                end_time: None,
                all_day: false,
                location: Some("图书馆".into()),
                is_deadline: false,
                remind_offsets: vec![600],
                source: "ai".into(),
                flags: Vec::new(),
            },
            ScheduleDraft {
                title: "交实验报告".into(),
                date: Some(day(2)),
                time: None,
                end_time: None,
                all_day: true,
                location: None,
                is_deadline: true,
                remind_offsets: vec![600],
                source: "ai".into(),
                flags: Vec::new(),
            },
        ];
        for d in &drafts {
            if db.schedule_create(d, "pending", created)?.1 {
                out.schedules += 1;
            }
        }
        Ok(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::features::persist;
    use crate::domain::routine;
    use crate::infra::templates::TemplateDirs;

    fn base() -> BaselineDefault {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates");
        BaselineDefault::load(&TemplateDirs::factory_only(dir)).unwrap()
    }

    fn noon() -> DateTime<Local> {
        let naive = NaiveDate::from_ymd_opt(2026, 10, 12)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        Local.from_local_datetime(&naive).earliest().unwrap()
    }

    #[test]
    fn seeded_week_leaves_cold_start_and_feeds_the_dashboard() {
        let db = Db::open_in_memory().unwrap();
        let now = noon();
        let s = seed(&db, now, &base()).unwrap();
        assert_eq!(
            s,
            Seeded {
                windows: 7 * 45,
                schedules: 2
            }
        );

        // 启动时的重算：越过冷启动，白天和夜间两个桶都有个人值
        let stats = persist::recompute(&db, now.timestamp_millis()).unwrap();
        assert!(stats.windows >= crate::domain::features::baseline::COLD_START_WINDOWS);
        assert!(stats.rows.iter().any(|r| r.bucket.as_str() == "night"));

        // 作息洞察：7 晚都有记录，第 6 晚过了午夜
        let r = routine::get(&db, 7, now).unwrap();
        assert_eq!(r.counted_nights, 7);
        assert_eq!(r.late_nights, 1);
        assert_eq!(r.nights[5].stop_min, Some(24 * 60 + 30));

        assert_eq!(db.schedules_by_status("pending").unwrap().len(), 2);
        assert_eq!(db.self_reports_recent(10).unwrap().len(), 3);
    }

    #[test]
    fn seeding_is_reproducible() {
        let (a, b) = (Db::open_in_memory().unwrap(), Db::open_in_memory().unwrap());
        seed(&a, noon(), &base()).unwrap();
        seed(&b, noon(), &base()).unwrap();
        let ra = persist::recompute(&a, noon().timestamp_millis()).unwrap();
        let rb = persist::recompute(&b, noon().timestamp_millis()).unwrap();
        assert_eq!(ra.rows, rb.rows);
    }

    #[test]
    fn jitter_stays_in_range() {
        for d in 0..7 {
            for i in 0..45 {
                assert!((-1.0..=1.0).contains(&jitter(d, i, 3)));
            }
        }
    }
}
