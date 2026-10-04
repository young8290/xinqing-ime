//! 心晴性能埋点（11 NFR-PERF-01）：dev 构建里统计两组耗时，供 TC-PERF-01 对比心晴开 / 关。
//!
//! - 按键：`handle_key_event_policed` 整体（只计按下），心晴关着时也统计，作对照；
//! - 钩子：心晴每次挂在按键路径上的调用（`snap`、`before_key`、`after_key`、`on_commit`），
//!   只有装了 Tap 时才有样本。指标是单次调用 P99 ≤ 50 µs。
//!
//! 只在 dev 构建开启（`wind_config::variant::is_dev()`），环境变量 `XQ_PERF=1` / `0` 可强制开关。
//! 关着时每次调用只多一次已缓存的布尔判断。每满 [`REPORT_EVERY`] 个按键记一行 INFO 汇总
//! （累计值，只有计数和耗时，没有任何输入内容），所以对比开 / 关要各自重启核心后再打。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// 每多少个按键样本记一次汇总
pub(crate) const REPORT_EVERY: u64 = 2_000;

/// 每个 2 的幂区间再分 16 格：相对误差不超过 1/16
const SUB_BITS: u32 = 4;
const SUB: u64 = 1 << SUB_BITS;
/// 覆盖到 2^40 ns（约 18 分钟），够用
const BUCKETS: usize = ((40 - SUB_BITS + 1) as usize + 1) * SUB as usize;

fn bucket(ns: u64) -> usize {
    if ns < SUB {
        return ns as usize;
    }
    let e = 63 - ns.leading_zeros(); // ns 的最高位
    let sub = (ns >> (e - SUB_BITS)) & (SUB - 1);
    (((e - SUB_BITS + 1) as u64 * SUB + sub) as usize).min(BUCKETS - 1)
}

/// 该格的下界（ns）
fn lower(i: usize) -> u64 {
    let i = i as u64;
    if i < SUB {
        return i;
    }
    let e = i / SUB + u64::from(SUB_BITS) - 1;
    (1 << e) | ((i % SUB) << (e - u64::from(SUB_BITS)))
}

/// 无锁直方图：只用原子加，按键线程上不分配、不取锁
pub(crate) struct Histogram {
    buckets: [AtomicU64; BUCKETS],
    count: AtomicU64,
    max_ns: AtomicU64,
}

/// 一组耗时的汇总（微秒）
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Summary {
    pub count: u64,
    pub p50_us: f64,
    pub p99_us: f64,
    pub max_us: f64,
}

impl Histogram {
    pub(crate) const fn new() -> Self {
        Self {
            buckets: [const { AtomicU64::new(0) }; BUCKETS],
            count: AtomicU64::new(0),
            max_ns: AtomicU64::new(0),
        }
    }

    /// 记一个样本，返回记完之后的样本总数
    pub(crate) fn record(&self, ns: u64) -> u64 {
        self.buckets[bucket(ns)].fetch_add(1, Ordering::Relaxed);
        self.max_ns.fetch_max(ns, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub(crate) fn summary(&self) -> Summary {
        let counts: Vec<u64> = self
            .buckets
            .iter()
            .map(|b| b.load(Ordering::Relaxed))
            .collect();
        let total: u64 = counts.iter().sum();
        let pct = |q: f64| {
            if total == 0 {
                return 0.0;
            }
            let rank = ((total as f64) * q).ceil().max(1.0) as u64;
            let mut seen = 0;
            for (i, &c) in counts.iter().enumerate() {
                seen += c;
                if seen >= rank {
                    return lower(i) as f64 / 1000.0;
                }
            }
            0.0
        };
        Summary {
            count: total,
            p50_us: pct(0.50),
            p99_us: pct(0.99),
            max_us: self.max_ns.load(Ordering::Relaxed) as f64 / 1000.0,
        }
    }
}

static KEYS: Histogram = Histogram::new();
static HOOKS: Histogram = Histogram::new();

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| match std::env::var("XQ_PERF").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => wind_config::variant::is_dev(),
    })
}

/// 计时守卫：析构时把耗时记进对应的直方图
pub(crate) struct Timer {
    start: Instant,
    hist: &'static Histogram,
}

impl Drop for Timer {
    fn drop(&mut self) {
        let n = self.hist.record(self.start.elapsed().as_nanos() as u64);
        if std::ptr::eq(self.hist, &KEYS) && n.is_multiple_of(REPORT_EVERY) {
            report();
        }
    }
}

/// 按键整体计时；`key_down` 为假（松开等）时不计
pub(crate) fn key_timer(key_down: bool) -> Option<Timer> {
    (key_down && enabled()).then(|| Timer {
        start: Instant::now(),
        hist: &KEYS,
    })
}

/// 心晴钩子单次调用计时
pub(crate) fn hook_timer() -> Option<Timer> {
    enabled().then(|| Timer {
        start: Instant::now(),
        hist: &HOOKS,
    })
}

fn report() {
    let k = KEYS.summary();
    let h = HOOKS.summary();
    tracing::info!(
        "心晴性能（累计）：按键 {} 次 p50 {:.1} µs、p99 {:.1} µs、最长 {:.1} µs；钩子 {} 次 p50 {:.1} µs、p99 {:.1} µs、最长 {:.1} µs",
        k.count,
        k.p50_us,
        k.p99_us,
        k.max_us,
        h.count,
        h.p50_us,
        h.p99_us,
        h.max_us
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_monotonic_and_tight() {
        let mut prev = 0;
        for ns in [
            0u64,
            1,
            15,
            16,
            17,
            31,
            32,
            100,
            1_000,
            49_999,
            50_000,
            1_000_000,
            1 << 39,
        ] {
            let b = bucket(ns);
            assert!(b >= prev, "{ns}");
            prev = b;
            let lo = lower(b);
            assert!(lo <= ns, "{ns}: 下界 {lo}");
            assert!(ns - lo <= ns / SUB, "{ns}: 下界 {lo} 误差超过 1/16");
        }
        assert_eq!(bucket(u64::MAX), BUCKETS - 1);
    }

    #[test]
    fn percentiles() {
        let h = Histogram::new();
        assert_eq!(h.summary().count, 0);
        // 98 个 10 µs、1 个 40 µs、1 个 2 ms
        for _ in 0..98 {
            h.record(10_000);
        }
        h.record(40_000);
        assert_eq!(h.record(2_000_000), 100);
        let s = h.summary();
        assert_eq!(s.count, 100);
        assert!((9.4..=10.0).contains(&s.p50_us), "{s:?}");
        assert!((37.5..=40.0).contains(&s.p99_us), "{s:?}");
        assert_eq!(s.max_us, 2_000.0);
    }
}
