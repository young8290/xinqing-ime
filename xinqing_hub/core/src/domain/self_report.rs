//! 主动报告心情（自评天气，FR-STA-10）。
//!
//! 自评不经过融合（17 第 2.4 节）：之后 60 分钟内（或到下一次自评为止）显示用户自评的天气，
//! 自动判断照常在后台运行和记录。显示覆盖由感知任务维护（`SenseCmd::SelfReport`），这里只管
//! 写 `self_report` 表（09 D-24）、按天列出，以及判断是否要校准个人阈值。
//!
//! 校准（FR-STA-10 第 3 条“差异连续 3 次以上时按 FR-STA-06 第 8 条调整”）：最近连续的自评都与
//! 同一时刻的自动判断不同，每满 3 次，就把这一次被否定的自动状态的阈值上调一档。
//! “说不上来”不算差异也不打断连续；自动判断是流畅时不上调（流畅没有可调的阈值）。
//!
//! 备注只存本地，永不出网（NFR-PRI-09）；这里不打日志、不进总线。

use chrono::{Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};
use xqp::MoodState;

use crate::infra::store::{Db, StoreError};

/// 自评的显示覆盖时长。
pub const OVERRIDE_MS: i64 = 60 * 60_000;
/// 备注最多几个字（09 `self_report.note` 的长度约束）。
pub const NOTE_MAX_CHARS: usize = 50;
/// 连续几次与自动判断不同时校准一次。
pub const MISMATCH_STREAK: usize = 3;

/// 自评选项（`self_report.weather`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum SelfWeather {
    /// ☀️ 挺好
    Sunny,
    /// ⛅ 有点犹豫
    Cloudy,
    /// 🌧 有点低落
    Rain,
    /// ⛈ 有点烦
    Storm,
    /// 🌙 有点累
    Night,
    /// 🤷 说不上来
    Unsure,
}

impl SelfWeather {
    pub const ALL: [SelfWeather; 6] = [
        SelfWeather::Sunny,
        SelfWeather::Cloudy,
        SelfWeather::Rain,
        SelfWeather::Storm,
        SelfWeather::Night,
        SelfWeather::Unsure,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SelfWeather::Sunny => "sunny",
            SelfWeather::Cloudy => "cloudy",
            SelfWeather::Rain => "rain",
            SelfWeather::Storm => "storm",
            SelfWeather::Night => "night",
            SelfWeather::Unsure => "unsure",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.as_str() == s)
    }

    /// 对应的显示状态；“说不上来”没有对应状态，不覆盖显示。
    pub fn state(self) -> Option<MoodState> {
        match self {
            SelfWeather::Sunny => Some(MoodState::Fluent),
            SelfWeather::Cloudy => Some(MoodState::Hesitant),
            SelfWeather::Rain => Some(MoodState::Low),
            SelfWeather::Storm => Some(MoodState::Agitated),
            SelfWeather::Night => Some(MoodState::Tired),
            SelfWeather::Unsure => None,
        }
    }

    /// 负面自评：晴晴要立即回应（FR-STA-10 第 2 条，由暖心话服务订阅总线处理）。
    pub fn is_negative(self) -> bool {
        matches!(
            self,
            SelfWeather::Cloudy | SelfWeather::Rain | SelfWeather::Storm | SelfWeather::Night
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SelfReportError {
    #[error("备注超过 {NOTE_MAX_CHARS} 字")]
    NoteTooLong,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// 一次自评写库的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recorded {
    pub id: i64,
    /// 显示覆盖到期时间（Unix 毫秒）；“说不上来”为 `ts`，即不覆盖。
    pub until_ms: i64,
    /// 需要上调阈值的自动状态（连续差异满 3 次）。
    pub raise: Option<MoodState>,
}

/// `self_report_list` 的一行（10 第 5.1 节）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SelfReportItem {
    pub id: u32,
    /// Unix 毫秒。用 f64 是因为前端绑定不导出 i64，毫秒时间戳在 f64 中是精确的。
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub ts: f64,
    pub weather: SelfWeather,
    /// 只存本地（NFR-PRI-09）
    pub note: Option<String>,
    /// 同一时刻的自动判断
    pub auto_state: Option<MoodState>,
}

/// 库里的一行：(id, ts, weather, note, auto_state)。
pub type SelfReportRow = (i64, i64, String, Option<String>, Option<String>);

fn parse_state(s: &str) -> Option<MoodState> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
}

