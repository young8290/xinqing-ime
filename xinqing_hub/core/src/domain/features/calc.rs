//! 特征计算（FR-STA-02）。纯函数，便于与 Python 参考实现对拍。
//!
//! 依赖 FR-SEN-08（`tsf` 来源）的特征在未启用时为 `None`，不得用 0 代替。

use serde::Serialize;
use xqp::{CandOp, CompOp, KeyKind, KeySrc};

use super::baseline::{Baseline, Bucket};
use super::typo::TypoDetector;
use super::window::{WinEvent, WindowBuf};
use crate::infra::templates::AppCat;

pub const GAP_MS: u64 = 2_000;
pub const COMP_PAUSE_MS: u64 = 3_000;

/// 一个窗口的全部数值特征（写入 `window_features.features_json`，只含数值和 null）。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct WindowFeatures {
    pub n_keys: u32,
    pub active_ms: u64,
    pub kpm: Option<f64>,
    pub iki_med: Option<f64>,
    pub iki_iqr: Option<f64>,
    pub dwell_med: Option<f64>,
    pub bs_rate: f64,
    pub bs_burst_max: u32,
    pub typo_cnt: u32,
    pub pause_cnt: u32,
    /// 0 / 1（保持纯数值）。
    pub abandon: u8,
    pub delete_committed: Option<u32>,
    pub page_flips: u32,
    pub cand_pos_mean: Option<f64>,
    pub session_min: f64,
    pub hour: u32,
    /// 窗口结束时的本地分钟数（0–1439），R5 判断 23:30 用。
    pub minute_of_day: u32,
    pub kpm_z: Option<f64>,
    pub iki_med_z: Option<f64>,
    pub iki_iqr_z: Option<f64>,
    pub bs_rate_z: Option<f64>,
    pub dwell_med_z: Option<f64>,
}

/// 计算特征所需的上下文（不来自窗口本身的部分）。
#[derive(Debug, Clone, Copy)]
pub struct SessionCtx {
    /// 当前连续输入时长（分钟）。
    pub session_min: f64,
    /// 窗口结束时的本地时间。
    pub hour: u32,
    pub minute: u32,
    pub app_cat: AppCat,
}

/// 线性插值分位数（与 numpy 默认 `linear` 方法一致）。
pub fn quantile(sorted: &[f64], q: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let pos = q * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    Some(sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64))
}

struct KeyDown {
    ts: u64,
    kind: KeyKind,
    vk: Option<u8>,
    del_committed: Option<bool>,
}

