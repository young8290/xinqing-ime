//! 后端 → 前端事件（10 第 5.2 节）。事件名与产品书一致，载荷类型自动导出到 TypeScript。

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;
use xinqing_hub_core::care::ComfortSource;
use xinqing_hub_core::chat::ChatFailure;
use xinqing_hub_core::domain::rest::RestKind;
use xinqing_hub_core::domain::self_report::SelfWeather;
use xinqing_hub_core::domain::status::StatusSnapshot;
use xinqing_hub_core::infra::gateway::GatewayHealth;
use xinqing_hub_core::infra::imeconf::ImeConfigChange;

/// `status:changed`：载荷同 `get_status`。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "status:changed")]
pub struct StatusChanged(pub StatusSnapshot);

/// `settings:changed`：只带键名，窗口自行重新读取。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "settings:changed")]
pub struct SettingsChanged {
    pub key: String,
}

/// `self_report:changed`：用户刚自评（FR-STA-10）。到 `until_ts` 之前小组件显示“你说的：…”；
/// “说不上来”时 `until_ts` 就是自评时刻，即不覆盖显示。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "self_report:changed")]
pub struct SelfReportChanged {
    pub weather: SelfWeather,
    /// Unix 毫秒（前端绑定不导出 i64，毫秒时间戳在 f64 中是精确的）
    #[specta(type = specta_typescript::Number)]
    pub until_ts: f64,
}

/// `gateway:health`：Jev 与大模型两侧是否可用（`{jev, llm}`），任一侧不可用时小组件显示“离线”角标。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "gateway:health")]
pub struct GatewayHealthChanged(pub GatewayHealth);

/// `review:evening`：晚间小结（FR-REV-01）。小组件卡片层显示（FR-WGT-07）：统计、天气色带和一句本地模板（不加 AI 标识）；
/// 卡片上的“看看今天的看板”打开看板，“今天不用了”只收起卡片（同一天不会再出）。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "review:evening")]
pub struct ReviewEvening(pub xinqing_hub_core::domain::evening::EveningSummary);

/// `comfort:new`：晴晴说了一句暖心话（FR-CMF-04）。小组件一句话区显示；`ai_generated` 时句尾加 `AI 生成` 标签，
/// 模板句不加。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "comfort:new")]
pub struct ComfortNew {
    /// `comfort_log` 的行号，反馈（FR-CMF-05）时用
    pub id: u32,
    pub text: String,
    pub source: ComfortSource,
    pub ai_generated: bool,
}

/// `care:reduced`：连续 3 天的反馈都是 👎 / 🔕，主动关怀频率自动降了一档（FR-CMF-06 第 2 条）。
/// 小组件一句话区显示 `widget.care_reduced`（“我会少打扰你一些”）。`level` 是降档后的 `care.level`。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "care:reduced")]
pub struct CareReduced {
    pub level: String,
}

/// `rest:due`：该休息了（FR-RST-06）。小组件显示提醒卡片和 `已完成` / `5 分钟后` / `今天不再提醒`，
/// 用户的选择经 `rest_action` 交回后端。`tired` 时护眼卡片换文案“打了很久啦，眼睛也累了吧”（FR-RST-07）。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "rest:due")]
pub struct RestDue {
    pub kind: RestKind,
    pub tired: bool,
}

/// `research:invite`：研究模式的定时自评邀请（FR-DMO-04）。小组件弹出“主动报告心情”面板，可跳过（`research_dismiss`）；
/// `answer_until_ts`（Unix 毫秒）之前提交的自评记为研究数据（`source = esm`），过了就按普通自评记。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "research:invite")]
pub struct ResearchInvite {
    #[specta(type = specta_typescript::Number)]
    pub answer_until_ts: f64,
}

/// `ime_config:changed`：输入法配置变了（ADR 0017），设置中心的“输入法”分类重新取一次 `ime_config_get`。
/// 来自核心的 `config.changed`（`setItems` / `applyPatch` / `reload`），或外壳刚连上事件通道（`connected`）。
/// 核心自己的语言栏、菜单改的配置不广播，所以设置窗口重新获得焦点时也要取一次。
#[derive(Debug, Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "ime_config:changed")]
pub struct ImeConfigChanged(pub ImeConfigChange);

/// `chat:delta`：对话回复的一段增量（FR-CHT-04 第 1 条），按到达顺序拼接显示。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "chat:delta")]
pub struct ChatDelta {
    pub request_id: u32,
    pub text_delta: String,
}

/// `chat:done`：回复结束。`text` 是最终写库的全文，校验替换或截断过时与增量拼出来的不同，界面以它为准；
/// `ai_generated` 为假（固定回应、替换句）时不显示 `AI 生成` 标签。用户停止生成且一个字都没有时 `message_id` 为 `null`。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "chat:done")]
pub struct ChatDone {
    pub request_id: u32,
    pub session_id: u32,
    pub message_id: Option<u32>,
    pub text: String,
    pub ai_generated: bool,
    pub stopped: bool,
}

/// `chat:error`：没拿到回复（超时、模型不可用、今日额度用完）。用户消息已保存，界面移除半截回复，
/// 显示 `chat.stuck` 或 `chat.daily_cap` 和“重试”（`chat_retry`）。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "chat:error")]
pub struct ChatErrorEvent {
    pub request_id: u32,
    pub session_id: u32,
    pub reason: ChatFailure,
}

/// `safety:invite` 的来源。目前只有温柔改写。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SafetyInviteSource {
    Rewrite,
}

/// `safety:invite`：改写时原文命中危机词表（FR-RWR-05 第 3 条，ADR 0028 第 4 条）。核心已退出改写模式并在光标旁
/// 显示气泡；界面在小组件一句话区给出最高优先级的求助入口（`rewrite.crisis_bubble`），点了打开对话并显示求助卡片。
/// 不自动弹出对话窗口：用户正在别的应用里打字。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "safety:invite")]
pub struct SafetyInvite {
    pub source: SafetyInviteSource,
}

/// `safety:triggered`：危机识别命中（FR-SAF-01）。对话窗口立即在顶部固定显示求助卡片（FR-SAF-02），
/// 本会话切换为安全模式。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "safety:triggered")]
pub struct SafetyTriggered {
    pub session_id: u32,
}
