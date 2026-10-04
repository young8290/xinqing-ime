//! 融合与滞回（FR-STA-06）、降级运行（FR-STA-08）、反馈校准（FR-STA-06 第 8 条）。
//!
//! 两处对规格的解释（写入 docs/adr/0008）：
//! 1. 回到 `fluent` 只走第 7 条（连续 3 个 fluent 窗口且 p ≥ 0.6），第 4、5 条只用于切到非 fluent 状态，
//!    否则 p ≥ 0.9 的单个 fluent 窗口会让状态来回抖动；
//! 2. 降级（只有本地规则）时沿用同样的滞回：同一规则状态连续 2 个窗口才切换，连续 3 个无提示窗口回到 fluent。

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use xqp::MoodState;

use super::rules::{Hint, Hints};

/// Jev 对一个窗口的判断（FR-STA-05 输出）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JevVerdict {
    pub probs: HashMap<MoodState, f64>,
    pub choice: MoodState,
    pub confidence: f64,
    /// 0–4
    pub valence: f64,
    pub need_comfort: f64,
}

impl JevVerdict {
    pub fn p(&self, s: MoodState) -> f64 {
        self.probs.get(&s).copied().unwrap_or(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Jev,
    Rule,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FusionOut {
    pub shown: MoodState,
    pub changed: bool,
    pub conflict: bool,
    pub source: Source,
    /// 本窗口的候选状态（Jev choice 或规则映射）。
    pub cand: MoodState,
}

pub const P_CONFLICT: f64 = 0.80;
pub const P_IMMEDIATE: f64 = 0.90;
pub const P_CONSECUTIVE: f64 = 0.70;
pub const P_FLUENT: f64 = 0.60;
pub const FLUENT_STREAK: u8 = 3;
pub const BUMP: f64 = 0.05;
pub const BUMP_CAP: f64 = 0.95;
const FEEDBACK_WINDOW_MS: i64 = 30 * 60_000;
const BUMP_DAYS_MS: i64 = 7 * 24 * 3_600_000;

fn hint_supports(s: MoodState, h: &Hints) -> bool {
    match s {
        MoodState::Hesitant => h.has(Hint::HesitationHint),
        MoodState::Agitated => h.has(Hint::AgitationHint),
        MoodState::Tired => h.has(Hint::FatigueHint) || h.has(Hint::LateNight),
        MoodState::Low => h.has(Hint::LowHint),
        MoodState::Fluent | MoodState::Unknown => true,
    }
}

/// FR-STA-08：只用本地规则时的候选状态。
fn rule_candidate(h: &Hints) -> MoodState {
    if h.has(Hint::HesitationHint) {
        MoodState::Hesitant
    } else if h.has(Hint::FatigueHint) || h.has(Hint::LateNight) {
        MoodState::Tired
    } else {
        MoodState::Fluent
    }
}

#[derive(Debug, Clone)]
pub struct Fusion {
    shown: MoodState,
    last_cand: Option<MoodState>,
    fluent_streak: u8,
    /// 每个状态的阈值上调量及到期时间（Unix 毫秒）。
    bumps: HashMap<MoodState, (f64, i64)>,
    unfit: HashMap<MoodState, VecDeque<i64>>,
}

impl Default for Fusion {
    fn default() -> Self {
        Self::new()
    }
}

impl Fusion {
    pub fn new() -> Self {
        Self {
            shown: MoodState::Fluent,
            last_cand: None,
            fluent_streak: 0,
            bumps: HashMap::new(),
            unfit: HashMap::new(),
        }
    }

    pub fn shown(&self) -> MoodState {
        self.shown
    }

    /// 某状态当前的个人阈值上调量（FR-STA-06 第 8 条），已过期为 0。
    pub fn bump(&self, s: MoodState, now_ms: i64) -> f64 {
        match self.bumps.get(&s) {
            Some((b, until)) if *until > now_ms => *b,
            _ => 0.0,
        }
    }

    /// 处理一个窗口。`now_ms` 用于判断个人阈值是否仍有效。
    pub fn on_window(
        &mut self,
        hints: &Hints,
        verdict: Option<&JevVerdict>,
        now_ms: i64,
    ) -> FusionOut {
        let Some(v) = verdict else {
            return self.rule_only(hints);
        };
        let cand = v.choice;
        let p = v.p(cand);
        let before = self.shown;
        let mut conflict = false;

        if cand == MoodState::Fluent {
            if p >= P_FLUENT {
                self.fluent_streak = self.fluent_streak.saturating_add(1);
            } else {
                self.fluent_streak = 0;
            }
            if self.fluent_streak >= FLUENT_STREAK {
                self.shown = MoodState::Fluent;
            }
        } else {
            self.fluent_streak = 0;
            let b = self.bump(cand, now_ms);
            if !hint_supports(cand, hints) && p < P_CONFLICT {
                conflict = true;
            } else {
                // 第 4 条：p ≥ 0.90 立即切换；第 5 条：p ≥ 0.70 且上一窗口候选相同
                let immediate = p >= (P_IMMEDIATE + b).min(BUMP_CAP);
                let consecutive =
                    p >= (P_CONSECUTIVE + b).min(BUMP_CAP) && self.last_cand == Some(cand);
                if immediate || consecutive {
                    self.shown = cand;
                }
            }
        }
        self.last_cand = Some(cand);
        FusionOut {
            shown: self.shown,
            changed: self.shown != before,
            conflict,
            source: Source::Jev,
            cand,
        }
    }

    fn rule_only(&mut self, hints: &Hints) -> FusionOut {
        let cand = rule_candidate(hints);
        let before = self.shown;
        if cand == MoodState::Fluent {
            self.fluent_streak = self.fluent_streak.saturating_add(1);
            if self.fluent_streak >= FLUENT_STREAK {
                self.shown = MoodState::Fluent;
            }
        } else {
            self.fluent_streak = 0;
            if self.last_cand == Some(cand) {
                self.shown = cand;
            }
        }
        self.last_cand = Some(cand);
        FusionOut {
            shown: self.shown,
            changed: self.shown != before,
            conflict: false,
            source: Source::Rule,
            cand,
        }
    }

    /// 用户点“不准”（FR-STA-07）：30 分钟内对同一状态 3 次 → 该状态阈值 +0.05，持续 7 天。
    pub fn record_unfit(&mut self, s: MoodState, now_ms: i64) {
        let q = self.unfit.entry(s).or_default();
        q.push_back(now_ms);
        while q.front().is_some_and(|t| now_ms - t > FEEDBACK_WINDOW_MS) {
            q.pop_front();
        }
        if q.len() >= 3 {
            q.clear();
            let cur = self.bump(s, now_ms);
            let next = (cur + BUMP).min(BUMP_CAP - P_IMMEDIATE);
            self.bumps.insert(s, (next, now_ms + BUMP_DAYS_MS));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(choice: MoodState, p: f64) -> JevVerdict {
        JevVerdict {
            probs: HashMap::from([(choice, p)]),
            choice,
            confidence: p,
            valence: 2.0,
            need_comfort: 0.3,
        }
    }

    fn h(list: &[Hint]) -> Hints {
        Hints(list.to_vec())
    }

    #[test]
    fn immediate_switch_at_090() {
        let mut f = Fusion::new();
        let o = f.on_window(
            &h(&[Hint::HesitationHint]),
            Some(&v(MoodState::Hesitant, 0.92)),
            0,
        );
        assert!(o.changed);
        assert_eq!(o.shown, MoodState::Hesitant);
    }

    #[test]
    fn needs_two_windows_at_070() {
        let mut f = Fusion::new();
        let hint = h(&[Hint::HesitationHint]);
        assert!(
            !f.on_window(&hint, Some(&v(MoodState::Hesitant, 0.75)), 0)
                .changed
        );
        assert!(
            f.on_window(&hint, Some(&v(MoodState::Hesitant, 0.75)), 0)
                .changed
        );
    }

    #[test]
    fn conflict_without_supporting_hint() {
        let mut f = Fusion::new();
        let o = f.on_window(&h(&[]), Some(&v(MoodState::Agitated, 0.79)), 0);
        assert!(o.conflict);
        assert_eq!(o.shown, MoodState::Fluent);
        // p ≥ 0.80 不算冲突，但仍要 ≥ 0.90 才立即切换
        let o = f.on_window(&h(&[]), Some(&v(MoodState::Low, 0.85)), 0);
        assert!(!o.conflict);
        assert!(!o.changed);
        let o = f.on_window(&h(&[]), Some(&v(MoodState::Low, 0.85)), 0);
        assert!(o.changed, "连续两个 ≥ 0.70 的 low 窗口应切换");
    }

    #[test]
    fn back_to_fluent_after_three_fluent_windows() {
        let mut f = Fusion::new();
        f.on_window(
            &h(&[Hint::HesitationHint]),
            Some(&v(MoodState::Hesitant, 0.95)),
            0,
        );
        for i in 0..2 {
            let o = f.on_window(&h(&[]), Some(&v(MoodState::Fluent, 0.95)), 0);
            assert_eq!(
                o.shown,
                MoodState::Hesitant,
                "第 {} 个 fluent 窗口不应切回",
                i + 1
            );
        }
        assert_eq!(
            f.on_window(&h(&[]), Some(&v(MoodState::Fluent, 0.7)), 0)
                .shown,
            MoodState::Fluent
        );
    }

    #[test]
    fn rule_only_mode() {
        let mut f = Fusion::new();
        let r2 = h(&[Hint::HesitationHint]);
        let o = f.on_window(&r2, None, 0);
        assert_eq!(o.source, Source::Rule);
        assert!(!o.changed);
        assert_eq!(f.on_window(&r2, None, 0).shown, MoodState::Hesitant);
        // R3 / R6 在降级时不产生状态（状态集合缩减为 fluent / hesitant / tired）
        let mut f = Fusion::new();
        let r3 = h(&[Hint::AgitationHint, Hint::LowHint]);
        f.on_window(&r3, None, 0);
        assert_eq!(f.on_window(&r3, None, 0).shown, MoodState::Fluent);
    }

    #[test]
    fn three_unfit_in_30_min_raise_threshold() {
        let mut f = Fusion::new();
        for t in [0, 60_000, 120_000] {
            f.record_unfit(MoodState::Hesitant, t);
        }
        let hint = h(&[Hint::HesitationHint]);
        // 阈值变为 0.95，0.92 不再立即切换
        let o = f.on_window(&hint, Some(&v(MoodState::Hesitant, 0.92)), 200_000);
        assert!(!o.changed);
        // 7 天后恢复
        let mut g = f.clone();
        let later = 200_000 + BUMP_DAYS_MS;
        assert!(
            g.on_window(&hint, Some(&v(MoodState::Hesitant, 0.92)), later)
                .changed
        );
    }
}
