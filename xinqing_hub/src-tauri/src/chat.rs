//! 启动对话服务（C-07、C-08，03 第 2.2 节 `chat` 任务）并实现它用的 [`ChatPort`]。
//!
//! 回复经 `chat:delta` / `chat:done` / `chat:error` 推给对话窗口；危机识别命中时发布 `HubEvent::Safety`、
//! 推 `safety:triggered` 并打开对话窗口（FR-SAF-02 第 2 条）。

use std::ops::Deref;
use std::sync::Arc;

use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use xinqing_hub_core::bus::HubEvent;
use xinqing_hub_core::chat::{ChatEvent, ChatPort, ChatService};
use xinqing_hub_core::domain::chat::{ChatCopy, ChatPrompts};
use xinqing_hub_core::domain::comfort::Style;
use xinqing_hub_core::domain::consent::{ConsentItem, ConsentState};
use xinqing_hub_core::domain::safety::CrisisLexicon;
use xinqing_hub_core::domain::settings;
use xinqing_hub_core::domain::validate::BannedWords;
use xinqing_hub_core::infra::store::Db;
use xinqing_hub_core::infra::templates::TemplateDirs;

use crate::events::{ChatDelta, ChatDone, ChatErrorEvent, SafetyTriggered};
use crate::gateway::Ai;
use crate::paths;
use crate::sensing::Sensing;
use crate::state::AppState;
use crate::windows::{self, WindowTarget};

/// 由 Tauri 托管。模板加载失败时为 `None`，对话命令返回通用错误，其余功能不受影响。
pub struct Chat(pub Option<Arc<ChatService>>);

/// 须在 `AppState`、`Arc<Ai>`、`Sensing` 都托管之后调用。
pub fn start(app: &AppHandle) {
    let loaded = paths::templates_dir()
        .ok_or_else(|| anyhow::anyhow!("找不到 hub_templates"))
        .and_then(|dir| {
            let dirs = TemplateDirs::factory_only(dir);
            Ok((
                ChatPrompts::load(&dirs)?,
                ChatCopy::load(&dirs)?,
                BannedWords::load(&dirs)?,
                CrisisLexicon::load(&dirs)?,
            ))
        });
    let service = match loaded {
        Ok((prompts, copy, banned, lex)) => Some(Arc::new(ChatService::new(
            Arc::new(ShellPort { app: app.clone() }),
            app.state::<Arc<Ai>>().inner().clone(),
            crate::sim::clock(),
            prompts,
            copy,
            Arc::new(banned),
            Arc::new(lex),
        ))),
        Err(e) => {
            eprintln!("对话不可用：{e}");
            None
        }
    };
    app.manage(Chat(service));
}

struct ShellPort {
    app: AppHandle,
}

impl ChatPort for ShellPort {
    fn db(&self) -> Box<dyn Deref<Target = Db> + '_> {
        Box::new(self.app.state::<AppState>().inner().db())
    }

    fn style(&self) -> Style {
        let v = settings::get(&self.app.state::<AppState>().db(), "care.style").ok();
        Style::parse(v.as_ref().and_then(|v| v.as_str()).unwrap_or(""))
    }

    fn summary_allowed(&self) -> bool {
        ConsentState::load(&self.app.state::<AppState>().db())
            .is_ok_and(|c| c.granted(ConsentItem::LlmSummary))
    }

    fn emit(&self, ev: ChatEvent) {
        let r = match ev {
            ChatEvent::Delta { request_id, text } => ChatDelta {
                request_id,
                text_delta: text,
            }
            .emit(&self.app),
            ChatEvent::Done {
                request_id,
                session_id,
                message_id,
                text,
                ai_generated,
                stopped,
            } => ChatDone {
                request_id,
                session_id: session_id as u32,
                message_id: message_id.map(|id| id as u32),
                text,
                ai_generated,
                stopped,
            }
            .emit(&self.app),
            ChatEvent::Error {
                request_id,
                session_id,
                reason,
            } => ChatErrorEvent {
                request_id,
                session_id: session_id as u32,
                reason,
            }
            .emit(&self.app),
        };
        if let Err(e) = r {
            eprintln!("推送对话事件失败：{e}");
        }
    }

    fn safety(&self, session_id: i64) {
        let _ = self
            .app
            .state::<Sensing>()
            .bus
            .send(HubEvent::Safety { session_id });
        if let Err(e) = (SafetyTriggered {
            session_id: session_id as u32,
        })
        .emit(&self.app)
        {
            eprintln!("推送 safety:triggered 失败：{e}");
        }
        if let Err(e) = windows::open(&self.app, WindowTarget::Chat) {
            eprintln!("打开对话窗口失败：{e:?}");
        }
    }

    fn note(&self, msg: &str) {
        eprintln!("{msg}");
    }
}
