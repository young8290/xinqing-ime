//! 启动日程与待办识别服务、提醒调度（C-05、C-06），实现它们用的 [`SchedulePort`]、[`ReminderPort`]（ADR 0032）。
//!
//! - 识别：L2 通过时经 XQP `tip` 在光标旁冒“📅 识别到日程”，推 `schedule:detected` / `todo:detected`，抽取完推
//!   `schedule:ready` / `todo:ready`；卡片层（D-04）据此显示“识别中…”与字段，按钮调 `schedule_*` / `todo_*` 命令。
//! - 提醒：到点推 `reminder:due`，并弹系统通知（`notify.rs`，非 Windows 不弹）；启动时 12 小时内错过的推一次
//!   `reminder:missed`。通知与卡片上的按钮都经 `reminder_action` 交回。

use std::ops::Deref;
use std::sync::Arc;

use serde::Deserialize;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use tokio::sync::mpsc;
use xinqing_hub_core::domain::consent::{ConsentItem, ConsentState};
use xinqing_hub_core::domain::notify::Notice;
use xinqing_hub_core::domain::reminder::{self, ReminderCopy};
use xinqing_hub_core::domain::schedule::{CandidateKind, ExtractPrompts, ScheduleRecognizer};
use xinqing_hub_core::domain::settings::{self, SettingValue};
use xinqing_hub_core::domain::validate::BannedWords;
use xinqing_hub_core::infra::store::Db;
use xinqing_hub_core::infra::templates::{TemplateDirs, read_toml};
use xinqing_hub_core::reminder::{Reminder, ReminderCmd, ReminderPort, ReminderService};
use xinqing_hub_core::schedule::{Ready, SchedulePort, ScheduleService};
use xqp::Down;

use crate::events::{
    ReminderDue, ReminderMissed, ScheduleDetected, ScheduleReady, TodoDetected, TodoReady,
};
use crate::gateway::Ai;
use crate::notify::Notifier;
use crate::paths;
use crate::sensing::Sensing;
use crate::state::AppState;

/// 光标旁气泡 1.5 秒（FR-SCH-05 第 1 条）。
const TIP_MS: u32 = 1_500;

/// 由 Tauri 托管，`reminder_action` 经它把操作交给提醒服务。
pub struct Reminders {
    pub cmds: mpsc::Sender<ReminderCmd>,
}

/// 日程命令用到的固定文案（`ui_copy.toml` 的 `[schedule]`）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScheduleCopy {
    pub ics_description: String,
}

#[derive(Deserialize)]
struct RawCopy {
    version: u32,
    schedule: ScheduleCopy,
}

impl ScheduleCopy {
    fn load(dirs: &TemplateDirs) -> anyhow::Result<Self> {
        let banned = BannedWords::load(dirs)?;
        Ok(dirs.load_with_fallback("ui_copy.toml", |path| {
            let raw: RawCopy = read_toml(path)?;
            if raw.version == 0
                || raw.schedule.ics_description.trim().is_empty()
                || banned
                    .find(
                        &raw.schedule.ics_description,
                        xinqing_hub_core::domain::validate::Scene::Other,
                    )
                    .is_some()
            {
                return Err(xinqing_hub_core::infra::templates::TemplateError::Invalid {
                    file: "ui_copy.toml",
                    reason: "日程导出说明版本或正文校验失败",
                });
            }
            Ok(raw.schedule)
        })?)
    }
}

/// 须在 `AppState`、`Arc<Ai>`、`Sensing`、`Notifier` 都托管之后调用。模板加载失败时识别不启动（记日志），
/// 提醒照常（文案也读不出时提醒也不启动）。
pub fn start(app: &AppHandle) {
    let dirs = paths::templates_dir().map(TemplateDirs::factory_only);
    let copy_dirs = paths::hub_template_dirs().ok().or_else(|| dirs.clone());
    let copy = copy_dirs
        .as_ref()
        .and_then(|d| ScheduleCopy::load(d).ok())
        .unwrap_or_default();
    app.manage(copy);

    let (cmds, cmd_rx) = mpsc::channel(16);
    app.manage(Reminders { cmds });
    let reminder_copy = paths::hub_template_dirs()
        .or_else(|error| dirs.clone().ok_or(error))
        .and_then(|d| Ok(ReminderCopy::load(&d)?));
    match reminder_copy {
        Ok(copy) => {
            let service = ReminderService::new(
                Arc::new(ShellPort { app: app.clone() }),
                crate::sim::clock(),
                copy,
            );
            tauri::async_runtime::spawn(service.run(cmd_rx));
        }
        Err(e) => eprintln!("日程提醒不可用：{e}"),
    }

    let loaded = paths::hub_template_dirs().and_then(|d| {
        Ok((
            ScheduleRecognizer::load(&d)?,
            ExtractPrompts::load(&d)?,
            BannedWords::load(&d)?,
        ))
    });
    match loaded {
        Ok((recognizer, prompts, banned)) => {
            let service = Arc::new(ScheduleService::new(
                Arc::new(ShellPort { app: app.clone() }),
                app.state::<Arc<Ai>>().inner().clone(),
                crate::sim::clock(),
                recognizer,
                prompts,
                Arc::new(banned),
            ));
            let bus = app.state::<Sensing>().bus.subscribe();
            tauri::async_runtime::spawn(service.run(bus));
        }
        Err(e) => eprintln!("日程识别不可用：{e}"),
    }
}

