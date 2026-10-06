//! 启动温柔改写服务（C-09）并实现它用的 [`RewritePort`]：回核心走 XQP，`rewrite_log` 排队写库，
//! 危机时记 `safety_log` 并推 `safety:invite`（ADR 0028）。

use std::sync::Arc;

use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use xinqing_hub_core::domain::consent::{ConsentItem, ConsentState};
use xinqing_hub_core::domain::rewrite::{self as rw, RewritePrompt};
use xinqing_hub_core::domain::safety::CrisisLexicon;
use xinqing_hub_core::domain::validate::BannedWords;
use xinqing_hub_core::rewrite::{RewriteLog, RewritePort, RewriteService};
use xqp::Down;

use crate::events::{SafetyInvite, SafetyInviteSource};
use crate::gateway::Ai;
use crate::paths;
use crate::sensing::Sensing;
use crate::state::AppState;

/// 须在 `AppState`、`Arc<Ai>`、`Sensing` 都托管之后调用。模板加载失败时不启动（记日志）：核心等 10 秒后提示
/// “暂时改写不了”，其余功能不受影响。
pub fn start(app: &AppHandle) {
    let loaded = paths::hub_template_dirs()
        .and_then(|dirs| {
            Ok((
                RewritePrompt::load(&dirs)?,
                CrisisLexicon::load(&dirs)?,
                BannedWords::load(&dirs)?,
            ))
        });
    let (prompt, lexicon, banned) = match loaded {
        Ok(t) => t,
        Err(e) => {
            eprintln!("温柔改写不可用：{e}");
            return;
        }
    };
    let bus = app.state::<Sensing>().bus.subscribe();
    let service = RewriteService::new(
        Arc::new(ShellPort { app: app.clone() }),
        app.state::<Arc<Ai>>().inner().clone(),
        crate::sim::clock(),
        prompt,
        Arc::new(lexicon),
        Arc::new(banned),
    );
    tauri::async_runtime::spawn(service.run(bus));
}

struct ShellPort {
    app: AppHandle,
}

impl RewritePort for ShellPort {
    fn allowed(&self) -> bool {
        ConsentState::load(&self.app.state::<AppState>().db())
            .is_ok_and(|c| c.granted(ConsentItem::Rewrite))
    }

    fn reply(&self, d: Down) {
        self.app.state::<Sensing>().xqp.send(d);
    }

    fn crisis(&self, ts: i64) {
        if let Err(e) = self
            .app
            .state::<AppState>()
            .writer()
            .write_sync(|db| db.safety_log(ts, "lexicon"))
        {
            eprintln!("写入 safety_log 失败：{e}");
        }
        let ev = SafetyInvite {
            source: SafetyInviteSource::Rewrite,
        };
        if let Err(e) = ev.emit(&self.app) {
            eprintln!("推送 safety:invite 失败：{e}");
        }
    }

    fn log(&self, r: RewriteLog) {
        self.app
            .state::<AppState>()
            .writer()
            .enqueue("rewrite_log", move |db| {
                db.rewrite_log_insert(
                    r.ts,
                    rw::source_str(r.source),
                    rw::style_str(r.style),
                    r.chosen,
                    r.len_in,
                    r.len_out,
                    rw::outcome_str(r.outcome),
                )
            });
    }

    fn note(&self, msg: &str) {
        eprintln!("{msg}");
    }
}
