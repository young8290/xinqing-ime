//! 打错字与打字心电图的推送（DS-MOTION-02“晃一下”、FR-DSH-02 打字心电图，ADR 0036）。
//!
//! 订阅总线：`HubEvent::Typo` 转成 `mood:typo`（限频）；XQP `key` 经 [`PulseTracker`] 只留键间间隔，
//! 每 100 毫秒批量推一次 `typing:pulse`，且只在看板窗口看得见时推（DS-MOTION-05、NFR-RES-02）。

use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use tokio::sync::broadcast::error::RecvError;
use xinqing_hub_core::bus::HubEvent;
use xinqing_hub_core::domain::pulse::PulseTracker;

use crate::events::{MoodTypo, TypingPulse};
use crate::sensing::Sensing;
use crate::windows::WindowTarget;

/// 两次“晃一下”至少隔这么久：动画 0.5 秒，连着打错也不会闪个不停（DS-MOTION-04）。
pub const TYPO_MIN_GAP_MS: i64 = 1_500;
/// 心电图刷新间隔：不超过 10 Hz（FR-DSH-02）。
const PULSE_TICK: Duration = Duration::from_millis(100);

/// 须在 `Sensing` 托管之后调用。
pub fn start(app: &AppHandle) {
    let mut rx = app.state::<Sensing>().bus.subscribe();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tracker = PulseTracker::default();
        let mut batch = Vec::new();
        let mut last_typo = i64::MIN;
        let mut tick = tokio::time::interval(PULSE_TICK);
        loop {
            tokio::select! {
                ev = rx.recv() => match ev {
                    Ok(HubEvent::Xqp(up)) => {
                        if let Some(p) = tracker.on_up(&up, crate::sim::clock().now_ms()) {
                            batch.push(p);
                        }
                    }
                    Ok(HubEvent::Typo { .. }) => {
                        let now = crate::sim::clock().now_ms();
                        if now - last_typo >= TYPO_MIN_GAP_MS {
                            last_typo = now;
                            if let Err(e) = (MoodTypo {}).emit(&app) {
                                eprintln!("推送 mood:typo 失败：{e}");
                            }
                        }
                    }
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                },
                _ = tick.tick() => {
                    if batch.is_empty() {
                        continue;
                    }
                    let keys = std::mem::take(&mut batch);
                    if dashboard_visible(&app)
                        && let Err(e) = (TypingPulse { keys }).emit(&app)
                    {
                        eprintln!("推送 typing:pulse 失败：{e}");
                    }
                }
            }
        }
    });
}

fn dashboard_visible(app: &AppHandle) -> bool {
    app.get_webview_window(WindowTarget::Dashboard.label())
        .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false))
}
