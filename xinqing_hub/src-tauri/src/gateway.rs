//! AI 网关的外壳接线（C-03，17 第 2.2 节第 4 步、第 2.5 节）：按配置建网关、出网日志写入 `net_log`、
//! 健康状态同步到状态快照并推送 `gateway:health`、启动时和之后每 30 分钟刷新模型列表（FR-AIG-04）。
//!
//! 配置来源按优先级（ADR 0012）：dev 构建的明文 `secrets.toml` → `secrets.bin`（DPAPI）→ dev 构建连本机
//! mock-ai → 都没有则离线（两侧都走降级，03 第 8 节）。设置页保存后用新配置原地换掉网关，
//! 领域服务拿的是 [`Ai`]（它本身实现 `AiGateway`），不用重新取。

use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use tokio::sync::{mpsc, watch};
use xinqing_hub_core::domain::settings;
use xinqing_hub_core::domain::status::StatusSnapshot;
use xinqing_hub_core::infra::gateway::{
    AiError, AiGateway, CompleteRequest, CompleteResponse, Delta, GatewayHealth, JudgeRequest,
    JudgeResponse, NetLogEntry,
};
use xinqing_hub_core::infra::store::Db;
use xinqing_hub_gateway::{AiSecrets, GatewayConfig, HttpGateway, MaskedSecrets};

use crate::commands::emit_status;
use crate::events::GatewayHealthChanged;
use crate::secrets::{self, SecretStore, SecretsStoreError};
use crate::state::AppState;

const MODEL_REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// 当前配置从哪里来，设置页据此提示（例如 dev 构建正连着 mock-ai）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// 设置页保存的 `secrets.bin`
    Saved,
    /// dev 构建的 `secrets.toml`
    DevFile,
    /// dev 构建默认连本机 mock-ai
    DevMock,
    /// 没有配置，离线
    None,
}

/// 由 Tauri 托管（`Arc<Ai>`）。实现 `AiGateway`，每次调用转给当前网关。
pub struct Ai {
    current: RwLock<Current>,
    store: SecretStore,
    log_tx: mpsc::UnboundedSender<NetLogEntry>,
    /// 每换一次网关加 1，后台任务据此改订阅新网关的健康状态。
    swapped: watch::Sender<u64>,
}

struct Current {
    gateway: Arc<HttpGateway>,
    source: Source,
    masked: MaskedSecrets,
}

impl Ai {
    fn new(store: SecretStore, log_tx: mpsc::UnboundedSender<NetLogEntry>) -> Self {
        let (secrets, source) = choose(&store);
        let current = Current {
            gateway: Arc::new(build(&secrets, &log_tx, None)),
            source,
            masked: secrets.masked(),
        };
        Self {
            current: RwLock::new(current),
            store,
            log_tx,
            swapped: watch::Sender::new(0),
        }
    }

    pub fn gateway(&self) -> Arc<HttpGateway> {
        self.read().gateway.clone()
    }

    /// 设置 `ai.cap.*` 的每日上限交给当前网关（FR-AIG-07，ADR 0019 第 4 条）。换网关时上限随预算一起接过去。
    pub fn apply_caps(&self, db: &Db) {
        let gw = self.gateway();
        for (kind, cap) in settings::caps(db) {
            gw.set_cap(kind, cap);
        }
    }

    /// 设置页展示用：来源、地址、模型和密钥末 4 位。
    pub fn view(&self) -> (Source, MaskedSecrets) {
        let c = self.read();
        (c.source, c.masked.clone())
    }

    /// 设置页保存（`secrets_set`）：没有重新填写的密钥沿用已保存的那份，两侧都清空时删除文件；
    /// 保存成功后按优先级重新选配置并换掉网关。
    pub fn save(&self, new: AiSecrets) -> Result<(), SecretsStoreError> {
        new.validate()?;
        let saved = self.store.load().ok().flatten().unwrap_or_default();
        let merged = new.merged_onto(&saved);
        if merged.is_empty() {
            self.store.clear()?;
        } else {
            self.store.save(&merged)?;
        }
        self.reload();
        Ok(())
    }

