//! 用合成回放脚本做端到端回归：窗口切分 → 特征 → 规则 → 融合（FR-STA-06 验收口径）。
//! 脚本由 tools/xq-sim/gen_synthetic.py 生成；真实录制的 E-STATE 脚本另行评测。

use std::path::PathBuf;

use chrono::{Local, TimeZone};
use xinqing_hub_core::domain::features::Baseline;
use xinqing_hub_core::domain::fusion::JevVerdict;
use xinqing_hub_core::domain::rules::Hint;
use xinqing_hub_core::infra::templates::{AppCategories, BaselineDefault, TemplateDirs};
use xinqing_hub_core::pipeline::{ReplayReport, StatePipeline, replay};
use xqp::{MoodState, Up};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn verdict(w: &xinqing_hub_core::pipeline::WindowOut) -> Option<JevVerdict> {
    let (choice, p) = if w.hints.has(Hint::HesitationHint) {
        (MoodState::Hesitant, 0.85)
    } else {
        (MoodState::Fluent, 0.88)
    };
    Some(JevVerdict {
        probs: [(choice, p)].into_iter().collect(),
        choice,
        confidence: p,
        valence: 2.0,
        need_comfort: 0.3,
    })
}

fn run(name: &str, jev: bool) -> ReplayReport {
    let dirs = TemplateDirs::factory_only(repo().join("hub_templates"));
    let base = Baseline::from_defaults(&BaselineDefault::load(&dirs).unwrap());
    let start = Local.with_ymd_and_hms(2026, 10, 5, 14, 0, 0).unwrap();
    let mut p = StatePipeline::new(base, AppCategories::load(&dirs).unwrap(), start);
    let text =
        std::fs::read_to_string(repo().join(format!("tools/xq-sim/scripts/{name}.jsonl"))).unwrap();
    let events: Vec<Up> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    replay(&mut p, &events, |w| if jev { verdict(w) } else { None })
}

#[test]
fn fluent_script_never_switches() {
    let r = run("fluent", true);
    assert!(r.windows.len() >= 30);
    assert!(
        r.changes().is_empty(),
        "流畅脚本不应出现状态切换：{:?}",
        r.changes()
    );
}

#[test]
fn hesitant_switches_on_second_hint_window_without_jitter() {
    let r = run("hesitant", true);
    let first_hint = r
        .windows
        .iter()
        .position(|w| w.window.hints.has(Hint::HesitationHint))
        .unwrap();
    let changes = r.changes();
    assert_eq!(
        changes.first(),
        Some(&(first_hint + 2, MoodState::Hesitant))
    );
    assert_eq!(changes.last().map(|c| c.1), Some(MoodState::Fluent));
    for pair in changes.windows(2) {
        assert!(
            pair[1].0 - pair[0].0 >= 2,
            "相邻两次切换间隔应 ≥ 2 个窗口：{changes:?}"
        );
    }
}

#[test]
fn rule_only_mode_still_detects_hesitation() {
    let r = run("hesitant", false);
    assert!(r.changes().iter().any(|c| c.1 == MoodState::Hesitant));
}

#[test]
fn typo_script_triggers_r1() {
    let r = run("typo_burst", true);
    assert!(r.typos >= 5, "打错字次数 {}", r.typos);
    let in_windows: u32 = r.windows.iter().map(|w| w.window.features.typo_cnt).sum();
    assert!(in_windows > 0);
}
