//! 每天 04:30 的数据清理（FR-DAT-02）。规则在 `xinqing_hub_core::domain::retention`，这里只管什么时候跑。
//!
//! Hub 启动 5 分钟后先补跑一次（docs/adr/0013 第 1 条）：电脑在 04:30 常常是关着的，只按时刻跑会一直清不到。
//! 等待按最多 10 分钟一段来睡，睡眠唤醒后最迟 10 分钟内发现已经过了时刻。

use std::time::Duration;

use chrono::Local;
use tauri::{AppHandle, Manager};
use xinqing_hub_core::domain::retention::{self, Policy};

use crate::state::AppState;

const STARTUP_DELAY: Duration = Duration::from_secs(5 * 60);
const MAX_NAP: Duration = Duration::from_secs(10 * 60);

/// 须在 `AppState` 托管之后调用。
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            let handle = app.clone();
            if let Err(e) = tauri::async_runtime::spawn_blocking(move || run_once(&handle)).await {
                eprintln!("数据清理任务异常退出：{e}");
            }
            let due = retention::next_run(Local::now());
            while Local::now() < due {
                let left = (due - Local::now()).to_std().unwrap_or_default();
                tokio::time::sleep(left.min(MAX_NAP)).await;
            }
        }
    });
}

fn run_once(app: &AppHandle) {
    // 对话保留期的设置键 `chat.retention_days`（FR-CHT-08）登记后从设置读，现在用默认 90 天
    let policy = Policy::default();
    let report = app
        .state::<AppState>()
        .writer()
        .write_sync(|db| retention::run(db, &policy, Local::now()));
    // 报告只有表名、条数和错误信息，不含用户内容（NFR-LOG-01）
    eprintln!("{}", report.summary());
}
