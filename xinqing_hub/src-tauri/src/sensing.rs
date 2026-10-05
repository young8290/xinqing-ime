//! 启动 XQP 客户端和实时感知任务（17 第 2.2 节第 5 步），并实现感知任务用的 [`SensePort`]。
//!
//! 传输：设置了环境变量 `XQ_XQP_TCP`（如 `127.0.0.1:18765`）时连 `xq-sim --tcp`，任何平台可用；
//! 否则 Windows 上连命名管道 `xinqing_tap`（调试构建为 `xinqing_tap_dev`），其他平台不连接。

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use tauri::{AppHandle, Manager};
use tokio::sync::{broadcast, mpsc};
use xinqing_hub_core::bus::{self, HubEvent};
use xinqing_hub_core::domain::explain::Explanation;
use xinqing_hub_core::domain::features::persist;
use xinqing_hub_core::domain::features::{Baseline, BaselineStats};
use xinqing_hub_core::domain::feedback;
use xinqing_hub_core::domain::status::StatusSnapshot;
use xinqing_hub_core::infra::templates::{AppCategories, BaselineDefault, TemplateDirs};
use xinqing_hub_core::infra::xqp::{Connector, LinkOptions, TcpConnector, XqpHandle, XqpLink};
use xinqing_hub_core::pipeline::StatePipeline;
use xinqing_hub_core::sense::{Sense, SenseCmd, SensePort, WindowRecord};
use xqp::{Down, OpenTarget};

use crate::commands::emit_status;
use crate::paths;
use crate::sim;
use crate::state::AppState;
use crate::windows::{self, WindowTarget};

pub const XQP_TCP_ENV: &str = "XQ_XQP_TCP";

/// 由 Tauri 托管，命令通过它下发 XQP 消息、控制感知任务。
pub struct Sensing {
    pub xqp: XqpHandle,
    pub cmds: mpsc::Sender<SenseCmd>,
    pub bus: broadcast::Sender<HubEvent>,
    /// 与感知任务同一份基线（启动和每次重算、重置后更新），重建历史状态的解释时用；模板加载失败时为 `None`。
    baseline: RwLock<Option<Baseline>>,
    /// 调试构建设了 `XQ_SIM_BASELINE`：基线固定，不重算也不重置（ADR 0020）
    pub fixed_baseline: bool,
}

impl Sensing {
    /// 暂停 / 恢复感知。感知任务没有运行时（模板加载失败）直接改快照并下发。
    pub fn pause(&self, app: &AppHandle, on: bool) {
        if self.cmds.try_send(SenseCmd::Pause(on)).is_ok() {
            return;
        }
        let (snap, changed) = app
            .state::<AppState>()
            .update_status(|s| StatusSnapshot::set(&mut s.paused, on));
        if changed {
            emit_status(app, snap);
        }
        self.xqp.send(Down::Pause { on });
    }

    pub fn baseline(&self) -> Option<Baseline> {
        self.baseline
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 基线重算或重置后，同步外壳这份拷贝。
    pub fn apply_baseline(&self, stats: &BaselineStats) {
        if let Some(b) = self
            .baseline
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            b.apply(stats);
        }
    }
}

/// `cfg` 是按当前同意状态生成的首个下行配置。须在 `AppState` 托管之后调用。
pub fn start(app: &AppHandle, cfg: Down) -> Sensing {
    let (bus, _) = bus::channel();
    let (cmds, cmd_rx) = mpsc::channel(16);
    let (xqp, link_rx) = match connector() {
        Some(c) => {
            let (link, handle, rx) = XqpLink::new(c, cfg, LinkOptions::default());
            tauri::async_runtime::spawn(link.run());
            (handle, rx)
        }
        None => {
            eprintln!(
                "未连接输入法：非 Windows 平台请设置 {XQP_TCP_ENV}=127.0.0.1:18765 并运行 xq-sim --tcp"
            );
            (XqpHandle::pair().0, mpsc::channel(1).1)
        }
    };
    let mut baseline = None;
    let fixed = sim::fixed_baseline();
    let fixed_baseline = fixed.is_some();
    match load_pipeline(fixed) {
        Ok(mut p) => {
            restore_unfit(app, &mut p);
            if !fixed_baseline {
                recompute_baseline(app, &mut p);
            }
            baseline = Some(p.baseline().clone());
            let port = Arc::new(ShellPort {
                app: app.clone(),
                fixed_baseline,
            });
            let sense = Sense::new(p, port, xqp.clone(), bus.clone(), sim::clock());
            tauri::async_runtime::spawn(sense.run(link_rx, cmd_rx));
        }
        Err(e) => eprintln!("状态识别不可用：{e}"),
    }
    Sensing {
        xqp,
        cmds,
        bus,
        baseline: RwLock::new(baseline),
        fixed_baseline,
    }
}

