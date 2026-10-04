//! 启动 XQP 客户端和实时感知任务（17 第 2.2 节第 5 步），并实现感知任务用的 [`SensePort`]。
//!
//! 传输：设置了环境变量 `XQ_XQP_TCP`（如 `127.0.0.1:18765`）时连 `xq-sim --tcp`，任何平台可用；
//! 否则 Windows 上连命名管道 `xinqing_tap`（调试构建为 `xinqing_tap_dev`），其他平台不连接。

use std::net::SocketAddr;
use std::sync::Arc;

use tauri::{AppHandle, Manager};
use tokio::sync::{broadcast, mpsc};
use xinqing_hub_core::bus::{self, HubEvent};
use xinqing_hub_core::domain::explain::Explanation;
use xinqing_hub_core::domain::features::Baseline;
use xinqing_hub_core::domain::feedback;
use xinqing_hub_core::domain::status::StatusSnapshot;
use xinqing_hub_core::infra::clock::SystemClock;
use xinqing_hub_core::infra::templates::{AppCategories, BaselineDefault, TemplateDirs};
use xinqing_hub_core::infra::xqp::{Connector, LinkOptions, TcpConnector, XqpHandle, XqpLink};
use xinqing_hub_core::pipeline::StatePipeline;
use xinqing_hub_core::sense::{Sense, SenseCmd, SensePort, WindowRecord};
use xqp::{Down, OpenTarget};

use crate::commands::emit_status;
use crate::paths;
use crate::state::AppState;
use crate::windows::{self, WindowTarget};

pub const XQP_TCP_ENV: &str = "XQ_XQP_TCP";

/// 由 Tauri 托管，命令通过它下发 XQP 消息、控制感知任务。
pub struct Sensing {
    pub xqp: XqpHandle,
    pub cmds: mpsc::Sender<SenseCmd>,
    pub bus: broadcast::Sender<HubEvent>,
    /// 与感知任务同一份基线（B-04 持久化前是出厂默认值），重建历史状态的解释时用；模板加载失败时为 `None`。
    pub baseline: Option<Baseline>,
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
    match load_pipeline() {
        Ok(mut p) => {
            restore_unfit(app, &mut p);
            baseline = Some(p.baseline().clone());
            let port = Arc::new(ShellPort { app: app.clone() });
            let sense = Sense::new(p, port, xqp.clone(), bus.clone(), Arc::new(SystemClock));
            tauri::async_runtime::spawn(sense.run(link_rx, cmd_rx));
        }
        Err(e) => eprintln!("状态识别不可用：{e}"),
    }
    Sensing {
        xqp,
        cmds,
        bus,
        baseline,
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

fn load_pipeline() -> anyhow::Result<StatePipeline> {
    let dir = paths::templates_dir().ok_or_else(|| {
        anyhow::anyhow!("找不到 hub_templates（可设置 {}）", paths::TEMPLATES_ENV)
    })?;
    let dirs = TemplateDirs::factory_only(dir);
    // 个人基线由每天 04:00 的重算任务写入（B-04）；接入前先用出厂默认值
    let baseline = Baseline::from_defaults(&BaselineDefault::load(&dirs)?);
    Ok(StatePipeline::new(
        baseline,
        AppCategories::load(&dirs)?,
        chrono::Local::now(),
    ))
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
        let db = state.db();
        let w = rec.window;
        let saved = db
            .insert_window(rec.start_ms, rec.end_ms, w.app_cat, &w.features, &w.hints)
            .and_then(|id| {
                db.insert_mood_state(
                    rec.end_ms,
                    Some(id),
                    rec.fusion.cand,
                    rec.fusion.shown,
                    rec.fusion.source,
                )
            });
        if let Err(e) = saved {
            eprintln!("写入特征窗口失败：{e}");
        }
    }

    fn note(&self, msg: &str) {
        eprintln!("{msg}");
    }

    fn explained(&self, e: &Explanation) {
        self.app.state::<AppState>().set_explanation(e.clone());
    }
}