/// 这条自评是否与同一时刻的自动判断不同；`None` 表示不参与比较（说不上来）。
fn mismatch(weather: SelfWeather, auto: Option<MoodState>) -> Option<bool> {
    let mine = weather.state()?;
    Some(auto.is_some_and(|a| a != mine))
}

/// 自评的来源（D-24 `self_report.source`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportSource {
    /// 用户主动报告
    User,
    /// 研究模式的定时邀请（FR-DMO-04）
    Esm,
}

impl ReportSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ReportSource::User => "user",
            ReportSource::Esm => "esm",
        }
    }
}

/// 删掉研究模式邀请得到的全部自评（研究结束时，FR-DMO-04），返回删了几条。用户主动的自评不动。
pub fn delete_esm(db: &Db) -> Result<usize, StoreError> {
    db.self_reports_delete_source(ReportSource::Esm.as_str())
}

/// 记一次用户自评。`auto_state` 是此刻自动判断的显示状态（还没有窗口时为 `None`）。
/// 备注去掉首尾空白，空备注按没填处理。
pub fn record(
    db: &Db,
    weather: SelfWeather,
    note: Option<&str>,
    auto_state: Option<MoodState>,
    ts: i64,
) -> Result<Recorded, SelfReportError> {
    record_from(db, weather, note, auto_state, ts, ReportSource::User)
}

/// 同 [`record`]，指明来源：研究模式邀请后的回答记 [`ReportSource::Esm`]。
pub fn record_from(
    db: &Db,
    weather: SelfWeather,
    note: Option<&str>,
    auto_state: Option<MoodState>,
    ts: i64,
    source: ReportSource,
) -> Result<Recorded, SelfReportError> {
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    if note.is_some_and(|n| n.chars().count() > NOTE_MAX_CHARS) {
        return Err(SelfReportError::NoteTooLong);
    }
    let id = db.insert_self_report(
        ts,
        weather.as_str(),
        note,
        auto_state.map(MoodState::as_str),
        source.as_str(),
    )?;

    let mut streak = 0;
    for (_, _, w, _, auto) in db.self_reports_recent(64)? {
        let Some(w) = SelfWeather::parse(&w) else {
            continue;
        };
        match mismatch(w, auto.as_deref().and_then(parse_state)) {
            None => continue,
            Some(true) => streak += 1,
            Some(false) => break,
        }
    }
    let raise = match (mismatch(weather, auto_state), auto_state) {
        (Some(true), Some(a)) if a != MoodState::Fluent && streak % MISMATCH_STREAK == 0 => Some(a),
        _ => None,
    };
    let until_ms = if weather.state().is_some() {
        ts + OVERRIDE_MS
    } else {
        ts
    };
    Ok(Recorded {
        id,
        until_ms,
        raise,
    })
}

