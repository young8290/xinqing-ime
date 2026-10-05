//! 输入法配置变更（07 FR-SET-02、10 第 4 节 `config.changed`、ADR 0017）：常驻一个线程连着 wind-rpc 的
//! 事件通道，把核心的 `config.changed` 转成 `ime_config:changed` 推给前端。
//!
//! 读是阻塞的，用专用线程而不是 async 任务。核心没运行或重启时按 [`Backoff`] 重连（1 秒起，最多 30 秒）；
//! 每次连上先推一条 `connected`，断开期间漏掉的变更由界面重新取配置补上。

use tauri::AppHandle;
use tauri_specta::Event;
use xinqing_hub_core::infra::imeconf::{Backoff, ImeConfigChange, ImeEvents};

use crate::events::ImeConfigChanged;

pub fn start(app: &AppHandle) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("ime-events".into())
        .spawn(move || run(&app));
    if let Err(e) = spawned {
        eprintln!("启动输入法事件线程失败：{e}");
    }
}

fn run(app: &AppHandle) -> ! {
    // 开发版 Hub 连开发版核心，与 `commands::ime` 的控制通道同一规则
    let endpoint = ImeEvents::endpoint_from_env(cfg!(debug_assertions));
    let mut backoff = Backoff::default();
    loop {
        // 连不上就是核心没运行，静默重试，不刷日志
        if let Ok(mut events) = ImeEvents::connect(&endpoint) {
            backoff.reset();
            emit(app, ImeConfigChange::connected());
            loop {
                match events.next_config_change() {
                    Ok(change) => emit(app, change),
                    Err(e) => {
                        eprintln!("输入法事件通道断开，稍后重连：{e}");
                        break;
                    }
                }
            }
        }
        std::thread::sleep(backoff.next_delay());
    }
}

fn emit(app: &AppHandle, change: ImeConfigChange) {
    if let Err(e) = ImeConfigChanged(change).emit(app) {
        eprintln!("推送 ime_config:changed 失败：{e}");
    }
}