pub fn compute(win: &WindowBuf, base: &Baseline, ctx: &SessionCtx) -> WindowFeatures {
    // 同一窗口若有 tsf 来源（FR-SEN-08，看得到全部按键），按键统计只用 tsf，避免与 core 重复计数。
    let want_src = win.key_src();
    let has_tsf = want_src == KeySrc::Tsf;

    let mut keys: Vec<KeyDown> = Vec::new();
    let mut in_comp_at: Vec<bool> = Vec::new();
    let mut comp_active = false;
    let mut abandon = false;
    let mut page_flips = 0u32;
    let mut cand_pos: Vec<f64> = Vec::new();
    let mut ups: Vec<(u64, Option<u8>)> = Vec::new();

    for e in &win.events {
        match e {
            WinEvent::Key {
                ts,
                kind,
                vk,
                in_comp,
                src,
                del_committed,
            } if *src == want_src && *kind != KeyKind::Other => {
                keys.push(KeyDown {
                    ts: *ts,
                    kind: *kind,
                    vk: *vk,
                    del_committed: *del_committed,
                });
                in_comp_at.push(*in_comp || comp_active);
            }
            WinEvent::Key { .. } => {}
            WinEvent::KeyUp { ts, vk } => ups.push((*ts, *vk)),
            WinEvent::Comp { op, len, .. } => {
                comp_active = matches!(op, CompOp::Update) && len.unwrap_or(1) > 0;
                if matches!(op, CompOp::Cancel | CompOp::Clear) {
                    abandon = true;
                }
            }
            WinEvent::Cand { op, pos, .. } => match op {
                CandOp::Page => page_flips += 1,
                CandOp::Select => {
                    if let Some(p) = pos {
                        cand_pos.push(*p as f64);
                    }
                }
            },
            WinEvent::Commit { .. } => comp_active = false,
        }
    }

    let n_keys = keys.len() as u32;
    let mut ikis: Vec<f64> = Vec::new();
    let mut gap_sum = 0u64;
    let mut pause_cnt = 0u32;
    for (i, pair) in keys.windows(2).enumerate() {
        let d = pair[1].ts.saturating_sub(pair[0].ts);
        if d <= GAP_MS {
            ikis.push(d as f64);
        } else {
            gap_sum += d;
        }
        if d > COMP_PAUSE_MS && in_comp_at[i] {
            pause_cnt += 1;
        }
    }
    let span = match (keys.first(), keys.last()) {
        (Some(a), Some(b)) => b.ts.saturating_sub(a.ts),
        _ => 0,
    };
    let active_ms = span.saturating_sub(gap_sum);
    let kpm = (active_ms > 0).then(|| n_keys as f64 / active_ms as f64 * 60_000.0);

    ikis.sort_by(f64::total_cmp);
    let iki_med = quantile(&ikis, 0.5);
    let iki_iqr = match (quantile(&ikis, 0.25), quantile(&ikis, 0.75)) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };

    let mut bs = 0u32;
    let mut run = 0u32;
    let mut bs_burst_max = 0u32;
    let mut typo = TypoDetector::new();
    let mut typo_cnt = 0u32;
    let mut delete_committed = 0u32;
    for k in &keys {
        if k.kind == KeyKind::Backspace {
            bs += 1;
            run += 1;
            bs_burst_max = bs_burst_max.max(run);
        } else {
            run = 0;
        }
        if typo.on_key(k.ts, k.kind, k.vk) {
            typo_cnt += 1;
        }
        if k.del_committed == Some(true) {
            delete_committed += 1;
        }
    }
    let bs_rate = if n_keys > 0 {
        bs as f64 / n_keys as f64
    } else {
        0.0
    };

    let dwell_med = if has_tsf {
        dwell_median(&keys, &ups)
    } else {
        None
    };

    let bucket = Bucket::from_hour(ctx.hour);
    let mut f = WindowFeatures {
        n_keys,
        active_ms,
        kpm,
        iki_med,
        iki_iqr,
        dwell_med,
        bs_rate,
        bs_burst_max,
        typo_cnt,
        pause_cnt,
        abandon: abandon as u8,
        delete_committed: has_tsf.then_some(delete_committed),
        page_flips,
        cand_pos_mean: (!cand_pos.is_empty())
            .then(|| cand_pos.iter().sum::<f64>() / cand_pos.len() as f64),
        session_min: ctx.session_min,
        hour: ctx.hour,
        minute_of_day: ctx.hour * 60 + ctx.minute,
        ..Default::default()
    };
    f.kpm_z = base.z(bucket, "kpm", f.kpm);
    f.iki_med_z = base.z(bucket, "iki_med", f.iki_med);
    f.iki_iqr_z = base.z(bucket, "iki_iqr", f.iki_iqr);
    f.bs_rate_z = base.z(bucket, "bs_rate", Some(f.bs_rate));
    f.dwell_med_z = base.z(bucket, "dwell_med", f.dwell_med);
    f
}

/// 每个按下事件配对其后第一个相同 vk 的抬起事件。
fn dwell_median(keys: &[KeyDown], ups: &[(u64, Option<u8>)]) -> Option<f64> {
    let mut used = vec![false; ups.len()];
    let mut dw: Vec<f64> = Vec::new();
    for k in keys {
        let Some(vk) = k.vk else { continue };
        if let Some((i, (t, _))) = ups
            .iter()
            .enumerate()
            .find(|(i, (t, v))| !used[*i] && *v == Some(vk) && *t >= k.ts)
        {
            used[i] = true;
            dw.push((*t - k.ts) as f64);
        }
    }
    dw.sort_by(f64::total_cmp);
    quantile(&dw, 0.5)
}

/// 连续输入时长：相邻活动间隔 < 2 分钟视为连续（FR-STA-02 `session_min`）。
#[derive(Debug, Default, Clone)]
pub struct SessionTracker {
    start: Option<u64>,
    last: Option<u64>,
}

pub const SESSION_BREAK_MS: u64 = 120_000;

impl SessionTracker {
    pub fn on_activity(&mut self, ts: u64) {
        match self.last {
            Some(l) if ts.saturating_sub(l) < SESSION_BREAK_MS => {}
            _ => self.start = Some(ts),
        }
        self.last = Some(ts);
    }

