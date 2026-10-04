//! AI 网关的外壳接线：持久化出网日志、同步健康状态并刷新模型列表。

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tokio::sync::mpsc;
use xinqing_hub_core::domain::status::StatusSnapshot;
use xinqing_hub_core::infra::clock::SystemClock;
use xinqing_hub_gateway::{GatewayConfig, HttpGateway};

use crate::commands::emit_status;
use crate::state::AppState;

const MODEL_REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);

pub fn start(app: &AppHandle) -> Arc<HttpGateway> {
    let (log_tx, mut log_rx) = mpsc::unbounded_channel();
    let config = if cfg!(debug_assertions) {
        GatewayConfig::dev_from_env()
    } else {
        GatewayConfig::default()
    };
    let gateway = match HttpGateway::new(config, Arc::new(SystemClock)) {
        Ok(gateway) => Arc::new(gateway.with_net_log(log_tx.clone())),
        Err(err) => {
            eprintln!("AI 网关配置无效，使用离线模式：{err}");
            Arc::new(
                HttpGateway::new(GatewayConfig::default(), Arc::new(SystemClock))
                    .expect("空配置的离线网关必须可创建")
                    .with_net_log(log_tx),
            )
        }
    };

    let log_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(entry) = log_rx.recv().await {
            if let Err(err) = log_app.state::<AppState>().db().net_log_insert(&entry) {
                eprintln!("保存 AI 出网日志失败：{err}");
            }
        }
    });

    update_offline(app, gateway.health().offline());

    let mut health = gateway.subscribe_health();
    let health_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while health.changed().await.is_ok() {
            update_offline(&health_app, health.borrow_and_update().offline());
        }
    });

    let refresh_gateway = gateway.clone();
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(MODEL_REFRESH_INTERVAL);
        loop {
            interval.tick().await;
            if let Err(err) = refresh_gateway.refresh_models().await {
                eprintln!("刷新 AI 模型列表失败：{err}");
            }
        }
    });

    gateway
}

fn update_offline(app: &AppHandle, offline: bool) {
    let (snapshot, changed) = app
        .state::<AppState>()
        .update_status(|status| StatusSnapshot::set(&mut status.offline, offline));
    if changed {
        emit_status(app, snapshot);
    }
}
