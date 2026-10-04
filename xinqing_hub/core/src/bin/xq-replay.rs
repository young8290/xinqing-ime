//! xq-replay：离线回放 XQP 脚本，跑一遍 Hub 的状态识别流水线并输出每个窗口的结果。
//!
//! 不需要命名管道、Tauri 或任何密钥，用于开发调试、E-STATE 回放评测和 CI 回归。
//! `--mock-jev` 用与 tools/mock-ai 相同的“按本地提示给概率”规则模拟 Jev，
//! 不加时走降级路径（FR-STA-08，只用本地规则）。

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{Local, NaiveTime, TimeZone};
use clap::Parser;
use xinqing_hub_core::domain::features::{Baseline, Bucket};
use xinqing_hub_core::domain::fusion::JevVerdict;
use xinqing_hub_core::domain::rules::{Hint, Hints};
use xinqing_hub_core::infra::templates::{AppCategories, BaselineDefault, TemplateDirs, read_toml};
use xinqing_hub_core::pipeline::{StatePipeline, replay};
use xqp::{MoodState, Up};

#[derive(Parser, Debug)]
#[command(name = "xq-replay", about = "心晴状态识别离线回放")]
struct Args {
    /// 回放脚本（JSONL）
    script: PathBuf,
    /// 出厂模板目录
    #[arg(long, default_value = "hub_templates")]
    templates: PathBuf,
    /// 模拟的起始本地时间 HH:MM（FR-DMO-01 `--start-at`）
    #[arg(long)]
    start_at: Option<String>,
    /// 固定基线文件（格式同 baseline_default.toml，FR-DMO-01 `--baseline`）
    #[arg(long)]
    baseline: Option<PathBuf>,
    /// 用确定性的模拟 Jev 结果走完整融合路径
    #[arg(long)]
    mock_jev: bool,
    /// 每个窗口输出一行 JSON
    #[arg(long)]
    json: bool,
}

fn mock_verdict(h: &Hints) -> JevVerdict {
    let (choice, p) = if h.has(Hint::HesitationHint) {
        (MoodState::Hesitant, 0.85)
    } else if h.has(Hint::AgitationHint) {
        (MoodState::Agitated, 0.82)
    } else if h.has(Hint::FatigueHint) || h.has(Hint::LateNight) {
        (MoodState::Tired, 0.80)
    } else if h.has(Hint::LowHint) {
        (MoodState::Low, 0.84)
    } else {
        (MoodState::Fluent, 0.88)
    };
    let rest = (1.0 - p) / 4.0;
    let probs: HashMap<MoodState, f64> = [
        MoodState::Fluent,
        MoodState::Hesitant,
        MoodState::Low,
        MoodState::Agitated,
        MoodState::Tired,
    ]
    .into_iter()
    .map(|s| (s, if s == choice { p } else { rest }))
    .collect();
    JevVerdict {
        probs,
        choice,
        confidence: p,
        valence: 2.0,
        need_comfort: 0.4,
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let dirs = TemplateDirs::factory_only(&args.templates);
    let defaults = match &args.baseline {
        Some(p) => read_toml::<BaselineDefault>(p)?,
        None => BaselineDefault::load(&dirs)?,
    };
    let mut baseline = Baseline::from_defaults(&defaults);
    if args.baseline.is_some() {
        // 指定基线文件时把它当作个人基线使用，保证回放可复现
        for (bucket, map) in [
            (Bucket::Day, &defaults.day),
            (Bucket::Night, &defaults.night),
        ] {
            for (f, v) in map {
                baseline.set_personal(bucket, f, *v);
            }
        }
        baseline.windows = u32::MAX / 2;
    }
    let apps = AppCategories::load(&dirs)?;
    let start = match &args.start_at {
        Some(s) => {
            let t = NaiveTime::parse_from_str(s, "%H:%M").context("--start-at 格式应为 HH:MM")?;
            let naive = Local::now().date_naive().and_time(t);
            Local
                .from_local_datetime(&naive)
                .earliest()
                .context("无效的本地时间")?
        }
        None => Local::now(),
    };
    let mut p = StatePipeline::new(baseline, apps, start);

    let text = std::fs::read_to_string(&args.script)
        .with_context(|| format!("读取 {}", args.script.display()))?;
    let events = text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| {
            serde_json::from_str::<Up>(l)
                .with_context(|| format!("{}:{}", args.script.display(), i + 1))
        })
        .collect::<Result<Vec<_>>>()?;
    let report = replay(&mut p, &events, |w| {
        args.mock_jev.then(|| mock_verdict(&w.hints))
    });

    for (i, r) in report.windows.iter().enumerate() {
        let (w, f) = (&r.window, &r.fusion);
        let n = i + 1;
        if args.json {
            println!(
                "{}",
                serde_json::json!({"window": n, "start_ts": w.start_ts, "end_ts": w.end_ts,
                    "app_cat": w.app_cat, "features": w.features, "hints": w.hints.joined(),
                    "cand": f.cand, "shown": f.shown, "changed": f.changed, "conflict": f.conflict})
            );
        } else {
            println!(
                "#{n:<3} {:>7.1}s–{:>7.1}s keys={:<3} kpm={:<6} bs={:.2} pause={} flips={} hints=[{}] → {}{}",
                w.start_ts as f64 / 1000.0,
                w.end_ts as f64 / 1000.0,
                w.features.n_keys,
                w.features
                    .kpm
                    .map(|k| format!("{k:.0}"))
                    .unwrap_or("-".into()),
                w.features.bs_rate,
                w.features.pause_cnt,
                w.features.page_flips,
                w.hints.joined(),
                f.shown.as_str(),
                if f.changed { "  ← 切换" } else { "" }
            );
        }
    }
    if !args.json {
        let seq: Vec<String> = report
            .changes()
            .iter()
            .map(|(w, s)| format!("#{w} {}", s.as_str()))
            .collect();
        eprintln!(
            "共 {} 个窗口，打错字 {} 次，状态切换：{}",
            report.windows.len(),
            report.typos,
            if seq.is_empty() {
                "无".into()
            } else {
                seq.join(" → ")
            }
        );
    }
    Ok(())
}