    fn reload(&self) {
        let (secrets, source) = choose(&self.store);
        {
            let mut c = self.current.write().unwrap_or_else(|e| e.into_inner());
            let gateway = build(&secrets, &self.log_tx, Some(&c.gateway));
            *c = Current {
                gateway: Arc::new(gateway),
                source,
                masked: secrets.masked(),
            };
        }
        self.swapped.send_modify(|n| *n += 1);
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Current> {
        self.current.read().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl AiGateway for Ai {
    async fn judge(&self, req: JudgeRequest) -> Result<JudgeResponse, AiError> {
        self.gateway().judge(req).await
    }

    async fn complete(&self, req: CompleteRequest) -> Result<CompleteResponse, AiError> {
        self.gateway().complete(req).await
    }

    async fn stream(
        &self,
        req: CompleteRequest,
    ) -> Result<BoxStream<'static, Result<Delta, AiError>>, AiError> {
        self.gateway().stream(req).await
    }

    fn health(&self) -> GatewayHealth {
        self.gateway().health()
    }
}

/// 按优先级选配置。读不出来的来源记日志（不含内容）后跳过，不删除文件。
fn choose(store: &SecretStore) -> (AiSecrets, Source) {
    if let Some((path, r)) = secrets::load_dev_file() {
        match r {
            Ok(s) => return (s, Source::DevFile),
            Err(e) => eprintln!("{} 无法使用：{e}", path.display()),
        }
    }
    match store.load() {
        Ok(Some(s)) => return (s, Source::Saved),
        Ok(None) => {}
        Err(e) => eprintln!("AI 配置无法读取，先用离线模式：{e}"),
    }
    if cfg!(debug_assertions) {
        (AiSecrets::dev_from_env(), Source::DevMock)
    } else {
        (AiSecrets::default(), Source::None)
    }
}

fn build(
    secrets: &AiSecrets,
    log_tx: &mpsc::UnboundedSender<NetLogEntry>,
    old: Option<&HttpGateway>,
) -> HttpGateway {
    let gw = HttpGateway::new(secrets.to_config(), crate::sim::clock()).unwrap_or_else(|e| {
        eprintln!("AI 网关配置无效，使用离线模式：{e}");
        HttpGateway::new(GatewayConfig::default(), crate::sim::clock())
            .expect("空配置的离线网关必须可创建")
    });
    let gw = match old {
        Some(old) => gw.inherit_budget(old),
        None => gw,
    };
    gw.with_net_log(log_tx.clone())
}

/// 须在 `AppState` 托管之后调用。
pub fn start(app: &AppHandle, data_dir: &std::path::Path) -> Arc<Ai> {
    let (log_tx, mut log_rx) = mpsc::unbounded_channel();
    let ai = Arc::new(Ai::new(SecretStore::new(data_dir), log_tx));
    ai.apply_caps(&app.state::<AppState>().db());

    let log_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(entry) = log_rx.recv().await {
            if let Err(err) = log_app.state::<AppState>().db().net_log_insert(&entry) {
                eprintln!("保存 AI 出网日志失败：{err}");
            }
        }
    });

    tauri::async_runtime::spawn(supervise(app.clone(), ai.clone()));
    ai
}

/// 跟随当前网关：推送健康变化，定时刷新模型列表；网关被换掉后改跟新的。
async fn supervise(app: AppHandle, ai: Arc<Ai>) {
    let mut swapped = ai.swapped.subscribe();
    let mut last: Option<GatewayHealth> = None;
    loop {
        swapped.borrow_and_update();
        let gw = ai.gateway();
        let mut health = gw.subscribe_health();
        publish(&app, &mut last, gw.health());
        // 第一次 tick 立即触发：启动和换配置后马上刷新一次
        let mut refresh = tokio::time::interval(MODEL_REFRESH_INTERVAL);
        loop {
            tokio::select! {
                _ = refresh.tick() => {
                    if gw.configured().llm && let Err(err) = gw.refresh_models().await {
                        eprintln!("刷新 AI 模型列表失败：{err}");
                    }
                }
                r = health.changed() => {
                    if r.is_err() {
                        break;
                    }
                    let h = *health.borrow_and_update();
                    publish(&app, &mut last, h);
                }
                r = swapped.changed() => {
                    if r.is_err() {
                        return;
                    }
                    break;
                }
            }
        }
    }
}

