# xinqing-hub-gateway
- 职责：AI 网关的 HTTP 实现（08 FR-AIG-01～05、07、08，17 第 2.5 节）。`HttpGateway` 实现 core 的 `AiGateway` trait：Jev 客户端（合并请求、连接 3 秒 / 总 5 秒、只对超时 / 5xx / 网络错误重试 1 次、在途 ≤ 2、熔断、每分钟 ≤ 60 次）；OpenAI 兼容大模型客户端（分场景参数、SSE 流式、换下一个模型重试 1 次、每个模型独立熔断、`/models` 健康检查、暖心话选 P50 最低的模型、“测试连接”）。
- 对外接口：`HttpGateway::new(GatewayConfig, Clock)`、`with_net_log`（出网日志通道）、`subscribe_health`（推 `gateway:health` 用）、`refresh_models`、`test_connection`、`metrics`、`models`、`budget_used`、`set_cap`；配置类型 `GatewayConfig` / `JevConfig` / `LlmConfig` / `ScenarioParams` / `ApiKey`。
- 依赖：`xinqing-hub-core`、reqwest（rustls，加密后端用 ring，不需要 CMake / NASM）。领域层只认 trait，不依赖本 crate（TC-AIG-06，见 docs/adr/0007）。
- 配置：默认值都取自产品书；dev 构建用 `GatewayConfig::dev_from_env()`，读 `XQ_JEV_BASE_URL` / `XQ_LLM_BASE_URL`，默认指向本机 mock-ai，不需要密钥（14 第 3.3 节）。
- 测试：`cargo test -p xinqing-hub-gateway`。`tests/mock_ai.rs` 在进程内起 mock-ai，覆盖 TC-AIG-01～05、07、08 的网关部分：熔断与恢复、5xx 重试、422 不重试、超时、限速、预算、`model_not_found` 换模型、`/models`、流式拼接、断流、首字超时、中途取消、密钥只出现在认证头。
- 隐私检查：Jev 只发字段白名单内的 state；所有出网文本先脱敏；出网日志（`NetLogEntry`）只有时间、接口、模型、字段名、耗时、状态、token 用量，不含内容；`ApiKey` 用后清零，`Debug` 不输出，界面只显示末 4 位（FR-AIG-06）。
- 待定：Jev 线上请求格式产品书没给，`jev.rs` 的 `Wire*` 与 mock-ai 用同一暂定结构；密钥的 DPAPI 存取（FR-AIG-06）在外壳实现。