    pub fn minutes(&self) -> f64 {
        match (self.start, self.last) {
            (Some(s), Some(l)) => (l - s) as f64 / 60_000.0,
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::templates::BaselineDefault;

    fn base() -> Baseline {
        let d: BaselineDefault = toml::from_str(
            r#"
version = 0
[day.kpm]
med = 220.0
mad = 55.0
[day.iki_med]
med = 190.0
mad = 45.0
[day.iki_iqr]
med = 160.0
mad = 50.0
[day.bs_rate]
med = 0.08
mad = 0.03
[day.dwell_med]
med = 95.0
mad = 18.0
[night.kpm]
med = 200.0
mad = 55.0
"#,
        )
        .unwrap();
        Baseline::from_defaults(&d)
    }

    fn ctx() -> SessionCtx {
        SessionCtx {
            session_min: 3.0,
            hour: 14,
            minute: 5,
            app_cat: AppCat::Chat,
        }
    }

    fn key(ts: u64, kind: KeyKind, vk: Option<u8>) -> WinEvent {
        WinEvent::Key {
            ts,
            kind,
            vk,
            in_comp: false,
            src: KeySrc::Core,
            del_committed: None,
        }
    }

    #[test]
    fn hand_computed_window() {
        // 6 个按键：S(0) BS(200) D(400) F(600) G(900) H(1200)
        let win = WindowBuf {
            events: vec![
                key(0, KeyKind::Letter, Some(b'S')),
                key(200, KeyKind::Backspace, Some(8)),
                key(400, KeyKind::Letter, Some(b'D')),
                key(600, KeyKind::Letter, Some(b'F')),
                key(900, KeyKind::Letter, Some(b'G')),
                key(1200, KeyKind::Letter, Some(b'H')),
                WinEvent::Cand {
                    ts: 1250,
                    op: CandOp::Page,
                    pos: None,
                },
                WinEvent::Cand {
                    ts: 1300,
                    op: CandOp::Select,
                    pos: Some(2),
                },
            ],
        };
        let f = compute(&win, &base(), &ctx());
        assert_eq!(f.n_keys, 6);
        assert_eq!(f.active_ms, 1200);
        assert!((f.kpm.unwrap() - 300.0).abs() < 1e-9);
        // 间隔 200,200,200,300,300 → 中位数 200，IQR = 300 − 200
        assert_eq!(f.iki_med, Some(200.0));
        assert_eq!(f.iki_iqr, Some(100.0));
        assert!((f.bs_rate - 1.0 / 6.0).abs() < 1e-9);
        assert_eq!(f.bs_burst_max, 1);
        assert_eq!(f.typo_cnt, 1, "S → 退格 → D 是 R1");
        assert_eq!(f.page_flips, 1);
        assert_eq!(f.cand_pos_mean, Some(2.0));
        assert_eq!(f.dwell_med, None, "未启用 FR-SEN-08 时为 null");
        assert_eq!(f.delete_committed, None);
        let z = (300.0 - 220.0) / (1.4826 * 55.0);
        assert!((f.kpm_z.unwrap() - z).abs() < 1e-9);
    }

    #[test]
    fn long_gap_excluded_from_active_and_comp_pause_counted() {
        let win = WindowBuf {
            events: vec![
                key(0, KeyKind::Letter, Some(b'N')),
                WinEvent::Comp {
                    ts: 1,
                    op: CompOp::Update,
                    len: Some(1),
                },
                key(100, KeyKind::Letter, Some(b'I')),
                key(3_600, KeyKind::Letter, Some(b'H')),
                WinEvent::Comp {
                    ts: 3_700,
                    op: CompOp::Cancel,
                    len: None,
                },
                key(3_800, KeyKind::Letter, Some(b'A')),
                key(4_000, KeyKind::Letter, Some(b'O')),
            ],
        };
        let f = compute(&win, &base(), &ctx());
        assert_eq!(f.active_ms, 4_000 - 3_500);
        assert_eq!(f.pause_cnt, 1);
        assert_eq!(f.abandon, 1);
    }

    #[test]
    fn tsf_source_enables_dwell_and_delete_committed() {
        let k = |ts, vk, del| WinEvent::Key {
            ts,
            kind: if vk == 8 {
                KeyKind::Backspace
            } else {
                KeyKind::Letter
            },
            vk: Some(vk),
            in_comp: false,
            src: KeySrc::Tsf,
            del_committed: Some(del),
        };
        let up = |ts, vk| WinEvent::KeyUp { ts, vk: Some(vk) };
        let win = WindowBuf {
            events: vec![
                k(0, b'A', false),
                up(80, b'A'),
                k(200, 8, true),
                up(300, 8),
                k(400, 8, true),
                up(520, 8),
            ],
        };
        let f = compute(&win, &base(), &ctx());
        assert_eq!(f.dwell_med, Some(100.0));
        assert_eq!(f.delete_committed, Some(2));
        assert_eq!(f.bs_burst_max, 2);
    }

    #[test]
    fn session_tracker_breaks_after_two_minutes() {
        let mut s = SessionTracker::default();
        s.on_activity(0);
        s.on_activity(60_000);
        s.on_activity(170_000);
        assert!((s.minutes() - 170_000.0 / 60_000.0).abs() < 1e-9);
        s.on_activity(300_000);
        assert_eq!(s.minutes(), 0.0);
    }

    #[test]
    fn serialized_features_are_numeric_only() {
        let f = WindowFeatures::default();
        let v = serde_json::to_value(&f).unwrap();
        for (_, x) in v.as_object().unwrap() {
            assert!(x.is_number() || x.is_null());
        }
    }
}
