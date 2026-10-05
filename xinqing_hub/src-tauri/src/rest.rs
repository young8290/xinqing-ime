//! 启动休息提醒服务（B-08，03 第 2.2 节 `rest` 任务）并实现它用的 [`RestPort`]。
//!
//! 显示（FR-RST-06 第 4 条）：经 XQP `tip` 在光标旁冒一句短文案（核心在输入中不弹、自带限流，A-07），
//! 同时推送 `rest:due`，小组件显示完整卡片和 `已完成` / `5 分钟后` / `今天不再提醒`。
//! 小组件隐藏时改用系统通知（FR-NTF-01）还没做：系统通知随 D-09 接入，在此之前只有光标旁气泡。

use std::sync::Arc;

use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use tokio::sync::mpsc;
use xinqing_hub_core::domain::rest::{Due, KindCfg, RestAction, RestConfig, RestKind, parse_hhmm};
use xinqing_hub_core::domain::settings::{self, SettingValue};
use xinqing_hub_core::infra::clock::SystemClock;
use xinqing_hub_core::rest::{RestCmd, RestPort, RestService};
use xqp::Down;

use crate::events::RestDue;
use crate::sensing::Sensing;
use crate::state::AppState;

/// 光标旁气泡的显示时长（10 第 2.5 节：1500–2500 ms；FR-RST-06 第 4 条写的是 2.5 秒）。
const TIP_MS: u32 = 2_500;

/// 由 Tauri 托管，`rest_action` 命令经它把用户的操作交给服务。
pub struct Rest {
    pub cmds: mpsc::Sender<RestCmd>,
}

/// 须在 `AppState`、`Sensing` 都托管之后调用。
pub fn start(app: &AppHandle) {
    let bus = app.state::<Sensing>().bus.subscribe();
    let (cmds, cmd_rx) = mpsc::channel(16);
    app.manage(Rest { cmds });
    let service = RestService::new(
        Arc::new(ShellPort { app: app.clone() }),
        Arc::new(SystemClock),
    );
    tauri::async_runtime::spawn(service.run(bus, cmd_rx));
}

struct ShellPort {
    app: AppHandle,
}

impl ShellPort {
    fn setting(&self, key: &str) -> Option<SettingValue> {
        settings::get(&self.app.state::<AppState>().db(), key).ok()
    }

    fn flag(&self, key: &str) -> bool {
        self.setting(key).and_then(|v| v.as_bool()).unwrap_or(true)
    }

    fn minutes(&self, key: &str, fallback: u32) -> u32 {
        match self.setting(key) {
            Some(SettingValue::Number(x)) => x.round() as u32,
            _ => fallback,
        }
    }

    fn kind(&self, name: &str, fallback: KindCfg) -> KindCfg {
        KindCfg {
            enabled: self.flag(&format!("rest.{name}.enabled")),
            interval_min: self.minutes(&format!("rest.{name}.interval"), fallback.interval_min),
        }
    }
}

impl RestPort for ShellPort {
    fn config(&self) -> RestConfig {
        let d = RestConfig::default();
        RestConfig {
            eye: self.kind("eye", d.eye),
            water: self.kind("water", d.water),
            mv: self.kind("move", d.mv),
            night_enabled: self.flag("rest.night.enabled"),
            night_start_min: self
                .setting("rest.night.start")
                .and_then(|v| v.as_str().and_then(parse_hhmm))
                .unwrap_or(d.night_start_min),
        }
    }

    fn count_when_paused(&self) -> bool {
        self.flag("rest.count_when_paused")
    }

    fn linked(&self) -> bool {
        self.app.state::<AppState>().status().connected
    }

    fn dnd(&self) -> bool {
        crate::fullscreen::foreground_fullscreen()
    }

    fn system_idle_ms(&self) -> Option<u64> {
        system_idle_ms()
    }

    fn show(&self, due: &Due) {
        let tip = if due.tired {
            "眼睛也累了吧"
        } else {
            due.kind.tip()
        };
        self.app.state::<Sensing>().xqp.send(Down::Tip {
            text: tip.into(),
            ms: TIP_MS,
        });
        let ev = RestDue {
            kind: due.kind,
            tired: due.tired,
        };
        if let Err(e) = ev.emit(&self.app) {
            eprintln!("推送 rest:due 失败：{e}");
        }
    }

    fn log(&self, ts: i64, kind: RestKind, action: RestAction) {
        if let Err(e) =
            self.app
                .state::<AppState>()
                .db()
                .reminder_insert(ts, kind.as_str(), action.as_str())
        {
            eprintln!("写入休息提醒记录失败：{e}");
        }
    }

    fn note(&self, msg: &str) {
        eprintln!("{msg}");
    }
}

/// 距最近一次键鼠操作过了多少毫秒（`GetLastInputInfo`，只有时间，不含任何按键信息，04 FR-SEN-06 第 4 条）。
#[cfg(windows)]
fn system_idle_ms() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: 出参指向栈上已按要求填好 cbSize 的结构
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
        return None;
    }
    // SAFETY: 无参数
    let now = unsafe { GetTickCount() };
    // 两者都是开机以来的毫秒数（32 位，约 49.7 天回绕），用回绕减法
    Some(u64::from(now.wrapping_sub(info.dwTime)))
}

#[cfg(not(windows))]
fn system_idle_ms() -> Option<u64> {
    None
}
