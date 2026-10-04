//! typer：按节奏脚本经 `SendInput` 发键（按键照常经过 TSF 和输入法），
//! 用于 TC-PERF-01 性能测试与 TC-STA-04 打错字测试（12 第 2 节，17 第 4 节）。
//!
//! 发键只在 Windows 上可用；`gen`、`check` 和 `run --dry-run` 任何平台都能跑。

mod r#gen;
mod r1;
mod script;
#[cfg(windows)]
mod send;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};

use script::Step;

#[derive(Parser, Debug)]
#[command(name = "typer", about = "心晴节奏打字脚本")]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// 生成节奏脚本
    Gen {
        #[command(subcommand)]
        kind: GenKind,
    },
    /// 校验脚本，并用离线复刻的 R1 规则核对标注
    Check { script: PathBuf },
    /// 倒计时后向前台窗口发键，并记录每一键的实际发送时刻
    Run {
        script: PathBuf,
        /// 倍速：计划时刻除以它（0.5 更慢，2 更快）
        #[arg(long, default_value_t = 1.0)]
        speed: f64,
        /// 开始前倒数几秒，留时间切到目标窗口
        #[arg(long, default_value_t = 5)]
        countdown: u32,
        /// 每键日志（CSV：idx,key,label,planned_ms,actual_us,lag_us）；缺省写到标准输出
        #[arg(long)]
        log: Option<PathBuf>,
        /// 不发键，只打印计划
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
enum GenKind {
    /// TC-PERF-01：匀速长打（缺省 8 键/秒、10,000 键）
    Perf {
        #[arg(long, default_value_t = 10_000)]
        keys: usize,
        /// 每秒键数
        #[arg(long, default_value_t = 8.0)]
        rate: f64,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// TC-STA-04：在若干词里注入相邻键手误（标注 typo）和非手误退格（标注 bs_*）
    R1 {
        #[arg(long, default_value_t = 200)]
        words: usize,
        #[arg(long, default_value_t = 40)]
        typos: usize,
        #[arg(long, default_value_t = 20)]
        backspaces: usize,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    match Args::parse().cmd {
        Cmd::Gen { kind } => generate(kind),
        Cmd::Check { script } => check(&load(&script)?),
        Cmd::Run {
            script,
            speed,
            countdown,
            log,
            dry_run,
        } => {
            ensure!(speed > 0.0, "倍速必须大于 0");
            let steps = load(&script)?;
            if speed != 1.0 && steps.iter().any(|s| s.label.is_some()) {
                eprintln!(
                    "注意：标注是按原速核对的，倍速 {speed} 下手误与退格的间隔变了，不能用来算 R1 的召回率和精确率"
                );
            }
            if dry_run {
                return check(&steps);
            }
            run(&steps, speed, countdown, log.as_deref())
        }
    }
}

fn load(path: &Path) -> Result<Vec<Step>> {
    let text = fs::read_to_string(path).with_context(|| format!("读不了 {}", path.display()))?;
    let steps = script::parse(&text).with_context(|| path.display().to_string())?;
    ensure!(!steps.is_empty(), "{} 里没有键", path.display());
    Ok(steps)
}

fn generate(kind: GenKind) -> Result<()> {
    let (header, steps, out) = match kind {
        GenKind::Perf {
            keys,
            rate,
            seed,
            out,
        } => (
            format!(
                "typer gen perf --keys {keys} --rate {rate} --seed {seed}\nTC-PERF-01：匀速 {rate} 键/秒，共 {keys} 键"
            ),
            r#gen::perf(keys, rate, seed)?,
            out,
        ),
        GenKind::R1 {
            words,
            typos,
            backspaces,
            seed,
            out,
        } => (
            format!(
                "typer gen r1 --words {words} --typos {typos} --backspaces {backspaces} --seed {seed}\n\
                 TC-STA-04：{words} 个词，{typos} 处相邻键手误（改对的键标 typo），{backspaces} 处非手误退格（标 bs_*）"
            ),
            r#gen::r1(words, typos, backspaces, seed)?,
            out,
        ),
    };
    let text = script::format(&header, &steps);
    match out {
        Some(p) => fs::write(&p, text).with_context(|| format!("写不了 {}", p.display()))?,
        None => print!("{text}"),
    }
    Ok(())
}

fn check(steps: &[Step]) -> Result<()> {
    let times = script::timeline(steps);
    let total = *times.last().unwrap_or(&0);
    println!(
        "{} 键，时长 {:.1} 秒，平均 {:.2} 键/秒",
        steps.len(),
        total as f64 / 1000.0,
        rate(steps.len(), total)
    );
    let mut labels: Vec<(&str, usize)> = Vec::new();
    for l in steps.iter().filter_map(|s| s.label.as_deref()) {
        match labels.iter_mut().find(|(k, _)| *k == l) {
            Some((_, n)) => *n += 1,
            None => labels.push((l, 1)),
        }
    }
    for (l, n) in &labels {
        println!("  标注 {l}：{n}");
    }
    let hits = r1::hits(steps, &times);
    let expected: Vec<usize> = (0..steps.len())
        .filter(|&i| steps[i].label.as_deref() == Some("typo"))
        .collect();
    let missed = expected.iter().filter(|i| !hits.contains(i)).count();
    let extra = hits.iter().filter(|i| !expected.contains(i)).count();
    println!(
        "R1 复刻：命中 {}，标了 typo {}，漏 {missed}，多 {extra}",
        hits.len(),
        expected.len()
    );
    ensure!(
        missed == 0 && extra == 0,
        "标注与 R1 规则对不上（按原速回放时）：以这份脚本测出的召回率、精确率没有意义"
    );
    Ok(())
}

fn rate(keys: usize, ms: u64) -> f64 {
    if ms == 0 {
        0.0
    } else {
        (keys.saturating_sub(1)) as f64 * 1000.0 / ms as f64
    }
}

/// 一键的发送记录
#[cfg_attr(not(windows), allow(dead_code))]
struct Sent {
    planned_ms: u64,
    actual_us: u64,
}

#[cfg(windows)]
fn run(steps: &[Step], speed: f64, countdown: u32, log: Option<&Path>) -> Result<()> {
    use std::time::{Duration, Instant};

    for n in (1..=countdown).rev() {
        eprintln!("{n} 秒后开始，请切到目标窗口并打开心晴输入法……");
        std::thread::sleep(Duration::from_secs(1));
    }
    let _timer = send::HighResTimer::new();
    let fg = send::foreground();
    let planned: Vec<u64> = script::timeline(steps)
        .into_iter()
        .map(|t| (t as f64 / speed).round() as u64)
        .collect();
    let mut sent = Vec::with_capacity(steps.len());
    let start = Instant::now();
    for (s, &p) in steps.iter().zip(&planned) {
        let target = start + Duration::from_millis(p);
        // 先睡到目标前 2 ms，再自旋到点：Windows 的 sleep 粒度即使调到 1 ms 也会晚到
        if let Some(d) = target
            .checked_duration_since(Instant::now())
            .and_then(|d| d.checked_sub(Duration::from_millis(2)))
        {
            std::thread::sleep(d);
        }
        while Instant::now() < target {
            std::hint::spin_loop();
        }
        ensure!(
            send::foreground() == fg,
            "前台窗口变了，已在第 {} 键停下，免得把按键打进别的窗口",
            sent.len() + 1
        );
        let at = start.elapsed();
        send::tap(s.key.vk())?;
        sent.push(Sent {
            planned_ms: p,
            actual_us: at.as_micros() as u64,
        });
    }
    report(steps, &sent, log)
}

#[cfg(not(windows))]
fn run(_: &[Step], _: f64, _: u32, _: Option<&Path>) -> Result<()> {
    anyhow::bail!(
        "发键（SendInput）只在 Windows 上可用；这里可以用 `typer check` 或 `run --dry-run` 检查脚本"
    )
}

#[cfg_attr(not(windows), allow(dead_code))]
fn report(steps: &[Step], sent: &[Sent], log: Option<&Path>) -> Result<()> {
    let mut out: Box<dyn Write> = match log {
        Some(p) => {
            Box::new(fs::File::create(p).with_context(|| format!("写不了 {}", p.display()))?)
        }
        None => Box::new(std::io::stdout().lock()),
    };
    writeln!(out, "idx,key,label,planned_ms,actual_us,lag_us")?;
    let mut lags: Vec<i64> = Vec::with_capacity(sent.len());
    for (i, (s, r)) in steps.iter().zip(sent).enumerate() {
        let lag = r.actual_us as i64 - (r.planned_ms * 1000) as i64;
        lags.push(lag);
        writeln!(
            out,
            "{i},{},{},{},{},{lag}",
            s.key,
            s.label.as_deref().unwrap_or(""),
            r.planned_ms,
            r.actual_us
        )?;
    }
    out.flush()?;
    lags.sort_unstable();
    let pct = |q: f64| lags[((lags.len() - 1) as f64 * q).round() as usize];
    eprintln!(
        "已发 {} 键；发送时刻比计划晚 p50 {} µs、p99 {} µs、最多 {} µs",
        sent.len(),
        pct(0.5),
        pct(0.99),
        lags.last().copied().unwrap_or(0)
    );
    Ok(())
}
