//! 启动研究模式服务（FR-DMO-04，B-10，ADR 0026）并实现它用的 [`ResearchPort`]。
//!
//! 到点时推送 `research:invite`，小组件弹出“主动报告心情”面板（可跳过）。邀请后 30 分钟内的自评经
//! `self_report_set` 记为 `source = esm`；点“跳过”走 `research_dismiss`；研究结束删数据走 `research_clear`。

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use xinqing_hub_core::domain::research::{ANSWER_MS, Esm};
use xinqing_hub_core::domain::settings;
use xinqing_hub_core::research::{ResearchPort, ResearchService};

use crate::events::ResearchInvite;
use crate::sensing::Sensing;
use crate::sim;
use crate::state::AppState;

/// 由 Tauri 托管：命令经它判断一次自评是不是邀请的回答。
pub struct Research {
    pub esm: Arc<Mutex<Esm>>,
}

impl Research {
    /// 这次自评是不是邀请的回答（是的话消耗掉待答状态）。
    pub fn take_answer(&self, now_ms: i64) -> bool {
        self.esm
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take_answer(now_ms)
    }

    pub fn dismiss(&self) {
        self.esm.lock().unwrap_or_else(|e| e.into_inner()).dismiss();
    }
}

/// 须在 `AppState`、`Sensing` 都托管之后调用。
pub fn start(app: &AppHandle) {
    let bus = app.state::<Sensing>().bus.subscribe();
    let esm = Arc::new(Mutex::new(Esm::default()));
    app.manage(Research { esm: esm.clone() });
    let service = ResearchService::new(esm, Arc::new(ShellPort { app: app.clone() }), sim::clock());
    tauri::async_runtime::spawn(service.run(bus));
}

struct ShellPort {
    app: AppHandle,
}

impl ResearchPort for ShellPort {
    fn config(&self) -> (String, bool) {
        let db = self.app.state::<AppState>();
        let db = db.db();
        let id = settings::get(&db, "research.id")
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let enabled = settings::get(&db, "research.enabled")
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        (id, enabled)
    }

    fn invite(&self) {
        let ev = ResearchInvite {
            answer_until_ts: (sim::clock().now_ms() + ANSWER_MS) as f64,
        };
        if let Err(e) = ev.emit(&self.app) {
            eprintln!("推送 research:invite 失败：{e}");
        }
    }

    fn note(&self, msg: &str) {
        eprintln!("{msg}");
    }
}
