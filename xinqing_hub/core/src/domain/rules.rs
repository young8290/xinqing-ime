//! 本地规则 R2–R6（FR-STA-04，窗口级）。R1 是实时的，见 `features::typo`。

use serde::Serialize;

use super::features::WindowFeatures;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Hint {
    /// R2
    HesitationHint,
    /// R3
    AgitationHint,
    /// R4
    FatigueHint,
    /// R5
    LateNight,
    /// R6
    LowHint,
}

impl Hint {
    pub const ALL: [Hint; 5] = [
        Hint::HesitationHint,
        Hint::AgitationHint,
        Hint::FatigueHint,
        Hint::LateNight,
        Hint::LowHint,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Hint::HesitationHint => "hesitation_hint",
            Hint::AgitationHint => "agitation_hint",
            Hint::FatigueHint => "fatigue_hint",
            Hint::LateNight => "late_night",
            Hint::LowHint => "low_hint",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Hints(pub Vec<Hint>);

impl Hints {
    pub fn has(&self, h: Hint) -> bool {
        self.0.contains(&h)
    }

    /// 读回 `window_features.hints`；不认识的名字跳过。
    pub fn parse(joined: &str) -> Self {
        Hints(
            joined
                .split(',')
                .filter_map(|s| Hint::ALL.into_iter().find(|h| h.as_str() == s.trim()))
                .collect(),
        )
    }

    /// 写入 `window_features.hints` 的逗号分隔串。
    pub fn joined(&self) -> String {
        self.0
            .iter()
            .map(|h| h.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

const LATE_START_MIN: u32 = 23 * 60 + 30;
const LATE_END_MIN: u32 = 5 * 60;

/// `recent` 为此前的窗口（按时间顺序，最后一个是上一窗口），R4 需要最近 3 个窗口的趋势。
pub fn evaluate(f: &WindowFeatures, recent: &[WindowFeatures]) -> Hints {
    let mut out = Vec::new();

    // R2 犹豫
    if f.pause_cnt >= 2
        || f.abandon == 1
        || f.delete_committed.unwrap_or(0) >= 5
        || f.page_flips >= 3
    {
        out.push(Hint::HesitationHint);
    }

    // R3 急躁
    if matches!((f.kpm_z, f.bs_rate_z), (Some(k), Some(b)) if k > 1.5 && b > 1.5) {
        out.push(Hint::AgitationHint);
    }

    // R4 疲劳：连续 > 45 分钟，且最近 3 个窗口 kpm 单调下降、bs_rate 单调上升
    if f.session_min > 45.0 && recent.len() >= 2 {
        let w = [&recent[recent.len() - 2], &recent[recent.len() - 1], f];
        let kpm_down = w
            .windows(2)
            .all(|p| matches!((p[0].kpm, p[1].kpm), (Some(a), Some(b)) if b < a));
        let bs_up = w.windows(2).all(|p| p[1].bs_rate > p[0].bs_rate);
        if kpm_down && bs_up {
            out.push(Hint::FatigueHint);
        }
    }

    // R5 深夜：[23:30, 05:00) 且连续 > 10 分钟
    let m = f.minute_of_day;
    if !(LATE_END_MIN..LATE_START_MIN).contains(&m) && f.session_min > 10.0 {
        out.push(Hint::LateNight);
    }

    // R6 低落：明显慢、间隔长、修改不多
    if matches!((f.kpm_z, f.iki_med_z, f.bs_rate_z), (Some(k), Some(i), Some(b)) if k <= -1.0 && i >= 1.0 && b <= 0.5)
    {
        out.push(Hint::LowHint);
    }

    Hints(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f() -> WindowFeatures {
        WindowFeatures {
            n_keys: 20,
            kpm: Some(200.0),
            kpm_z: Some(0.0),
            iki_med_z: Some(0.0),
            bs_rate_z: Some(0.0),
            hour: 14,
            minute_of_day: 14 * 60,
            ..Default::default()
        }
    }

    #[test]
    fn hints_round_trip() {
        let h = Hints(vec![Hint::HesitationHint, Hint::LateNight]);
        assert_eq!(Hints::parse(&h.joined()), h);
        assert_eq!(Hints::parse(""), Hints::default());
        assert_eq!(Hints::parse("low_hint,unknown"), Hints(vec![Hint::LowHint]));
    }

    #[test]
    fn fluent_window_has_no_hints() {
        assert!(evaluate(&f(), &[]).0.is_empty());
    }

    #[test]
    fn r2_triggers() {
        for g in [
            WindowFeatures {
                pause_cnt: 2,
                ..f()
            },
            WindowFeatures { abandon: 1, ..f() },
            WindowFeatures {
                delete_committed: Some(5),
                ..f()
            },
            WindowFeatures {
                page_flips: 3,
                ..f()
            },
        ] {
            assert!(evaluate(&g, &[]).has(Hint::HesitationHint));
        }
        let g = WindowFeatures {
            pause_cnt: 1,
            page_flips: 2,
            ..f()
        };
        assert!(!evaluate(&g, &[]).has(Hint::HesitationHint));
    }

    #[test]
    fn r3_needs_both_z() {
        let g = WindowFeatures {
            kpm_z: Some(1.6),
            bs_rate_z: Some(1.6),
            ..f()
        };
        assert!(evaluate(&g, &[]).has(Hint::AgitationHint));
        let g = WindowFeatures {
            kpm_z: Some(1.6),
            bs_rate_z: Some(1.5),
            ..f()
        };
        assert!(!evaluate(&g, &[]).has(Hint::AgitationHint));
    }

    #[test]
    fn r4_trend_over_three_windows() {
        let w = |kpm, bs| WindowFeatures {
            kpm: Some(kpm),
            bs_rate: bs,
            session_min: 50.0,
            ..f()
        };
        let recent = [w(220.0, 0.05), w(200.0, 0.07)];
        assert!(evaluate(&w(180.0, 0.09), &recent).has(Hint::FatigueHint));
        assert!(!evaluate(&w(205.0, 0.09), &recent).has(Hint::FatigueHint));
        let short = WindowFeatures {
            session_min: 40.0,
            ..w(180.0, 0.09)
        };
        assert!(!evaluate(&short, &recent).has(Hint::FatigueHint));
    }

    #[test]
    fn r5_late_night_boundaries() {
        let at = |h: u32, m: u32, s| WindowFeatures {
            hour: h,
            minute_of_day: h * 60 + m,
            session_min: s,
            ..f()
        };
        assert!(evaluate(&at(23, 30, 11.0), &[]).has(Hint::LateNight));
        assert!(evaluate(&at(4, 59, 11.0), &[]).has(Hint::LateNight));
        assert!(!evaluate(&at(5, 0, 11.0), &[]).has(Hint::LateNight));
        assert!(!evaluate(&at(23, 29, 11.0), &[]).has(Hint::LateNight));
        assert!(!evaluate(&at(1, 0, 10.0), &[]).has(Hint::LateNight));
    }

    #[test]
    fn r6_low() {
        let g = WindowFeatures {
            kpm_z: Some(-1.2),
            iki_med_z: Some(1.1),
            bs_rate_z: Some(0.2),
            ..f()
        };
        assert!(evaluate(&g, &[]).has(Hint::LowHint));
        let g = WindowFeatures {
            bs_rate_z: Some(0.8),
            ..g
        };
        assert!(!evaluate(&g, &[]).has(Hint::LowHint));
    }
}