/// 同步到状态快照的 `offline`（小组件“离线”角标、下发给输入法的天气），并推送 `gateway:health`。
fn publish(app: &AppHandle, last: &mut Option<GatewayHealth>, h: GatewayHealth) {
    if *last == Some(h) {
        return;
    }
    *last = Some(h);
    let (snapshot, changed) = app
        .state::<AppState>()
        .update_status(|status| StatusSnapshot::set(&mut status.offline, h.offline()));
    if changed {
        emit_status(app, snapshot);
    }
    if let Err(e) = GatewayHealthChanged(h).emit(app) {
        eprintln!("推送 gateway:health 失败：{e}");
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use xinqing_hub_gateway::LlmSecret;

    use super::*;

    fn rig(tag: &str) -> (Ai, PathBuf) {
        let d = std::env::temp_dir().join(format!("xq-ai-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        (Ai::new(SecretStore::new(&d), tx), d)
    }

    fn llm(url: &str) -> AiSecrets {
        AiSecrets {
            jev: None,
            llm: Some(LlmSecret {
                base_url: url.into(),
                api_key: Some(xinqing_hub_gateway::ApiKey::new("sk-test-5678")),
                models: vec![],
            }),
        }
    }

    #[test]
    fn debug_build_without_config_talks_to_mock_ai() {
        let (ai, d) = rig("devmock");
        let (source, masked) = ai.view();
        if secrets::load_dev_file().is_none() {
            assert_eq!(source, Source::DevMock);
            assert!(masked.llm.unwrap().base_url.starts_with("http://127.0.0.1"));
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn invalid_config_is_rejected_before_touching_the_file() {
        let (ai, d) = rig("invalid");
        let before = Arc::as_ptr(&ai.gateway());
        assert!(matches!(
            ai.save(llm("llm.example/v1")),
            Err(SecretsStoreError::Invalid(_))
        ));
        assert_eq!(Arc::as_ptr(&ai.gateway()), before, "保存失败不换网关");
        assert!(!d.join(secrets::SECRETS_FILE).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(not(windows))]
    #[test]
    fn other_platforms_cannot_save_keys() {
        let (ai, d) = rig("unsupported");
        assert!(matches!(
            ai.save(llm("https://llm.example/v1")),
            Err(SecretsStoreError::Unsupported)
        ));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(windows)]
    #[test]
    fn saving_swaps_the_gateway_and_keeps_the_key_tail_only() {
        let (ai, d) = rig("save");
        let swapped = ai.swapped.subscribe();
        let before = Arc::as_ptr(&ai.gateway());
        ai.save(llm("https://llm.example/v1")).unwrap();
        assert!(swapped.has_changed().unwrap());
        assert_ne!(Arc::as_ptr(&ai.gateway()), before);
        let (source, masked) = ai.view();
        if secrets::load_dev_file().is_none() {
            assert_eq!(source, Source::Saved);
            assert_eq!(masked.llm.unwrap().key_tail.as_deref(), Some("••••5678"));
        }
        // 再次保存时不填密钥：沿用已保存的那份
        let mut again = llm("https://llm2.example/v1");
        again.llm.as_mut().unwrap().api_key = None;
        ai.save(again).unwrap();
        let saved = SecretStore::new(&d).load().unwrap().unwrap();
        assert_eq!(
            saved.masked().llm.unwrap().key_tail.as_deref(),
            Some("••••5678")
        );
        // 两侧都清空：删除文件
        ai.save(AiSecrets::default()).unwrap();
        assert!(!d.join(secrets::SECRETS_FILE).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn reload_swaps_and_notifies() {
        let (ai, d) = rig("reload");
        let swapped = ai.swapped.subscribe();
        let before = Arc::as_ptr(&ai.gateway());
        ai.reload();
        assert!(swapped.has_changed().unwrap());
        assert_ne!(Arc::as_ptr(&ai.gateway()), before);
        let _ = std::fs::remove_dir_all(&d);
    }
}
