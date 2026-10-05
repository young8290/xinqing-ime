//! 窗口切分、特征计算与个人基线：Rust 实现与 Python 参考实现（eval/tools/features_ref.py）对拍。
//! - FR-STA-02 验收：固定事件序列的特征与参考值误差 < 1%（这里要求到 1e-9，两边是同一套公式）；
//! - FR-STA-03 验收：7 天模拟数据的基线与离线 Python 脚本一致。
//!
//! 重新生成对拍文件：python3 eval/tools/features_ref.py --write

use std::path::PathBuf;

use chrono::{Duration, Local, NaiveDate, TimeZone};
use serde::Deserialize;
use serde_json::Value;
use xinqing_hub_core::domain::features::baseline::compute_stats;
use xinqing_hub_core::domain::features::{Baseline, Bucket, WindowFeatures};
use xinqing_hub_core::infra::templates::{AppCategories, BaselineDefault, TemplateDirs};
use xinqing_hub_core::pipeline::{StatePipeline, replay};
use xqp::Up;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn dirs() -> TemplateDirs {
    TemplateDirs::factory_only(repo().join("hub_templates"))
}

#[derive(Deserialize)]
struct GoldenWindow {
    start_ts: u64,
    end_ts: u64,
    features: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
struct GoldenSeq {
    start_min: u32,
    windows: Vec<GoldenWindow>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    start_min: u32,
    events: Vec<Up>,
}

fn golden() -> serde_json::Map<String, Value> {
    let text =
        std::fs::read_to_string(repo().join("eval/datasets/e_features.golden.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// 用 Hub 的流水线回放（冷启动基线，起始本地时刻 `start_min`），返回每个窗口的 (起止, 特征 JSON)。
fn run(events: &[Up], start_min: u32) -> Vec<(u64, u64, Value)> {
    let d = dirs();
    let base = Baseline::from_defaults(&BaselineDefault::load(&d).unwrap());
    let day = NaiveDate::from_ymd_opt(2026, 10, 5)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let start =
        Local.from_local_datetime(&day).earliest().unwrap() + Duration::minutes(start_min as i64);
    let mut p = StatePipeline::new(base, AppCategories::load(&d).unwrap(), start);
    replay(&mut p, events, |_| None)
        .windows
        .into_iter()
        .map(|w| {
            (
                w.window.start_ts,
                w.window.end_ts,
                serde_json::to_value(&w.window.features).unwrap(),
            )
        })
        .collect()
}

fn close(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() <= 1e-9 * y.abs().max(1.0),
        _ => a.is_null() && b.is_null(),
    }
}

fn check(name: &str, events: &[Up], g: &GoldenSeq) -> usize {
    let got = run(events, g.start_min);
    assert_eq!(
        got.len(),
        g.windows.len(),
        "{name}：窗口数 Rust {} ≠ Python {}",
        got.len(),
        g.windows.len()
    );
    for (i, ((s, e, f), w)) in got.iter().zip(&g.windows).enumerate() {
        assert_eq!(
            (*s, *e),
            (w.start_ts, w.end_ts),
            "{name} 第 {} 个窗口的起止不同",
            i + 1
        );
        let f = f.as_object().unwrap();
        for (k, rv) in f {
            let pv = w
                .features
                .get(k)
                .unwrap_or_else(|| panic!("{name}：参考实现缺少特征 {k}，请重新生成"));
            assert!(
                close(rv, pv),
                "{name} 第 {} 个窗口 {k}：Rust {rv} ≠ Python {pv}",
                i + 1
            );
        }
        assert_eq!(f.len(), w.features.len(), "{name}：特征项数不同");
    }
    got.len()
}

#[test]
fn synthetic_scripts_match_reference() {
    let g = golden();
    for name in ["fluent", "hesitant", "typo_burst"] {
        let text =
            std::fs::read_to_string(repo().join(format!("tools/xq-sim/scripts/{name}.jsonl")))
                .unwrap();
        let events: Vec<Up> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let seq: GoldenSeq = serde_json::from_value(g[name].clone()).unwrap();
        assert!(check(name, &events, &seq) > 10);
    }
}

#[test]
fn fixed_cases_match_reference() {
    let g = golden();
    let text =
        std::fs::read_to_string(repo().join("eval/datasets/e_features_cases.jsonl")).unwrap();
    let mut n = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let c: Case = serde_json::from_str(line).unwrap();
        let seq: GoldenSeq = serde_json::from_value(
            g.get(&c.id)
                .unwrap_or_else(|| panic!("{} 不在对拍文件中，请重新生成", c.id))
                .clone(),
        )
        .unwrap();
        assert_eq!(seq.start_min, c.start_min);
        assert!(check(&c.id, &c.events, &seq) > 0, "{} 没有产生窗口", c.id);
        n += 1;
    }
    assert_eq!(n, 10, "FR-STA-02 验收要求 10 组固定事件序列");
}

#[derive(Deserialize)]
struct BaselineGolden {
    samples: Vec<Sample>,
    expected: Expected,
}

#[derive(Deserialize)]
struct Sample {
    hour: u32,
    kpm: Option<f64>,
    iki_med: Option<f64>,
    iki_iqr: Option<f64>,
    bs_rate: f64,
    dwell_med: Option<f64>,
}

#[derive(Deserialize)]
struct Expected {
    windows: u32,
    rows: Vec<Row>,
}

#[derive(Deserialize)]
struct Row {
    bucket: String,
    feature: String,
    med: f64,
    mad: f64,
    n: u32,
}

#[test]
fn seven_day_baseline_matches_reference() {
    let text =
        std::fs::read_to_string(repo().join("eval/datasets/e_baseline.golden.json")).unwrap();
    let g: BaselineGolden = serde_json::from_str(&text).unwrap();
    let windows: Vec<WindowFeatures> = g
        .samples
        .iter()
        .map(|s| WindowFeatures {
            hour: s.hour,
            kpm: s.kpm,
            iki_med: s.iki_med,
            iki_iqr: s.iki_iqr,
            bs_rate: s.bs_rate,
            dwell_med: s.dwell_med,
            ..Default::default()
        })
        .collect();
    let stats = compute_stats(&windows);
    assert_eq!(stats.windows, g.expected.windows);
    assert_eq!(
        stats.rows.len(),
        g.expected.rows.len(),
        "统计项数不同：Rust {:?}",
        stats
            .rows
            .iter()
            .map(|r| (r.bucket.as_str(), r.feature))
            .collect::<Vec<_>>()
    );
    for e in &g.expected.rows {
        let bucket = if e.bucket == "day" {
            Bucket::Day
        } else {
            Bucket::Night
        };
        let r = stats
            .rows
            .iter()
            .find(|r| r.bucket == bucket && r.feature == e.feature)
            .unwrap_or_else(|| panic!("Rust 缺少 {} {}", e.bucket, e.feature));
        assert_eq!(r.n, e.n, "{} {} 样本数", e.bucket, e.feature);
        assert!(
            (r.value.med - e.med).abs() < 1e-9 && (r.value.mad - e.mad).abs() < 1e-9,
            "{} {}：Rust {}/{} ≠ Python {}/{}",
            e.bucket,
            e.feature,
            r.value.med,
            r.value.mad,
            e.med,
            e.mad
        );
    }
}