/// 本地日期 `date` 当天的自评，按时间先后。
pub fn list_day(db: &Db, date: NaiveDate) -> Result<Vec<SelfReportItem>, StoreError> {
    let bound = |d: NaiveDate| {
        d.and_hms_opt(0, 0, 0)
            .and_then(|t| Local.from_local_datetime(&t).earliest())
            .map(|t| t.timestamp_millis())
    };
    let (Some(start), Some(end)) = (bound(date), date.succ_opt().and_then(bound)) else {
        return Ok(Vec::new());
    };
    Ok(db
        .self_reports_between(start, end)?
        .into_iter()
        .filter_map(|(id, ts, weather, note, auto)| {
            Some(SelfReportItem {
                id: u32::try_from(id).ok()?,
                ts: ts as f64,
                weather: SelfWeather::parse(&weather)?,
                note,
                auto_state: auto.as_deref().and_then(parse_state),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_maps_to_states() {
        assert_eq!(SelfWeather::Night.state(), Some(MoodState::Tired));
        assert_eq!(SelfWeather::Unsure.state(), None);
        assert!(SelfWeather::Rain.is_negative());
        assert!(!SelfWeather::Sunny.is_negative() && !SelfWeather::Unsure.is_negative());
        for w in SelfWeather::ALL {
            assert_eq!(SelfWeather::parse(w.as_str()), Some(w));
            assert_eq!(serde_json::to_value(w).unwrap(), w.as_str());
        }
    }

    #[test]
    fn note_is_trimmed_and_limited() {
        let db = Db::open_in_memory().unwrap();
        let r = record(&db, SelfWeather::Night, Some("  "), None, 1_000).unwrap();
        assert_eq!(r.until_ms, 1_000 + OVERRIDE_MS);
        let long = "字".repeat(NOTE_MAX_CHARS + 1);
        assert!(matches!(
            record(&db, SelfWeather::Night, Some(&long), None, 2_000),
            Err(SelfReportError::NoteTooLong)
        ));
        let ok = "字".repeat(NOTE_MAX_CHARS);
        record(
            &db,
            SelfWeather::Rain,
            Some(&ok),
            Some(MoodState::Fluent),
            3_000,
        )
        .unwrap();
        let rows = db.self_reports_recent(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].3.as_deref(), Some(ok.as_str()));
        assert_eq!(rows[0].4.as_deref(), Some("fluent"));
        assert_eq!(rows[1].3, None, "空白备注按没填处理");
    }

    #[test]
    fn unsure_does_not_override() {
        let db = Db::open_in_memory().unwrap();
        let r = record(&db, SelfWeather::Unsure, None, Some(MoodState::Low), 5).unwrap();
        assert_eq!((r.until_ms, r.raise), (5, None));
    }

    #[test]
    fn three_mismatches_in_a_row_raise_the_auto_state() {
        let db = Db::open_in_memory().unwrap();
        let tired = Some(MoodState::Tired);
        // 自动说“累”，用户三次说“挺好”，中间一次“说不上来”不打断
        assert_eq!(
            record(&db, SelfWeather::Sunny, None, tired, 1)
                .unwrap()
                .raise,
            None
        );
        assert_eq!(
            record(&db, SelfWeather::Sunny, None, tired, 2)
                .unwrap()
                .raise,
            None
        );
        record(&db, SelfWeather::Unsure, None, tired, 3).unwrap();
        assert_eq!(
            record(&db, SelfWeather::Sunny, None, tired, 4)
                .unwrap()
                .raise,
            Some(MoodState::Tired)
        );
        // 第 4 次不再上调，满 6 次再上调一档
        assert_eq!(
            record(&db, SelfWeather::Sunny, None, tired, 5)
                .unwrap()
                .raise,
            None
        );
        // 一致的自评打断连续
        record(&db, SelfWeather::Night, None, tired, 6).unwrap();
        assert_eq!(
            record(&db, SelfWeather::Sunny, None, tired, 7)
                .unwrap()
                .raise,
            None
        );
        assert_eq!(
            record(&db, SelfWeather::Sunny, None, tired, 8)
                .unwrap()
                .raise,
            None
        );
        assert_eq!(
            record(&db, SelfWeather::Sunny, None, tired, 9)
                .unwrap()
                .raise,
            Some(MoodState::Tired)
        );
    }

    #[test]
    fn fluent_auto_state_is_never_raised() {
        let db = Db::open_in_memory().unwrap();
        let fluent = Some(MoodState::Fluent);
        for t in 0..3 {
            assert_eq!(
                record(&db, SelfWeather::Rain, None, fluent, t)
                    .unwrap()
                    .raise,
                None
            );
        }
        // 没有自动判断时不比较
        assert_eq!(
            record(&db, SelfWeather::Rain, None, None, 9).unwrap().raise,
            None
        );
    }

    #[test]
    fn list_by_local_day() {
        let db = Db::open_in_memory().unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let at = |h: u32| {
            Local
                .from_local_datetime(&day.and_hms_opt(h, 0, 0).unwrap())
                .earliest()
                .unwrap()
                .timestamp_millis()
        };
        record(&db, SelfWeather::Night, Some("测试备注"), None, at(23)).unwrap();
        record(
            &db,
            SelfWeather::Sunny,
            None,
            Some(MoodState::Fluent),
            at(9),
        )
        .unwrap();
        record(&db, SelfWeather::Rain, None, None, at(0) - 1).unwrap();
        let items = list_day(&db, day).unwrap();
        assert_eq!(items.len(), 2, "前一天 23:59:59 的不算");
        assert_eq!(items[0].weather, SelfWeather::Sunny);
        assert_eq!(items[0].ts, at(9) as f64);
        assert_eq!(items[0].auto_state, Some(MoodState::Fluent));
        assert_eq!(items[1].note.as_deref(), Some("测试备注"));
    }
    #[test]
    fn esm_answers_are_tagged_and_deleted_alone() {
        let db = Db::open_in_memory().unwrap();
        record(&db, SelfWeather::Sunny, None, None, 1).unwrap();
        record_from(&db, SelfWeather::Rain, None, None, 2, ReportSource::Esm).unwrap();
        record_from(&db, SelfWeather::Cloudy, None, None, 3, ReportSource::Esm).unwrap();
        assert_eq!(delete_esm(&db).unwrap(), 2);
        let left = db.self_reports_recent(10).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].2, "sunny");
    }
}
