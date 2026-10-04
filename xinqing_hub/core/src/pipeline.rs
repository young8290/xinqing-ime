//! 状态识别流水线：XQP 上行事件 → 窗口 → 特征 → 规则（→ 由调用方取 Jev 结果后融合）。
//!
//! 同步、确定性实现：Hub 的 `feature_pipeline` 任务和 `xq-replay` 回放工具共用这一份逻辑。

use std::collections::VecDeque;

use chrono::{DateTime, Duration, Local, Timelike};
use serde::Serialize;
use xqp::{KeySrc, Up};

use crate::domain::features::{
    Baseline, SessionCtx, SessionTracker, TypoDetector, WindowBuf, WindowCutter, WindowFeatures,
    compute,
};
use crate::domain::fusion::{Fusion, FusionOut, JevVerdict};
use crate::domain::rules::{self, Hints};
use crate::infra::templates::{AppCat, AppCategories};

const RECENT_WINDOWS: usize = 3;

#[derive(Debug, Clone, Serialize)]
pub struct WindowOut {
    pub start_ts: u64,
    pub end_ts: u64,
    pub app_cat: AppCat,
    pub features: WindowFeatures,
    pub hints: Hints,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PipelineOut {
    Typo { ts: u64 },
    Window(Box<WindowOut>),
}

pub struct StatePipeline {
    cutter: WindowCutter,
    typo: TypoDetector,
    session: SessionTracker,
    baseline: Baseline,
    apps: AppCategories,
    app_cat: AppCat,
    recent: VecDeque<WindowFeatures>,
    fusion: Fusion,
    /// 会话时间 0 对应的本地时间。
    base_local: DateTime<Local>,
    /// 核心声明了 `tsf_trace` 时，实时打错字判定只看 tsf 来源，避免重复。
    prefer_tsf: bool,
}

impl StatePipeline {
    pub fn new(baseline: Baseline, apps: AppCategories, base_local: DateTime<Local>) -> Self {
        Self {
            cutter: WindowCutter::new(),
            typo: TypoDetector::new(),
            session: SessionTracker::default(),
            baseline,
            apps,
            app_cat: AppCat::Other,
            recent: VecDeque::with_capacity(RECENT_WINDOWS),
            fusion: Fusion::new(),
            base_local,
            prefer_tsf: false,
        }
    }

    pub fn baseline_mut(&mut self) -> &mut Baseline {
        &mut self.baseline
    }

    pub fn push(&mut self, ev: &Up) -> Vec<PipelineOut> {
        let mut out = Vec::new();
        // 焦点切换结束的窗口属于切换前的应用
        let cat_before = self.app_cat;
        match ev {
            Up::Hello { caps, .. } => {
                self.prefer_tsf = caps.iter().any(|c| c == "tsf_trace");
            }
            Up::Focus {
                app,
                blocked: false,
                ..
            } => {
                // 进程名只在内存中分类，之后只保留类别（09 D-05）
                self.app_cat = app
                    .as_deref()
                    .map(|a| self.apps.classify(a))
                    .unwrap_or(AppCat::Other);
            }
            Up::Key {
                ts, kind, vk, src, ..
            } => {
                self.session.on_activity(*ts);
                let want = if self.prefer_tsf {
                    KeySrc::Tsf
                } else {
                    KeySrc::Core
                };
                if *src == want && self.typo.on_key(*ts, *kind, *vk) {
                    out.push(PipelineOut::Typo { ts: *ts });
                }
            }
            _ => {}
        }
        if let Some(w) = self.cutter.push(ev) {
            out.push(PipelineOut::Window(Box::new(self.finish(w, cat_before))));
        }
        out
    }

    pub fn tick(&mut self, now: u64) -> Vec<PipelineOut> {
        let cat = self.app_cat;
        self.cutter
            .tick(now)
            .map(|w| vec![PipelineOut::Window(Box::new(self.finish(w, cat)))])
            .unwrap_or_default()
    }

    fn finish(&mut self, w: WindowBuf, app_cat: AppCat) -> WindowOut {
        let end_local = self.base_local + Duration::milliseconds(w.end_ts() as i64);
        let ctx = SessionCtx {
            session_min: self.session.minutes(),
            hour: end_local.hour(),
            minute: end_local.minute(),
            app_cat,
        };
        let features = compute(&w, &self.baseline, &ctx);
        let recent: Vec<WindowFeatures> = self.recent.iter().cloned().collect();
        let hints = rules::evaluate(&features, &recent);
        if self.recent.len() == RECENT_WINDOWS {
            self.recent.pop_front();
        }
        self.recent.push_back(features.clone());
        self.baseline.windows = self.baseline.windows.saturating_add(1);
        WindowOut {
            start_ts: w.start_ts(),
            end_ts: w.end_ts(),
            app_cat,
            features,
            hints,
        }
    }

    /// 融合（FR-STA-06 / 08）。`verdict = None` 表示 Jev 不可用，走本地规则。
    pub fn fuse(&mut self, w: &WindowOut, verdict: Option<&JevVerdict>) -> FusionOut {
        let now_ms = (self.base_local + Duration::milliseconds(w.end_ts as i64)).timestamp_millis();
        self.fusion.on_window(&w.hints, verdict, now_ms)
    }

    pub fn fusion_mut(&mut self) -> &mut Fusion {
        &mut self.fusion
    }
}

/// 回放结果中的一个窗口。
#[derive(Debug, Clone, Serialize)]
pub struct ReplayWindow {
    pub window: WindowOut,
    pub fusion: FusionOut,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ReplayReport {
    pub windows: Vec<ReplayWindow>,
    pub typos: usize,
}

impl ReplayReport {
    /// 状态切换序列：(窗口序号，从 1 开始，切换后的状态)。
    pub fn changes(&self) -> Vec<(usize, xqp::MoodState)> {
        self.windows
            .iter()
            .enumerate()
            .filter(|(_, w)| w.fusion.changed)
            .map(|(i, w)| (i + 1, w.fusion.shown))
            .collect()
    }
}

/// 按会话时间回放一组上行事件，每 250 ms 补一次 `tick`（17 第 2.3 节）。
/// `judge` 为每个窗口给出 Jev 结果；返回 `None` 表示走本地规则。
pub fn replay(
    p: &mut StatePipeline,
    events: &[Up],
    mut judge: impl FnMut(&WindowOut) -> Option<JevVerdict>,
) -> ReplayReport {
    let mut report = ReplayReport::default();
    let mut handle = |outs: Vec<PipelineOut>, p: &mut StatePipeline, r: &mut ReplayReport| {
        for o in outs {
            match o {
                PipelineOut::Typo { .. } => r.typos += 1,
                PipelineOut::Window(w) => {
                    let v = judge(&w);
                    let fusion = p.fuse(&w, v.as_ref());
                    r.windows.push(ReplayWindow { window: *w, fusion });
                }
            }
        }
    };
    let mut last_ts = 0u64;
    for ev in events {
        if let Some(ts) = ev.ts() {
            let mut t = last_ts + 250;
            while t < ts {
                let outs = p.tick(t);
                handle(outs, p, &mut report);
                t += 250;
            }
            last_ts = ts;
        }
        let outs = p.push(ev);
        handle(outs, p, &mut report);
    }
    let outs = p.tick(last_ts + 60_000);
    handle(outs, p, &mut report);
    report
}
