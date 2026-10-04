//! 内存指标与出网日志（08 FR-AIG-05 第 3 条、FR-AIG-08）。指标不落盘，Hub 重启即清空；
//! 出网日志条目经通道交给 Hub 写入 `net_log`。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use tokio::sync::mpsc::UnboundedSender;
use xinqing_hub_core::infra::clock::Clock;
use xinqing_hub_core::infra::gateway::NetLogEntry;

/// 每个接口保留的延迟样本数（算 P50 / P95 用）。
const LATENCY_SAMPLES: usize = 500;

/// 某个接口的调用统计（FR-AIG-08）。大模型按模型分开统计，`api` 形如 `llm/<模型>`。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiMetrics {
    pub api: String,
    pub calls: u64,
    pub ok: u64,
    /// 最近若干次成功调用的延迟中位数；还没有成功调用时为空
    pub p50_ms: Option<u64>,
    pub p95_ms: Option<u64>,
    pub breaker_open: bool,
}

#[derive(Debug, Default)]
struct Counter {
    calls: u64,
    ok: u64,
    latencies: VecDeque<u64>,
}

/// 最近邻排名法取百分位。
pub(crate) fn percentile(samples: impl Iterator<Item = u64>, p: f64) -> Option<u64> {
    let mut v: Vec<u64> = samples.collect();
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    let rank = ((p * v.len() as f64).ceil() as usize).clamp(1, v.len());
    Some(v[rank - 1])
}

/// 网关内各客户端共用的东西：时钟、指标、出网日志通道。
pub(crate) struct Shared {
    pub clock: Arc<dyn Clock>,
    counters: Mutex<HashMap<String, Counter>>,
    net_log: OnceLock<UnboundedSender<NetLogEntry>>,
}

impl Shared {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            counters: Mutex::new(HashMap::new()),
            net_log: OnceLock::new(),
        }
    }

    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// 只能设置一次；之后的调用忽略。
    pub fn set_net_log(&self, tx: UnboundedSender<NetLogEntry>) {
        let _ = self.net_log.set(tx);
    }

    /// 记一次调用：`api` 用指标的键（`jev`、`llm/<模型>`），失败的调用不计入延迟样本。
    pub fn record(&self, api: &str, ok: bool, latency_ms: u64) {
        let mut g = self.counters.lock().unwrap();
        let c = g.entry(api.to_string()).or_default();
        c.calls += 1;
        if ok {
            c.ok += 1;
            if c.latencies.len() == LATENCY_SAMPLES {
                c.latencies.pop_front();
            }
            c.latencies.push_back(latency_ms);
        }
    }

    /// 写一条出网日志；Hub 没接收时直接丢弃。
    pub fn log(&self, entry: NetLogEntry) {
        if let Some(tx) = self.net_log.get() {
            let _ = tx.send(entry);
        }
    }

    /// 按接口名排序的统计快照；`breaker_open` 由调用方给出。
    pub fn snapshot(&self, breaker_open: impl Fn(&str) -> bool) -> Vec<ApiMetrics> {
        let g = self.counters.lock().unwrap();
        let mut out: Vec<ApiMetrics> = g
            .iter()
            .map(|(api, c)| ApiMetrics {
                api: api.clone(),
                calls: c.calls,
                ok: c.ok,
                p50_ms: percentile(c.latencies.iter().copied(), 0.5),
                p95_ms: percentile(c.latencies.iter().copied(), 0.95),
                breaker_open: breaker_open(api),
            })
            .collect();
        out.sort_by(|a, b| a.api.cmp(&b.api));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xinqing_hub_core::infra::clock::SystemClock;

    #[test]
    fn percentile_nearest_rank() {
        assert_eq!(percentile([].into_iter(), 0.5), None);
        assert_eq!(percentile([7].into_iter(), 0.95), Some(7));
        assert_eq!(percentile([5, 1, 4, 2, 3].into_iter(), 0.5), Some(3));
        assert_eq!(percentile(1..=100, 0.95), Some(95));
    }

    #[test]
    fn failures_count_as_calls_but_not_latency() {
        let s = Shared::new(Arc::new(SystemClock));
        s.record("jev", true, 100);
        s.record("jev", false, 5000);
        s.record("jev", true, 300);
        let m = s.snapshot(|api| api == "jev");
        assert_eq!(
            m,
            vec![ApiMetrics {
                api: "jev".into(),
                calls: 3,
                ok: 2,
                p50_ms: Some(100),
                p95_ms: Some(300),
                breaker_open: true,
            }]
        );
    }
}
