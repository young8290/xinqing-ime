# mock-ai
- 职责：本地模拟 Jev（`POST /v1/systemone`）和 OpenAI 兼容接口（`POST /v1/chat/completions` 含 SSE、`GET /v1/models`），只监听 127.0.0.1。
- 场景：`--scenario normal|slow|timeout|500|422|model_not_found|stream_cut`，单个请求可用请求头 `X-Mock-Scenario` 覆盖。`model_not_found` 只让优先级第一的模型（`gemini-3.7-flash`）不存在，备选模型照常可用。
- 同时是一个库：网关集成测试用 `mock_ai::serve(场景)` 在进程内起服务，返回的 `Handle` 记录各接口收到的请求数，并能在运行中切换场景（测熔断恢复）。
- Q-STATE 按 `state.local_hints` 给出确定的概率，回放脚本能得到可复现的状态序列。
- 大模型回复：请求是 P-COMFORT（提示词里要求输出 `{"text":...,"kind":"comfort|rest|cheer"}`）时，按调用次数轮换 `COMFORT_REPLIES` 里的三句 JSON，能过暖心话的校验和去重；其余请求回固定的 `REPLY`。
- 注意：Jev 的线上请求格式产品书未定义，这里沿用 Hub 网关类型（questions + state → answers），拿到 Jev 文档后调整。