struct ShellPort {
    app: AppHandle,
}

impl ShellPort {
    fn setting(&self, key: &str) -> Option<SettingValue> {
        settings::get(&self.app.state::<AppState>().db(), key).ok()
    }
}

impl SchedulePort for ShellPort {
    fn allowed(&self) -> bool {
        ConsentState::load(&self.app.state::<AppState>().db())
            .is_ok_and(|c| c.granted(ConsentItem::Schedule))
    }

    fn daily_cap(&self) -> usize {
        match self.setting("ai.cap.schedule") {
            Some(SettingValue::Number(n)) if n >= 0.0 => n as usize,
            _ => usize::MAX,
        }
    }

    fn default_offsets(&self) -> Vec<i64> {
        self.setting("sch.default_offsets")
            .and_then(|v| v.as_str().map(reminder::offsets_of_setting))
            .unwrap_or_else(|| reminder::DEFAULT_OFFSETS.to_vec())
    }

    fn db(&self) -> Box<dyn Deref<Target = Db> + '_> {
        Box::new(self.app.state::<AppState>().inner().writer().lock())
    }

    fn tip(&self, kind: CandidateKind) {
        self.app.state::<Sensing>().xqp.send(Down::Tip {
            text: kind.tip().into(),
            ms: TIP_MS,
        });
    }

    fn detected(&self, card_id: u32, kind: CandidateKind) {
        let r = match kind {
            CandidateKind::Schedule => ScheduleDetected { card_id }.emit(&self.app),
            CandidateKind::Todo => TodoDetected { card_id }.emit(&self.app),
        };
        if let Err(e) = r {
            eprintln!("推送识别卡片失败：{e}");
        }
    }

    fn ready(&self, r: &Ready) {
        let res = match r.clone() {
            Ready::Schedule {
                card_id,
                row,
                conflicts,
            } => ScheduleReady {
                card_id,
                schedule: row.map(Into::into),
                conflicts: conflicts.into_iter().map(Into::into).collect(),
            }
            .emit(&self.app),
            Ready::Todo { card_id, row } => TodoReady {
                card_id,
                todo: row.map(Into::into),
            }
            .emit(&self.app),
        };
        if let Err(e) = res {
            eprintln!("推送识别结果失败：{e}");
        }
    }

    fn note(&self, msg: &str) {
        eprintln!("{msg}");
    }
}

fn due_of(r: &Reminder) -> ReminderDue {
    ReminderDue {
        id: r.id,
        kind: r.kind.as_str().into(),
        ref_id: r.ref_id as u32,
        title: r.notice.title.clone(),
        body: r.notice.body.clone(),
    }
}

impl ReminderPort for ShellPort {
    fn db(&self) -> Box<dyn Deref<Target = Db> + '_> {
        Box::new(self.app.state::<AppState>().inner().db())
    }

    fn checked(&self) -> Option<i64> {
        self.app
            .state::<AppState>()
            .db()
            .settings_get(reminder::CHECKED_KEY)
            .ok()??
            .parse()
            .ok()
    }

    fn set_checked(&self, ms: i64) {
        let v = ms.to_string();
        self.app
            .state::<AppState>()
            .writer()
            .enqueue("sch.reminded_until", move |db| {
                db.settings_set(reminder::CHECKED_KEY, &v)
            });
    }

    fn digest_enabled(&self) -> bool {
        self.setting("todo.daily_digest")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn remind(&self, r: &Reminder, toast: bool) {
        if let Err(e) = due_of(r).emit(&self.app) {
            eprintln!("推送 reminder:due 失败：{e}");
        }
        if toast {
            self.app
                .state::<Notifier>()
                .reminder(&self.app, &r.notice, Some(r.id));
        }
    }

    fn missed(&self, notice: &Notice, items: &[Reminder]) {
        let ev = ReminderMissed {
            title: notice.title.clone(),
            body: notice.body.clone(),
            items: items.iter().map(due_of).collect(),
        };
        if let Err(e) = ev.emit(&self.app) {
            eprintln!("推送 reminder:missed 失败：{e}");
        }
        self.app
            .state::<Notifier>()
            .reminder(&self.app, notice, None);
    }

    fn note(&self, msg: &str) {
        eprintln!("{msg}");
    }
}
