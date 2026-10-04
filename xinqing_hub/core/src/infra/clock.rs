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
