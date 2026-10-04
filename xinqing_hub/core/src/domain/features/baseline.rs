//! 个人基线（FR-STA-03）：按白天 / 夜间分桶的中位数与 MAD，只存统计值。

use std::collections::HashMap;

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
