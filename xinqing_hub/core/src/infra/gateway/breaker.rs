//! 熔断器（FR-AIG-02 / 03）：连续失败达到上限 → 打开一段时间 → 半开时放 1 个请求试探。
//! 时间由调用方传入（Unix 毫秒），便于测试和演示时钟。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    Closed { fails: u8 },
    Open { until_ms: i64 },
    HalfOpen { probing: bool },
}

#[derive(Debug, Clone)]
pub struct Breaker {
    state: BreakerState,
    fail_limit: u8,
    open_ms: i64,
}

impl Breaker {
    pub fn new(fail_limit: u8, open_ms: i64) -> Self {
        Self {
            state: BreakerState::Closed { fails: 0 },
            fail_limit,
            open_ms,
        }
    }

    /// Jev：连续 5 次失败 → 打开 60 秒。
    pub fn jev() -> Self {
        Self::new(5, 60_000)
    }

    /// 每个大模型：连续 3 次失败 → 打开 5 分钟。
    pub fn llm() -> Self {
        Self::new(3, 5 * 60_000)
    }

    pub fn state(&self) -> BreakerState {
        self.state
    }

    /// 是否允许发出请求；半开状态只放行 1 个试探请求。
    pub fn try_acquire(&mut self, now_ms: i64) -> bool {
        match self.state {
            BreakerState::Closed { .. } => true,
            BreakerState::Open { until_ms } if now_ms >= until_ms => {
                self.state = BreakerState::HalfOpen { probing: true };
                true
            }
            BreakerState::Open { .. } => false,
            BreakerState::HalfOpen { probing: true } => false,
            BreakerState::HalfOpen { probing: false } => {
                self.state = BreakerState::HalfOpen { probing: true };
                true
            }
        }
    }

    pub fn on_success(&mut self) {
        self.state = BreakerState::Closed { fails: 0 };
    }

    pub fn on_failure(&mut self, now_ms: i64) {
        self.state = match self.state {
            BreakerState::Closed { fails } if fails + 1 < self.fail_limit => {
                BreakerState::Closed { fails: fails + 1 }
            }
            _ => BreakerState::Open {
                until_ms: now_ms + self.open_ms,
            },
        };
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state, BreakerState::Open { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_after_limit_and_half_opens() {
        let mut b = Breaker::jev();
        for _ in 0..4 {
            assert!(b.try_acquire(0));
            b.on_failure(0);
        }
        assert!(!b.is_open());
        b.on_failure(0);
        assert!(b.is_open());
        assert!(!b.try_acquire(59_999));
        assert!(b.try_acquire(60_000), "半开放行一个试探");
        assert!(!b.try_acquire(60_001), "试探期间不再放行");
        b.on_failure(60_002);
        assert!(b.is_open(), "试探失败重新打开");
        assert!(b.try_acquire(120_002));
        b.on_success();
        assert_eq!(b.state(), BreakerState::Closed { fails: 0 });
    }
}