fn connector() -> Option<Box<dyn Connector>> {
    if let Ok(addr) = std::env::var(XQP_TCP_ENV) {
        let c = addr
            .parse::<SocketAddr>()
            .map_err(|e| e.to_string())
            .and_then(|a| TcpConnector::new(a).map_err(|e| e.to_string()));
        return match c {
            Ok(c) => Some(Box::new(c)),
            Err(e) => {
                eprintln!("{XQP_TCP_ENV}={addr} 无效：{e}");
                None
            }
        };
    }
    #[cfg(windows)]
    {
        let name = if cfg!(debug_assertions) {
            xqp::PIPE_NAME_DEV
        } else {
            xqp::PIPE_NAME
        };
        Some(Box::new(crate::xqp_pipe::PipeConnector::new(name)))
    }
    #[cfg(not(windows))]
    None
}

/// `fixed` 是 `XQ_SIM_BASELINE` 的固定基线；没有时先用出厂默认值。
fn load_pipeline(fixed: Option<Baseline>) -> anyhow::Result<StatePipeline> {
    let dir = paths::templates_dir().ok_or_else(|| {
        anyhow::anyhow!("找不到 hub_templates（可设置 {}）", paths::TEMPLATES_ENV)
    })?;
    let dirs = TemplateDirs::factory_only(dir);
    // 先用出厂默认值，随后 `recompute_baseline` 用库里最近 7 天的窗口换上个人值
    let baseline = match fixed {
        Some(b) => b,
        None => Baseline::from_defaults(&BaselineDefault::load(&dirs)?),
    };
    Ok(StatePipeline::new(
        baseline,
        AppCategories::load(&dirs)?,
        sim::clock().now(),
    ))
}

/// 启动时重算一次基线（FR-STA-03 第 4 条）：关机期间错过的 04:00 也能补上，冷启动进度也从库里接上。
fn recompute_baseline(app: &AppHandle, p: &mut StatePipeline) {
    let now = chrono::Utc::now().timestamp_millis();
    match app
        .state::<AppState>()
        .writer()
        .write_sync(|db| persist::recompute(db, now))
    {
        Ok(stats) => p.baseline_mut().apply(&stats),
        Err(e) => eprintln!("重算基线失败，先用出厂默认值：{e}"),
    }
}

/// 重放最近 7 天的“不准”，恢复个人阈值上调（FR-STA-07，上调状态只在内存里）。
fn restore_unfit(app: &AppHandle, p: &mut StatePipeline) {
    let now = chrono::Utc::now().timestamp_millis();
    match feedback::recent_unfit(&app.state::<AppState>().db(), now) {
        Ok(rows) => {
            for (state, ts) in rows {
                p.fusion_mut().record_unfit(state, ts);
            }
        }
        Err(e) => eprintln!("读取状态反馈失败：{e}"),
    }
}

struct ShellPort {
    app: AppHandle,
    fixed_baseline: bool,
}

impl SensePort for ShellPort {
    fn update_status(&self, f: &mut dyn FnMut(&mut StatusSnapshot) -> bool) {
        let (snap, changed) = self.app.state::<AppState>().update_status(|s| f(s));
        if changed {
            emit_status(&self.app, snap);
        }
    }

    fn open(&self, target: OpenTarget) {
        let target = match target {
            OpenTarget::Chat => WindowTarget::Chat,
            // 日程与待办在看板里（07 第 2 节）
            OpenTarget::Dashboard | OpenTarget::Schedule => WindowTarget::Dashboard,
            OpenTarget::Settings => WindowTarget::Settings,
            OpenTarget::Onboarding => WindowTarget::Onboarding,
        };
        if let Err(e) = windows::open(&self.app, target) {
            eprintln!("打开窗口失败：{}", e.code);
        }
    }

    fn save_window(&self, rec: &WindowRecord<'_>) {
        let state = self.app.state::<AppState>();
        state.set_auto_state(rec.fusion.shown);
        // 普通数据，排队批量提交（17 第 2.9 节）；窗口与状态记录在同一个任务里，一起成功或一起丢弃
        let (start, end) = (rec.start_ms, rec.end_ms);
        let (app_cat, features, hints) = (
            rec.window.app_cat,
            rec.window.features.clone(),
            rec.window.hints.clone(),
        );
        let (cand, shown, source) = (rec.fusion.cand, rec.fusion.shown, rec.fusion.source);
        state.writer().enqueue("window", move |db| {
            let id = db.insert_window(start, end, app_cat, &features, &hints)?;
            db.insert_mood_state(end, Some(id), cand, shown, source)
                .map(drop)
        });
    }

    fn note(&self, msg: &str) {
        eprintln!("{msg}");
    }

    fn request_exit(&self) {
        self.app.exit(0);
    }

    fn explained(&self, e: &Explanation) {
        self.app.state::<AppState>().set_explanation(e.clone());
    }

    fn recompute_baseline(&self, now_ms: i64) -> Option<BaselineStats> {
        if self.fixed_baseline {
            return None;
        }
        let state = self.app.state::<AppState>();
        match state
            .writer()
            .write_sync(|db| persist::recompute(db, now_ms))
        {
            Ok(stats) => {
                if let Some(sensing) = self.app.try_state::<Sensing>() {
                    sensing.apply_baseline(&stats);
                }
                Some(stats)
            }
            Err(e) => {
                eprintln!("重算基线失败，继续用现有基线：{e}");
                None
            }
        }
    }
}
