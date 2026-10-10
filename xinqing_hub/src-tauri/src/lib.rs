//! 心晴 Hub 的 Tauri 外壳（17 第 2 节的接口层）：组装基础层与领域服务、注册命令与事件、管理窗口。
//!
//! 领域逻辑都在 `xinqing-hub-core`，这里只做参数校验 → 调用领域服务 → 转成 `UiError`（ADR 0007）。

mod args;
mod chat;
mod cleanup;
mod comfort;
mod commands;
mod diary;
mod error;
mod evening;
mod events;
mod fullscreen;
mod gateway;
mod hotkeys;
mod ime_events;
mod letter;
mod notify;
mod paths;
mod pulse;
mod research;
mod rest;
mod rewrite;
mod schedule;
mod secrets;
mod sensing;
mod sim;
mod state;
mod windows;
#[cfg(windows)]
mod xqp_pipe;

use std::path::Path;

use specta_typescript::Typescript;
use tauri::Manager;
use tauri_specta::{Builder, collect_commands, collect_events};
use xinqing_hub_core::domain::consent::{self, ConsentState};

pub use args::LaunchArgs;
pub use error::UiError;
pub use windows::WindowTarget;

const BINDINGS_HEADER: &str =
    "// 本文件由 `cargo run -p xinqing-hub --bin export-bindings` 生成，请勿手改。";

/// 命令与事件的唯一登记处：`invoke_handler` 与 TypeScript 绑定都从这里生成，两边不会走样。
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::get_status,
            commands::data_export,
            commands::state_explain,
            commands::submit_feedback,
            commands::self_report_set,
            commands::self_report_list,
            commands::baseline_reset,
            commands::get_routine,
            commands::demo_status,
            commands::research_dismiss,
            commands::research_clear,
            commands::pause_set,
            commands::settings_schema,
            commands::settings_get,
            commands::settings_set,
            commands::consent_get,
            commands::consent_set,
            commands::open_window,
            commands::hotkey_status,
            commands::rest_action,
            commands::ai::ai_config_get,
            commands::ai::secrets_set,
            commands::ai::ai_test_connection,
            commands::ai::ai_usage_today,
            commands::ai::ai_gateway_metrics,
            commands::ai::ai_net_log_recent,
            commands::comfort::comfort_feedback,
            commands::dashboard::comfort_list,
            commands::dashboard::day_stats,
            commands::dashboard::month_moods,
            commands::dashboard::week_stats,
            commands::letter::letters_list,
            commands::letter::letter_read,
            commands::letter::letter_delete,
            commands::chat::chat_list_sessions,
            commands::chat::chat_get_messages,
            commands::chat::chat_set_mode,
            commands::chat::chat_search,
            commands::chat::memory_list,
            commands::chat::memory_add,
            commands::chat::memory_update,
            commands::chat::memory_delete,
            commands::chat::chat_send,
            commands::chat::chat_retry,
            commands::chat::chat_stop,
            commands::chat::chat_copy,
            commands::chat::chat_delete,
            commands::chat::chat_delete_all,
            commands::chat::safety_dismiss,
            commands::chat::safety_open,
            commands::diary::diary_generate,
            commands::diary::diary_save,
            commands::diary::diary_list,
            commands::diary::diary_delete,
            commands::schedule::schedule_list,
            commands::schedule::schedule_confirm,
            commands::schedule::schedule_ignore,
            commands::schedule::schedule_create,
            commands::schedule::schedule_update,
            commands::schedule::schedule_delete,
            commands::schedule::schedule_export_ics,
            commands::schedule::reminder_action,
            commands::schedule::todo_list,
            commands::schedule::todo_confirm,
            commands::schedule::todo_ignore,
            commands::schedule::todo_create,
            commands::schedule::todo_update,
            commands::schedule::todo_complete,
            commands::schedule::todo_delete,
            commands::ime::ime_schema,
            commands::ime::ime_config_get,
            commands::ime::ime_config_set,
        ])
        .events(collect_events![
            events::StatusChanged,
            events::SettingsChanged,
            events::SelfReportChanged,
            events::GatewayHealthChanged,
            events::ComfortNew,
            events::CareReduced,
            events::RestDue,
            events::ResearchInvite,
            events::ImeConfigChanged,
            events::ChatDelta,
            events::ChatDone,
            events::ChatErrorEvent,
            events::SafetyTriggered,
            events::SafetyInvite,
            events::ReviewEvening,
            events::LetterNew,
            events::ScheduleDetected,
            events::ScheduleReady,
            events::TodoDetected,
            events::TodoReady,
            events::ReminderDue,
            events::ReminderMissed,
            events::MoodTypo,
            events::TypingPulse,
        ])
}

