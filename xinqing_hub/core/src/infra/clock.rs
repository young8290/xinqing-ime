//! 统一时间来源（03 第 4 节）。演示模式和回放通过替换 `Clock` 实现时间加速或指定起始时刻。

use std::sync::Mutex;

use chrono::{DateTime, Duration, Local, NaiveDateTime, TimeZone, Timelike};

pub trait Clock: Send + Sync {
    /// 当前本地时间。
    fn now(&self) -> DateTime<Local>;

    /// 当前 Unix 毫秒。
    fn now_ms(&self) -> i64 {
        self.now().timestamp_millis()
    }

    /// 当前本地时间的“分钟数”（0–1439），规则 R5 等使用。
    fn minute_of_day(&self) -> u32 {
        let n = self.now();
        n.hour() * 60 + n.minute()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Local> {
        Local::now()
    }
}

/// 回放与测试用的手动时钟：时间只随 `set` / `advance` 改变。
#[derive(Debug)]
pub struct ManualClock {
    now: Mutex<DateTime<Local>>,
}

impl ManualClock {
    pub fn new(start: DateTime<Local>) -> Self {
        Self {
            now: Mutex::new(start),
        }
    }

    /// 以本地日期时间创建（例如 xq-sim `--start-at 23:40`）。
    pub fn at_local(naive: NaiveDateTime) -> Self {
        let start = Local
            .from_local_datetime(&naive)
            .earliest()
            .unwrap_or_else(Local::now);
        Self::new(start)
    }

    pub fn set(&self, t: DateTime<Local>) {
        *self.now.lock().unwrap() = t;
    }

    pub fn advance_ms(&self, ms: i64) {
        let mut g = self.now.lock().unwrap();
        *g += Duration::milliseconds(ms);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Local> {
        *self.now.lock().unwrap()
    }
}

/// 系统时间加一个固定偏移，照常走动。Hub 调试构建的 `XQ_SIM_START_AT`（FR-DMO-01
/// `--start-at`）用它把“现在”拨到指定时刻，各服务共用同一份（ADR 0020）。
#[derive(Debug, Clone, Copy)]
pub struct OffsetClock {
    offset: Duration,
}

impl OffsetClock {
    /// 让创建这一刻的 `now()` 等于 `start`。
    pub fn starting_at(start: DateTime<Local>) -> Self {
        Self {
            offset: start - Local::now(),
        }
    }
}

impl Clock for OffsetClock {
    fn now(&self) -> DateTime<Local> {
        Local::now() + self.offset
    }
}

/// 把 `HH:MM` 解析成 `today` 那天的本地时刻（`--start-at` / `XQ_SIM_START_AT`）。
/// 格式不对或该时刻因夏令时不存在时返回 `None`。
pub fn today_at(hhmm: &str, today: chrono::NaiveDate) -> Option<DateTime<Local>> {
    let t = chrono::NaiveTime::parse_from_str(hhmm.trim(), "%H:%M").ok()?;
    Local.from_local_datetime(&today.and_time(t)).earliest()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_clock_starts_at_given_time_and_keeps_running() {
        let start = Local::now() - Duration::hours(5);
        let c = OffsetClock::starting_at(start);
        let drift = (c.now() - start).num_milliseconds();
        assert!((0..1_000).contains(&drift), "drift {drift}");
    }

    #[test]
    fn today_at_parses_hh_mm() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
        let t = today_at("23:40", day).unwrap();
        assert_eq!((t.date_naive(), t.hour(), t.minute()), (day, 23, 40));
        assert!(today_at("24:00", day).is_none());
        assert!(today_at("2340", day).is_none());
    }
}
