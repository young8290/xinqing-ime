//! 启动情绪日记服务（C-10，FR-DIA-01～03）并实现它用的 [`DiaryPort`]：写库用写连接，同意状态读只读连接，
//! 日记里危机识别命中时与对话共用 [`crate::chat::show_safety`]（ADR 0031）。

use std::ops::Deref;
use std::sync::Arc;

use tauri::{AppHandle, Manager};
use xinqing_hub_core::diary::{DiaryPort, DiaryService};
use xinqing_hub_core::domain::consent::{ConsentItem, ConsentState};
use xinqing_hub_core::domain::diary::{DiaryCopy, DiaryPrompt};
use xinqing_hub_core::domain::safety::CrisisLexicon;
use xinqing_hub_core::domain::validate::BannedWords;
use xinqing_hub_core::infra::store::Db;
use xinqing_hub_core::infra::templates::TemplateDirs;

use crate::gateway::Ai;
use crate::paths;
use crate::state::AppState;

/// 由 Tauri 托管。模板加载失败时为 `None`，日记命令返回通用错误，其余功能不受影响。
pub struct Diary(pub Option<Arc<DiaryService>>);

/// 须在 `AppState`、`Arc<Ai>`、`Sensing` 都托管之后调用。
pub fn start(app: &AppHandle) {
    let loaded = paths::templates_dir()
        .ok_or_else(|| anyhow::anyhow!("找不到 hub_templates"))
        .and_then(|dir| {
            let dirs = TemplateDirs::factory_only(dir);
            Ok((
                DiaryPrompt::load(&dirs)?,
                DiaryCopy::load(&dirs)?,
                CrisisLexicon::load(&dirs)?,
                BannedWords::load(&dirs)?,
            ))
        });
    let service = match loaded {
        Ok((prompt, copy, lex, banned)) => Some(Arc::new(DiaryService::new(
            Arc::new(ShellPort { app: app.clone() }),
            app.state::<Arc<Ai>>().inner().clone(),
            crate::sim::clock(),
            prompt,
            copy,
            Arc::new(lex),
            Arc::new(banned),
        ))),
        Err(e) => {
            eprintln!("情绪日记不可用：{e}");
            None
        }
    };
    app.manage(Diary(service));
}

struct ShellPort {
    app: AppHandle,
}

impl ShellPort {
    fn granted(&self, item: ConsentItem) -> bool {
        ConsentState::load(&self.app.state::<AppState>().db()).is_ok_and(|c| c.granted(item))
    }
}

impl DiaryPort for ShellPort {
    fn db(&self) -> Box<dyn Deref<Target = Db> + '_> {
        Box::new(self.app.state::<AppState>().inner().writer().lock())
    }

    fn summary_allowed(&self) -> bool {
        self.granted(ConsentItem::LlmSummary)
    }

    fn jev_allowed(&self) -> bool {
        self.granted(ConsentItem::JevFeatures)
    }

    fn safety(&self, session_id: i64) {
        crate::chat::show_safety(&self.app, session_id);
    }

    fn note(&self, msg: &str) {
        eprintln!("{msg}");
    }
}
