//! 个人基线（FR-STA-03）：按白天 / 夜间分桶的中位数与 MAD，只存统计值。

use std::collections::HashMap;

use chrono::{DateTime, Local, TimeZone};

use super::calc::WindowFeatures;
use crate::infra::templates::{BaselineDefault, MedMad};

/// 参与 z 分数计算的基础特征（FR-STA-02 `*_z`）。
pub const BASE_FEATURES: [&str; 5] = ["kpm", "iki_med", "iki_iqr", "bs_rate", "dwell_med"];
/// 有效窗口数低于该值时使用人群默认值（冷启动）。
pub const COLD_START_WINDOWS: u32 = 200;
pub const Z_CLIP: f64 = 5.0;
const MAD_TO_SD: f64 = 1.4826;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bucket {
    Day,
    Night,
}

impl Bucket {
    /// 白天 06:00–22:00，夜间 22:00–06:00。
    pub fn from_hour(hour: u32) -> Self {
        if (6..22).contains(&hour) {
            Bucket::Day
        } else {
            Bucket::Night
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Bucket::Day => "day",
            Bucket::Night => "night",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Baseline {
    personal: HashMap<(Bucket, &'static str), MedMad>,
    defaults: HashMap<(Bucket, &'static str), MedMad>,
    /// 已积累的有效窗口数。
    pub windows: u32,
}

impl Baseline {
    pub fn from_defaults(d: &BaselineDefault) -> Self {
        let mut defaults = HashMap::new();
        for f in BASE_FEATURES {
            if let Some(v) = d.day.get(f) {
                defaults.insert((Bucket::Day, f), *v);
            }
            if let Some(v) = d.night.get(f) {
                defaults.insert((Bucket::Night, f), *v);
            }
        }
        Self {
            personal: HashMap::new(),
            defaults,
            windows: 0,
        }
    }

    /// 回放用的固定基线（FR-DMO-01 `--baseline`）：把文件里的值当作个人基线，并越过冷启动，
    /// 结果只取决于文件，不读写真实基线。`xq-replay` 与 Hub 的 `XQ_SIM_BASELINE` 共用（ADR 0020）。
    pub fn fixed(d: &BaselineDefault) -> Self {
        let mut b = Self::from_defaults(d);
        for (bucket, map) in [(Bucket::Day, &d.day), (Bucket::Night, &d.night)] {
            for (f, v) in map {
                b.set_personal(bucket, f, *v);
            }
        }
        b.windows = u32::MAX / 2;
        b
    }

    /// 换上一次重算的结果：个人统计值整体替换，有效窗口数取重算时的样本数（FR-STA-03 第 4 条）。
    pub fn apply(&mut self, stats: &BaselineStats) {
        self.personal.clear();
        for r in &stats.rows {
            self.set_personal(r.bucket, r.feature, r.value);
        }
        self.windows = stats.windows;
    }

    /// 写入个人统计值（来自 `baseline` 表或每天 04:00 的重算结果）。
    pub fn set_personal(&mut self, bucket: Bucket, feature: &str, v: MedMad) {
        if let Some(f) = BASE_FEATURES.iter().find(|f| **f == feature) {
            self.personal.insert((bucket, f), v);
        }
    }

    pub fn is_cold(&self) -> bool {
        self.windows < COLD_START_WINDOWS
    }

    /// 冷启动进度（0–100），设置页“正在熟悉你的打字习惯（已完成 x%）”。
    pub fn progress_pct(&self) -> u8 {
        ((self.windows.min(COLD_START_WINDOWS) * 100) / COLD_START_WINDOWS) as u8
    }

    /// 当前生效的统计值：冷启动用人群默认值，否则用个人值（没有个人值时退回默认值）。
    /// 返回 (生效值, 人群默认值)。
    fn used(&self, bucket: Bucket, feature: &str) -> Option<(&MedMad, &MedMad)> {
        let key = (bucket, *BASE_FEATURES.iter().find(|f| **f == feature)?);
        let default = self.defaults.get(&key)?;
        let used = if self.is_cold() {
            default
        } else {
            self.personal.get(&key).unwrap_or(default)
        };
        Some((used, default))
    }

    /// 当前生效的中位数（FR-STA-09 说明文案中 `{p}` 的分母）。
    pub fn med(&self, bucket: Bucket, feature: &str) -> Option<f64> {
        self.used(bucket, feature).map(|(u, _)| u.med)
    }

    /// `z = (x − med) / max(1.4826 × mad, ε)`，ε = 人群默认标准差的 10%，截断到 [−5, 5]。
    pub fn z(&self, bucket: Bucket, feature: &str, x: Option<f64>) -> Option<f64> {
        let x = x?;
        let (used, default) = self.used(bucket, feature)?;
        let eps = MAD_TO_SD * default.mad * 0.1;
        let denom = (MAD_TO_SD * used.mad).max(eps);
        if denom <= 0.0 {
            return None;
        }
        Some(((x - used.med) / denom).clamp(-Z_CLIP, Z_CLIP))
    }
}

/// 一个时段桶里一个特征的个人统计值（`baseline` 表的一行，09 D-09）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BaselineRow {
    pub bucket: Bucket,
    pub feature: &'static str,
    pub value: MedMad,
    /// 样本数
    pub n: u32,
}

/// 一次基线重算的结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BaselineStats {
    pub rows: Vec<BaselineRow>,
    /// 参与重算的有效窗口数（冷启动判断用）。
    pub windows: u32,
}

/// 某时段桶里一个特征至少要有这么多个样本，才用个人值，否则这一格仍用人群默认值。
/// 例如很少在夜里打字时，夜间桶几个窗口算出的中位数不可靠。
pub const MIN_BUCKET_SAMPLES: u32 = 30;

/// 由最近 7 天的窗口特征重算个人基线（FR-STA-03 第 1、4 条）。窗口按自身的 `hour` 分桶；
/// 值为 `null` 的特征（如未启用 FR-SEN-08 时的 `dwell_med`）不计入样本。
pub fn compute_stats<'a>(windows: impl IntoIterator<Item = &'a WindowFeatures>) -> BaselineStats {
    let mut samples: HashMap<(Bucket, &'static str), Vec<f64>> = HashMap::new();
    let mut count = 0u32;
    for w in windows {
        count = count.saturating_add(1);
        let bucket = Bucket::from_hour(w.hour);
        for f in BASE_FEATURES {
            let v = match f {
                "kpm" => w.kpm,
                "iki_med" => w.iki_med,
                "iki_iqr" => w.iki_iqr,
                "bs_rate" => Some(w.bs_rate),
                "dwell_med" => w.dwell_med,
                _ => None,
            };
            if let Some(v) = v.filter(|v| v.is_finite()) {
                samples.entry((bucket, f)).or_default().push(v);
            }
        }
    }
    let mut rows: Vec<BaselineRow> = samples
        .into_iter()
        .filter_map(|((bucket, feature), v)| {
            let n = u32::try_from(v.len()).unwrap_or(u32::MAX);
            if n < MIN_BUCKET_SAMPLES {
                return None;
            }
            Some(BaselineRow {
                bucket,
                feature,
                value: med_mad(&v)?,
                n,
            })
        })
        .collect();
    rows.sort_by_key(|r| (r.bucket.as_str(), r.feature));
    BaselineStats {
        rows,
        windows: count,
    }
}

/// 每天的重算时刻 04:00（FR-STA-03 第 4 条）。
pub const RECOMPUTE_HOUR: u32 = 4;

/// `now` 之后（不含）的下一个 04:00。
pub fn next_recompute_after(now: DateTime<Local>) -> DateTime<Local> {
    let at = |d: chrono::NaiveDate| {
        d.and_hms_opt(RECOMPUTE_HOUR, 0, 0)
            .and_then(|t| Local.from_local_datetime(&t).earliest())
    };
    let today = now.date_naive();
    match at(today) {
        Some(t) if t > now => t,
        _ => today
            .succ_opt()
            .and_then(at)
            .unwrap_or(now + chrono::Duration::days(1)),
    }
}

/// 由样本计算中位数与 MAD（每日重算用；与 Python `statistics.median` 一致）。
pub fn med_mad(values: &[f64]) -> Option<MedMad> {
    let med = median(values)?;
    let dev: Vec<f64> = values.iter().map(|v| (v - med).abs()).collect();
    Some(MedMad {
        med,
        mad: median(&dev)?,
    })
}

pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> BaselineDefault {
        toml::from_str(
            r#"
version = 0
[day.kpm]
med = 220.0
mad = 55.0
[night.kpm]
med = 200.0
mad = 55.0
"#,
        )
        .unwrap()
    }

    #[test]
    fn cold_start_uses_population_default() {
        let b = Baseline::from_defaults(&defaults());
        let z = b
            .z(Bucket::Day, "kpm", Some(220.0 + 1.4826 * 55.0))
            .unwrap();
        assert!((z - 1.0).abs() < 1e-9);
        assert_eq!(b.progress_pct(), 0);
    }

    #[test]
    fn personal_baseline_after_200_windows_and_clip() {
        let mut b = Baseline::from_defaults(&defaults());
        b.set_personal(
            Bucket::Day,
            "kpm",
            MedMad {
                med: 100.0,
                mad: 0.0,
            },
        );
        b.windows = 200;
        // mad = 0 时使用 ε = 1.4826 × 55 × 0.1，结果被截断到 5
        assert_eq!(b.z(Bucket::Day, "kpm", Some(200.0)), Some(5.0));
        assert_eq!(b.med(Bucket::Day, "kpm"), Some(100.0));
        // 个人值只在白天桶，夜间退回人群默认值
        assert_eq!(b.med(Bucket::Night, "kpm"), Some(200.0));
        assert_eq!(b.z(Bucket::Day, "kpm", None), None);
    }

    fn win(hour: u32, kpm: f64) -> WindowFeatures {
        WindowFeatures {
            hour,
            kpm: Some(kpm),
            bs_rate: 0.1,
            ..Default::default()
        }
    }

    #[test]
    fn recompute_buckets_and_minimum_samples() {
        // 白天 40 个窗口 kpm = 100..139，夜间只有 5 个
        let mut ws: Vec<WindowFeatures> = (0..40).map(|i| win(14, 100.0 + i as f64)).collect();
        ws.extend((0..5).map(|_| win(23, 50.0)));
        let stats = compute_stats(&ws);
        assert_eq!(stats.windows, 45);
        let day_kpm = stats
            .rows
            .iter()
            .find(|r| r.bucket == Bucket::Day && r.feature == "kpm")
            .unwrap();
        assert_eq!(day_kpm.n, 40);
        assert_eq!(day_kpm.value.med, 119.5);
        assert_eq!(day_kpm.value.mad, 10.0);
        // 夜间样本不足，iki_med 全是 null：都不出现
        assert!(stats.rows.iter().all(|r| r.bucket == Bucket::Day));
        assert!(
            stats
                .rows
                .iter()
                .all(|r| r.feature == "kpm" || r.feature == "bs_rate")
        );

        let mut b = Baseline::from_defaults(&defaults());
        b.apply(&stats);
        assert!(b.is_cold(), "45 个窗口仍是冷启动");
        assert_eq!(b.med(Bucket::Day, "kpm"), Some(220.0));
        b.windows = 200;
        assert_eq!(b.med(Bucket::Day, "kpm"), Some(119.5));
        assert_eq!(
            b.med(Bucket::Night, "kpm"),
            Some(200.0),
            "夜间退回人群默认值"
        );
        // 再换一次结果会整体替换
        b.apply(&BaselineStats::default());
        assert_eq!((b.windows, b.med(Bucket::Day, "kpm")), (0, Some(220.0)));
    }

    #[test]
    fn next_recompute_is_the_coming_four_am() {
        let t = |d: u32, h: u32, m: u32| Local.with_ymd_and_hms(2026, 10, d, h, m, 0).unwrap();
        assert_eq!(next_recompute_after(t(5, 3, 59)), t(5, 4, 0));
        assert_eq!(next_recompute_after(t(5, 4, 0)), t(6, 4, 0));
        assert_eq!(next_recompute_after(t(5, 23, 0)), t(6, 4, 0));
    }

    #[test]
    fn buckets_and_med_mad() {
        assert_eq!(Bucket::from_hour(6), Bucket::Day);
        assert_eq!(Bucket::from_hour(22), Bucket::Night);
        assert_eq!(Bucket::from_hour(3), Bucket::Night);
        let m = med_mad(&[1.0, 2.0, 3.0, 4.0, 100.0]).unwrap();
        assert_eq!(m.med, 3.0);
        assert_eq!(m.mad, 1.0);
    }
}
