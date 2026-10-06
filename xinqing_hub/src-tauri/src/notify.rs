//! 系统通知（07 FR-NTF-01，16 D-09）：只在小组件看不见时代替小组件卡片，目前用于休息提醒。
//!
//! - 文案与按钮来自 `ui_copy.toml`（core 的 [`NotifyCopy`]），和小组件卡片一致；暖心话、情绪状态、求助卡片不走这里（DS-COPY-08）。
//! - Windows 用系统 toast（`tauri-winrt-notification`）：带“知道了”/“5 分钟后”两个按钮，点了经 [`RestCmd::Act`] 交回休息服务，
//!   和点卡片按钮一样记录、清零或顺延。专注助手开着时由系统按自己的规则收进通知中心（C-PLT-09），这里不另判断。
//! - 应用标识：发行版用 `tauri.conf.json` 的 `identifier`（安装包建开始菜单快捷方式时登记，FR-NTF-01 第 4 条）；
//!   调试构建没有安装，借用 PowerShell 的标识，通知来源会显示成 PowerShell。
//! - 非 Windows（Linux 上开发、跑测试）不弹，调用方退回小组件卡片。

use tauri::{AppHandle, Manager};
use xinqing_hub_core::domain::notify::{Notice, NotifyCopy};
use xinqing_hub_core::domain::rest::{RestAction, RestKind};
use xinqing_hub_core::infra::templates::TemplateDirs;
use xinqing_hub_core::rest::RestCmd;

use crate::paths;
use crate::rest::Rest;

/// 由 Tauri 托管。文案加载失败时为 `None`，此时不弹系统通知，休息提醒照常走小组件卡片。
pub struct Notifier(Option<NotifyCopy>);

pub fn start(app: &AppHandle) {
    let copy = paths::templates_dir()
        .ok_or_else(|| anyhow::anyhow!("找不到 hub_templates"))
        .and_then(|dir| Ok(NotifyCopy::load(&TemplateDirs::factory_only(dir))?));
    let copy = match copy {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("系统通知不可用：{e}");
            None
        }
    };
    app.manage(Notifier(copy));
}

impl Notifier {
    /// 弹休息提醒通知（FR-NTF-01）。返回是否弹出；没弹出时调用方改走小组件卡片，提醒不会丢。
    pub fn rest(&self, app: &AppHandle, kind: RestKind, tired: bool) -> bool {
        let Some(copy) = &self.0 else {
            return false;
        };
        let handle = app.clone();
        show(app, &copy.rest(kind, tired), move |arg| {
            // 点的是通知正文（不是按钮）时没有参数：什么也不做，提醒按“没有回应”处理
            let Some(action) = arg.as_deref().and_then(parse_action) else {
                return;
            };
            if let Some(rest) = handle.try_state::<Rest>()
                && rest.cmds.try_send(RestCmd::Act { kind, action }).is_err()
            {
                eprintln!("休息提醒服务没在运行，通知上的操作被忽略");
            }
        })
    }
}

/// 按钮参数 → 休息提醒的操作（与 `rest_action` 命令的取值相同）
fn parse_action(arg: &str) -> Option<RestAction> {
    serde_json::from_value(serde_json::Value::String(arg.to_owned())).ok()
}

#[cfg(windows)]
fn show(
    app: &AppHandle,
    notice: &Notice,
    mut on_action: impl FnMut(Option<String>) + Send + 'static,
) -> bool {
    use tauri_winrt_notification::{Duration, Toast};

    let app_id = if cfg!(debug_assertions) {
        Toast::POWERSHELL_APP_ID.to_owned()
    } else {
        app.config().identifier.clone()
    };
    let mut toast = Toast::new(&app_id)
        .title(&notice.title)
        .text1(&notice.body)
        .duration(Duration::Short);
    for b in &notice.buttons {
        toast = toast.add_button(&b.label, b.action);
    }
    match toast
        .on_activated(move |arg| {
            on_action(arg);
            Ok(())
        })
        .show()
    {
        Ok(()) => true,
        Err(e) => {
            eprintln!("弹系统通知失败：{e}");
            false
        }
    }
}

#[cfg(not(windows))]
fn show(
    _app: &AppHandle,
    _notice: &Notice,
    _on_action: impl FnMut(Option<String>) + Send + 'static,
) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use xinqing_hub_core::domain::notify::{ACTION_OK, ACTION_SNOOZE};

    #[test]
    fn button_args_map_to_rest_actions() {
        assert_eq!(parse_action(ACTION_OK), Some(RestAction::Done));
        assert_eq!(parse_action(ACTION_SNOOZE), Some(RestAction::Later));
        assert_eq!(parse_action(""), None);
        assert_eq!(parse_action("open"), None);
    }
}
