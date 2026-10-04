# xinqing-hub-core
- 职责：Hub 的领域层和平台无关的基础层（FR-STA-01～08、FR-SAF-01 本地通道、08 第 5 节 V1/V3/V4/V5/V7、FR-AIG-02/05/07 的熔断/脱敏/预算、09 第 3 节数据库）。
- 对外接口：`pipeline::StatePipeline`（XQP 事件 → 窗口 → 特征 → 规则 → 融合）、`pipeline::replay`、`domain::safety::check_local`、`domain::validate::BannedWords`、`infra::store::Db`（含 `net_log` 读写，只留最近 200 条）、`infra::gateway::AiGateway` 与 `NetLogEntry`（HTTP 实现在 `xinqing_hub/gateway`）；界面用的 `domain::status`（显示状态快照与天气映射）、`domain::consent`（六项单独同意）、`domain::settings`（设置键注册表 `KEYS`，默认值与取值范围）。
- 依赖：只依赖 `xqp` 和通用库，**禁止**依赖 tauri / windows（NFR-MNT-03，见 docs/adr/0007）。可选特性 `specta` 只给外壳导出 TypeScript 类型用。
- 配置：读取 `hub_templates/` 中的 baseline_default、app_categories、crisis_lexicon、banned_words。
- 数据：`migrations/0001_init.sql`；`window_features.features_json` 写入前检查只含数值和 null。
- 测试：`cargo test -p xinqing-hub-core`；`tests/crisis_parity.rs` 与 Python 参考实现对拍，`tests/replay_scripts.rs` 用合成脚本做端到端回归（TC-STA-06 口径）。
- 回放工具：`cargo run -p xinqing-hub-core --bin xq-replay -- <脚本> [--mock-jev] [--start-at 23:40] [--baseline 文件] [--json]`。
- 隐私检查：窗口只存时间戳和类别，不存文字；进程名只用于分类，不落库、不出网；Q-STATE 请求按字段白名单过滤，文本字段先脱敏。