pub fn export_bindings(path: &Path) -> anyhow::Result<()> {
    specta_builder().export(Typescript::default().header(BINDINGS_HEADER), path)?;
    Ok(())
}

pub fn run() {
    let launch = LaunchArgs::parse(std::env::args().skip(1));
    // 演示模式要在任何服务取时钟之前定下来：`--demo` 或真实库里的 `dev.demo`（ADR 0023、0025）
    let demo_key = paths::hub_data_dir().is_ok_and(|d| sim::demo_setting(&d));
    sim::set_demo(launch.demo || demo_key);
    let builder = specta_builder();

    tauri::Builder::default()
        // 单实例（17 第 2.2 节第 1 步）：第二个实例把参数转交给已运行的实例后退出
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let again = LaunchArgs::parse(argv.into_iter().skip(1));
            let target = again.open.unwrap_or(WindowTarget::Widget);
            if let Err(e) = windows::open(app, target) {
                eprintln!("打开窗口失败：{e}");
            }
        }))
        // 系统“另存为”对话框：设置页“导出我的数据”选保存位置（FR-DAT-03）；权限只给设置窗口，见 capabilities/settings.json
        .plugin(tauri_plugin_dialog::init())
        // 全局快捷键 Ctrl+Alt+Q / W / D（FR-ENT-04）：组合键从设置读，setup 里注册（ADR 0036）
        .plugin(hotkeys::plugin())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let data_dir = paths::hub_data_dir()?;
            let state = if sim::demo() {
                sim::open_demo_state(&data_dir)?
            } else {
                state::AppState::init(&data_dir)?
            };
            let needs_onboarding = state.needs_onboarding()?;
            let cfg = consent::xqp_cfg(&ConsentState::load(&state.db())?);
            app.manage(state);
            // 第 4 步：AI 网关（无配置时为离线模式）
            app.manage(gateway::start(app.handle(), &data_dir));
            // 第 5 步：XQP 客户端与实时感知。Hub 是同意状态的唯一真相源，握手后立即下发 cfg
            let sensing = sensing::start(app.handle(), cfg);
            app.manage(sensing);
            cleanup::start(app.handle());
            // 暖心话：订阅总线，主动关怀与自评回应（C-04）
            comfort::start(app.handle());
            // 休息提醒：使用时长计时与四类提醒（B-08）
            notify::start(app.handle());
            rest::start(app.handle());
            // 打错字“晃一下”、打字心电图（DS-MOTION-02、FR-DSH-02，ADR 0036）
            pulse::start(app.handle());
            // 研究模式：定时自评邀请（FR-DMO-04）
            research::start(app.handle());
            // 对话与对话里的危机安全（C-07、C-08）
            chat::start(app.handle());
            diary::start(app.handle());
            rewrite::start(app.handle());
            evening::start(app.handle());
            letter::start(app.handle());
            // 日程与待办识别、提醒调度（C-05、C-06）
            schedule::start(app.handle());

            // 第 6 步：首次运行或隐私说明升级 → 引导窗口；否则显示小组件（`widget.visible` 关闭时不显示）。
            // 核心以 `--background` 拉起时同样走这一步（03 第 3.1 节），区别只是不额外打开其他窗口。
            if needs_onboarding {
                windows::open(app.handle(), WindowTarget::Onboarding)?;
            } else if app.state::<state::AppState>().widget_visible()? {
                windows::open(app.handle(), WindowTarget::Widget)?;
            }
            if let Some(target) = launch.open {
                windows::open(app.handle(), target)?;
            }
            app.manage(hotkeys::Hotkeys::default());
            hotkeys::apply(app.handle());
            // 前台全屏时自动隐藏小组件（FR-WGT-01）
            fullscreen::start(app.handle());
            // 输入法配置变更：设置中心跟着刷新（ADR 0017）
            ime_events::start(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("心晴 Hub 启动失败")
        .run(|app, event| match event {
            // Hub 是常驻后台进程（03 第 3.1 节）：关掉最后一个窗口不退出，只有显式退出才结束
            tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } => api.prevent_exit(),
            // 托管状态在退出时不一定会被析构，这里把排队中的写入提交掉（ADR 0020）
            tauri::RunEvent::Exit => {
                if let Some(state) = app.try_state::<state::AppState>() {
                    state.writer().flush();
                }
            }
            _ => {}
        });
}
