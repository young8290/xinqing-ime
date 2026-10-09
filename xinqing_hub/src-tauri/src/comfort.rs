//! 启动暖心话服务（C-04，03 第 2.2 节 `comfort` 任务）并实现它用的 [`ComfortPort`]。
//!
//! 显示：推送 `comfort:new`，由小组件一句话区显示（打字机效果、`AI 生成` 标签、小精灵“靠近”都在前端，FR-CMF-04）。
//! 小组件隐藏时不弹窗、不发系统通知，只点亮工具栏天气按钮的小圆点（XQP `badge`），用户打开小组件后熄灭。

use std::collections::HashSet;
use std::sync::Arc;

use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;
use xinqing_hub_core::care::{Comfort, ComfortPort, ComfortService, ComfortSource};
use xinqing_hub_core::domain::comfort::{CareLevel, ComfortPrompt, ComfortTemplates, Style};
use xinqing_hub_core::domain::comfort_feedback;
use xinqing_hub_core::domain::consent::{ConsentItem, ConsentState};
use xinqing_hub_core::domain::settings;
use xinqing_hub_core::domain::validate::BannedWords;
use xinqing_hub_core::infra::store::ComfortRecord;
use xqp::Down;

use crate::events::ComfortNew;
use crate::gateway::Ai;
use crate::paths;
use crate::sensing::Sensing;
use crate::state::AppState;
use crate::windows::WindowTarget;

/// 须在 `AppState`、`Arc<Ai>`、`Sensing` 都托管之后调用。模板加载失败时不启动（记日志），其余功能不受影响。
pub fn start(app: &AppHandle) {
    let loaded = paths::hub_template_dirs().and_then(|dirs| {
        Ok((
            ComfortTemplates::load(&dirs)?,
            ComfortPrompt::load(&dirs)?,
            BannedWords::load(&dirs)?,
        ))
    });
    let (templates, prompt, banned) = match loaded {
        Ok(t) => t,
        Err(e) => {
            eprintln!("暖心话不可用：{e}");
            return;
        }
    };
    let bus = app.state::<Sensing>().bus.subscribe();
    let gateway = app.state::<Arc<Ai>>().inner().clone();
    let service = ComfortService::new(
        Arc::new(ShellPort { app: app.clone() }),
        gateway,
        crate::sim::clock(),
        templates,
        prompt,
        Arc::new(banned),
    );
    tauri::async_runtime::spawn(service.run(bus));
}

/// 用户打开了小组件：熄灭工具栏小圆点（FR-CMF-04 第 4 条）。
pub fn widget_opened<R: Runtime, M: Manager<R>>(app: &M) {
    if let Some(s) = app.try_state::<Sensing>() {
        s.xqp.send(Down::Badge { on: false });
    }
}

struct ShellPort {
    app: AppHandle,
}

impl ShellPort {
    fn setting(&self, key: &str) -> Option<String> {
        let state = self.app.state::<AppState>();
        let v = settings::get(&state.db(), key).ok()?;
        v.as_str().map(str::to_string)
    }

    fn list(&self, key: &str) -> Vec<String> {
        let state = self.app.state::<AppState>();
        settings::get(&state.db(), key)
            .ok()
            .and_then(|v| v.as_list().map(<[String]>::to_vec))
            .unwrap_or_default()
    }
}

impl ComfortPort for ShellPort {
    fn level(&self) -> CareLevel {
        CareLevel::parse(self.setting("care.level").as_deref().unwrap_or(""))
    }

    fn style(&self) -> Style {
        Style::parse(self.setting("care.style").as_deref().unwrap_or(""))
    }

    fn llm_allowed(&self) -> bool {
        ConsentState::load(&self.app.state::<AppState>().db())
            .is_ok_and(|c| c.granted(ConsentItem::LlmSummary))
    }

    fn dnd(&self) -> bool {
        crate::fullscreen::foreground_fullscreen()
    }

    fn dnd_apps(&self) -> Vec<String> {
        self.list("care.dnd_apps")
    }

    fn quiet_hours(&self) -> Vec<String> {
        self.list("care.quiet_hours")
    }

    fn muted_until(&self) -> Option<i64> {
        let now = chrono::Utc::now().timestamp_millis();
        comfort_feedback::muted_until(&self.app.state::<AppState>().db(), now)
    }

    fn recent_texts(&self) -> Vec<String> {
        self.app
            .state::<AppState>()
            .db()
            .comfort_recent_texts(10)
            .unwrap_or_else(|e| {
                eprintln!("读取最近的暖心话失败：{e}");
                Vec::new()
            })
    }

    fn blocked_templates(&self) -> HashSet<String> {
        self.app
            .state::<AppState>()
            .db()
            .comfort_blocked_templates()
            .map(|v| v.into_iter().collect())
            .unwrap_or_default()
    }

    fn auto_since(&self, since_ms: i64) -> (u32, Option<i64>) {
        // 读不出来时按“今天已满”处理，宁可少说一句也不打扰
        self.app
            .state::<AppState>()
            .db()
            .comfort_auto_since(since_ms)
            .unwrap_or_else(|e| {
                eprintln!("读取暖心话次数失败：{e}");
                (u32::MAX, None)
            })
    }

    fn save(&self, rec: &ComfortRecord<'_>) -> Option<i64> {
        self.app
            .state::<AppState>()
            .writer()
            .write_sync(|db| db.comfort_insert(rec))
            .inspect_err(|e| eprintln!("写入暖心话记录失败：{e}"))
            .ok()
    }

    fn show(&self, c: &Comfort) {
        let ev = ComfortNew {
            id: u32::try_from(c.id).unwrap_or(0),
            text: c.text.clone(),
            source: c.source,
            ai_generated: c.source == ComfortSource::Llm,
        };
        if let Err(e) = ev.emit(&self.app) {
            eprintln!("推送 comfort:new 失败：{e}");
        }
        let widget_visible = self
            .app
            .get_webview_window(WindowTarget::Widget.label())
            .is_some_and(|w| w.is_visible().unwrap_or(false));
        if !widget_visible {
            self.app
                .state::<Sensing>()
                .xqp
                .send(Down::Badge { on: true });
        }
    }

    fn note(&self, msg: &str) {
        eprintln!("{msg}");
    }
}
