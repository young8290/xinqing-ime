//! 每日预算（FR-AIG-07），按自然日 00:00 重置。

use std::collections::HashMap;

use chrono::NaiveDate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BudgetKind {
    Jev,
    /// 大模型调用（不含对话）
    Llm,
    ChatTurn,
    SchedulePrefilter,
    Rewrite,
}

impl BudgetKind {
    pub fn default_cap(self) -> u32 {
        match self {
            BudgetKind::Jev => 3000,
            BudgetKind::Llm => 200,
            BudgetKind::ChatTurn => 100,
            BudgetKind::SchedulePrefilter => 50,
            BudgetKind::Rewrite => 100,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Budget {
    day: Option<NaiveDate>,
    used: HashMap<BudgetKind, u32>,
    caps: HashMap<BudgetKind, u32>,
}

impl Default for Budget {
    fn default() -> Self {
        Self::new()
    }
}

impl Budget {
    pub fn new() -> Self {
        Self {
            day: None,
            used: HashMap::new(),
            caps: HashMap::new(),
        }
    }

    pub fn set_cap(&mut self, kind: BudgetKind, cap: u32) {
        self.caps.insert(kind, cap);
    }

    fn roll(&mut self, today: NaiveDate) {
        if self.day != Some(today) {
            self.day = Some(today);
            self.used.clear();
        }
    }

    /// 占用 1 次额度；超限返回 `false`。
    pub fn try_take(&mut self, kind: BudgetKind, today: NaiveDate) -> bool {
        self.roll(today);
        let cap = self.caps.get(&kind).copied().unwrap_or(kind.default_cap());
        let used = self.used.entry(kind).or_insert(0);
        if *used >= cap {
            return false;
        }
        *used += 1;
        true
    }

    pub fn used(&mut self, kind: BudgetKind, today: NaiveDate) -> u32 {
        self.roll(today);
        self.used.get(&kind).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_and_daily_reset() {
        let d1 = NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
        let d2 = d1.succ_opt().unwrap();
        let mut b = Budget::new();
        b.set_cap(BudgetKind::Rewrite, 2);
        assert!(b.try_take(BudgetKind::Rewrite, d1));
        assert!(b.try_take(BudgetKind::Rewrite, d1));
        assert!(!b.try_take(BudgetKind::Rewrite, d1));
        assert!(b.try_take(BudgetKind::Rewrite, d2));
        assert_eq!(b.used(BudgetKind::Rewrite, d2), 1);
    }
}
