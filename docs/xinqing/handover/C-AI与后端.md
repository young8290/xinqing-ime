# C AI 与后端 · 交接文档

> 负责范围（产品书 13 第 1 节）：AIG 网关、CMF 暖心话、CHT 对话、DIA 日记、SAF 危机安全、SCH 日程与待办、RWR 温柔改写的 Hub 部分、REV-01/02 晚间小结与周信、评测脚本；
> 另是数据库迁移（`xinqing_hub/core/migrations/`）与设置键注册表（`xinqing_hub/core/src/domain/settings.rs`）的契约负责人（13 第 3.1 节）。
> 任务清单与估算见产品书 16 第 2.3 节。本文件随 C 的每个 PR 更新，任务中途换人时按产品书 13 第 3.1 节“交接”直接看这里。
> 最后更新：2026-10-04（C-03 第二部分：网关接入外壳、密钥保存、AI 服务命令）

## 1. 任务状态

| 编号 | 任务 | 状态 | 代码 / PR | 还差什么 |
|---|---|---|---|---|
| C-01 | Hub 骨架：分层、事件总线、SQLite 迁移 v1、单写线程、settings | 大部分完成 | `core/src/bus.rs`、`core/migrations/0001_init.sql`、`core/src/infra/store/`、`core/src/domain/settings.rs` | 单写线程 `DbWriter`（17 第 2.9 节）没做，外壳现在用 `Mutex<Db>` 串行；设置键只登记了 7 个（10 第 6.2 节列的 `care.*`、`ai.*`、`rest.*` 等随各功能补）；D-08 的表单生成器要一个“键注册表 → schema”的命令，待和 D 约定（契约 PR） |
| C-02 | mock-ai（Jev 三类、OpenAI 兼容含 SSE、各故障场景） | 完成 | `tools/mock-ai/` | 16 第 3 节建议移给 E，W1 评审会还没定 |
| C-03 | AI 网关：trait、Jev/LLM 客户端、熔断、重试、健康检查、隐私过滤、密钥 DPAPI、预算 | **进行中（主体完成）** | HTTP 实现：`xinqing_hub/gateway/`；trait、熔断、预算、脱敏：`core/src/infra/gateway/`；接入外壳：[xinqing-ime#7](https://github.com/young8290/xinqing-ime/pull/7)（Hu-yiye）+ [xinqing-ime#21](https://github.com/young8290/xinqing-ime/pull/21)，ADR 0011 | 见第 3 节 |
| C-04 | 暖心话：触发、生成、校验、模板兜底、频率控制、反馈 | 未开始（素材已备） | 模板 `hub_templates/comfort.toml`（2 风格 × 6 组 × 15 句）、提示词 `prompts/comfort.md`、校验器 `core/src/domain/validate.rs`（V1/V3/V4/V5/V7） | 计划 W5–W6，下一个做。触发条件 1 要 Jev 的 `need_comfort`，降级时不主动关怀（04 FR-STA-08 第 2 条）；自评回应（FR-STA-10）也走它，要给 B 一个接口 |
| C-05 | 日程识别：L1/L2/L3、代码校验、去重、提醒调度、冲突 | 未开始 | `hub_templates/schedule_patterns.toml`、`prompts/schedule.md`、评测集 `eval/datasets/e_plan.jsonl`、`e_extract.jsonl` | 计划 W7–W8；提醒调度 `scheduler` 也给 B-04（04:00 重算基线）用 |
| C-06 | 待办识别与清单、提醒 | 未开始 | `prompts/todo.md`、`eval/datasets/e_todo.jsonl` | 计划 W8（P1） |
| C-07 | AI 对话：会话、上下文、流式、记忆、历史、快捷指令 | 未开始 | `prompts/chat.md`、`chat_safe.md`；网关的 `stream` 已可用 | 计划 W7–W8；D-06 对话窗口在等命令和 `chat:*` 事件 |
| C-08 | 危机安全：双通道、求助卡片、安全模式、记录、误报处理 | 部分完成 | 本地通道 `core/src/domain/safety.rs`（`check_local`，与 `eval/tools/check_crisis.py` 对拍：`core/tests/crisis_parity.rs`）、`hub_templates/crisis_lexicon.toml` | Q-CRISIS 通道、求助卡片事件、安全模式、`safety_log`、误报处理；V6 校验。计划 W7 |
| C-09 | 温柔改写 Hub 服务：脱敏、P-REWRITE、保真校验、缓存 | 未开始 | `prompts/rewrite.md`、`eval/datasets/e_rewrite.jsonl`；`sense.rs` 收到 `rewrite_req` 时先回 `RewriteFail::Offline` | 计划 W8–W9；依赖 A-08 改写模式；V8 校验、可还原占位符 `[号码1]` |
| C-10 | 情绪日记、晚间小结、周信 | 未开始 | `hub_templates/evening.toml`、`letter_fallback.md`、`prompts/diary.md`、`letter.md` | 16 第 3 节建议移给 B，W1 评审会还没定（B 的交接文档也记了“未认领”） |
| C-11 | 评测脚本与报告 | 脚本完成，报告未出 | `eval/tools/`（[xinqing-ime#6](https://github.com/young8290/xinqing-ime/pull/6)，Hu-yiye）：日程 / 抽取 / 待办、对话安全 / 改写、周信、危机词表 | E-COMFORT 没有脚本；报告（`eval/reports/`）要等各服务做完、用真实接口跑出预测文件 |

## 2. 代码地图（C 负责的部分）

```
xinqing_hub/
├─ core/src/infra/gateway/   AiGateway trait 与类型、问题目录与字段白名单、熔断 breaker.rs、预算 budget.rs、脱敏 privacy.rs、离线网关
├─ core/src/domain/
│  ├─ validate.rs            08 第 5 节输出校验（V1、V3 计数、V4 禁用词、V5 重复、V7 语言；V2/V6/V8/V9 随功能补）
│  ├─ safety.rs              危机词表本地通道
│  └─ settings.rs            设置键注册表（契约）
├─ core/src/infra/store/     Db：迁移、窗口与状态、反馈、net_log（只留最近 200 条）
├─ core/migrations/          0001_init.sql（契约；已发布的脚本不改，新改动另起 0002_*.sql）
├─ gateway/src/
│  ├─ lib.rs                 HttpGateway：预算 → 选模型 → 隐私过滤 → 发送 → 熔断与统计 → 重试；health 订阅、换配置时接过预算
│  ├─ jev.rs / llm.rs / sse.rs   Jev 客户端；OpenAI 兼容客户端（分场景参数、换模型重试、/models、测试连接）；SSE 解析
│  ├─ config.rs              GatewayConfig 与默认值；ApiKey（Zeroizing，Debug 不输出，界面只露末 4 位）
│  └─ secrets.rs             AiSecrets：secrets.toml / secrets.bin 的内容格式、校验、“空密钥沿用旧值”、转 GatewayConfig
└─ src-tauri/src/
   ├─ gateway.rs             Ai（托管为 Arc<Ai>，本身实现 AiGateway）：选配置来源、出网日志写库、gateway:health、每 30 分钟刷新模型
   ├─ secrets.rs             secrets.bin 的 DPAPI 加解密（Windows）；dev 构建读明文 secrets.toml
   └─ commands/ai.rs         ai_config_get、secrets_set、ai_test_connection、ai_usage_today
hub_templates/               comfort.toml、prompts/*.md、crisis_lexicon.toml、banned_words.toml、schedule_patterns.toml、evening.toml、letter_fallback.md
tools/mock-ai/               模拟 Jev 与大模型（normal/slow/timeout/500/422/model_not_found/stream_cut）
eval/                        评测集与离线评测脚本
```

验证：`cargo test -p xinqing-hub-core -p xinqing-hub-gateway -p xinqing-hub`（`gateway/tests/mock_ai.rs` 在进程内起 mock-ai）。
看效果（Linux 也行，需要 WebKitGTK）：`cargo run -p mock-ai -- --port 18080 --scenario normal`，再 `cargo run -p xinqing-hub`；
dev 构建默认连这个 mock-ai，启动后 `net_log` 表里会出现一条 `llm/models`。换成真实接口：把 `secrets.example.toml` 拷成仓库根目录的
`secrets.toml` 填值（已被 `.gitignore` 忽略），或在 Windows 上用设置页保存。

## 3. 进行中：C-03

| 部分 | 需求 | 状态 |
|---|---|---|
| Jev / 大模型 HTTP 客户端、熔断、重试、限速、预算、脱敏、出网日志、`/models`、测试连接 | FR-AIG-01～05、07、08 | 完成（搬入时已有，`gateway/tests/mock_ai.rs` 覆盖 TC-AIG-01～05、07、08 的网关部分） |
| Hub 启动时建网关、出网日志写 `net_log`、健康状态同步到 `offline`、每 30 分钟刷新模型列表 | FR-AIG-04、17 第 2.2 节第 4 步 | 完成：Hu-yiye 的 #7 起头（编译不过：少引入 `AiGateway` trait），本 PR 合入并改写为可换配置的 `Ai` |
| `gateway:health` 事件 `{jev, llm}` | 10 第 5.2 节 | 完成（本 PR） |
| 密钥 DPAPI 保存 `secrets.bin`、内存清零、界面只显示末 4 位 | FR-AIG-06 | 完成（本 PR），ADR 0011。Windows 上的加解密只在 Windows CI 的 `cargo test` 里跑（`secrets.rs` 的 `dpapi_round_trip_and_no_plaintext_on_disk`、`gateway.rs` 的 `saving_swaps_the_gateway…`），本机交叉检查过类型 |
| `ai_config_get`（ADR 0011 新增）、`secrets_set`、`ai_test_connection`、`ai_usage_today` | 10 第 5.1 节、FR-SET-08、FR-ONB-05 | 完成（本 PR），绑定已重新生成，等 D 接到设置页与引导页 |
| `ai.daily_caps`、`ai.llm_models` 设置键 → `set_cap` / 模型列表 | FR-AIG-07、10 第 6.2 节 | 未做：`SettingValue` 只有布尔、数字、字符串，`daily_caps` 是表；要么拆成 `ai.cap.jev` 这类数字键，要么扩 `SettingValue`，属于契约改动，先和 D 商量。模型列表现在跟着 `secrets_set` 存 |
| 设置页“最近 20 次出网请求”（`Db::net_log_recent` 已有） | FR-SET-09 | 未做：要一个命令，10 第 5.1 节没列，和 D 的设置页隐私分类一起加 |
| 演示者视图的网关指标（`HttpGateway::metrics` / `models` 已有） | FR-AIG-08 | 未做：随 B-10 演示模式 |
| Jev 判断接进实时感知（`Sense`） | FR-STA-05 | **B 的任务**（B 交接文档 B-06）。外壳托管的 `Arc<Ai>` 实现 `AiGateway`，`sensing::start` 里 `app.state::<Arc<Ai>>()` 拿到后转成 `Arc<dyn AiGateway>` 传给 `Sense` 即可；换配置不用重新取 |

## 4. 关键决定与待评审

- **ADR 0011（AI 服务配置的存放与读取，8 条）**：提议，待 D（界面）与 E（隐私）评审。要点：`secrets.bin` 存整份配置（地址 + 密钥，TOML 后整份 DPAPI）；
  新增 `ai_config_get`；`secrets_set` 里空密钥沿用旧值；配置来源优先级 dev `secrets.toml` → `secrets.bin` → dev 连 mock-ai → 离线；
  非 Windows 不保存密钥；换配置不清零今日预算。接受后要改产品书 10 第 5.1 节、08 FR-AIG-06、14 第 4 节。
- **接手 #7 的方式**：没有改 Hu-yiye 的分支，而是在本分支合入它（保留其提交），在上面修编译错误并补完。#7 可以在本 PR 合并后关闭（合并后 GitHub 会显示它已包含在 main 里）。
- **“测试连接”只测大模型**：Jev 没有不耗额度的探测接口，它的可用性看 `gateway:health` 和出网日志（ADR 0011 第 8 条）。

## 5. 已知问题

- `secrets_set` 的明文密钥经 Tauri IPC 的 JSON 反序列化进来，那一份中间缓冲清不掉；进入 Rust 后立即放进 `Zeroizing`。
- 熔断打开期结束后不会自己发健康变化，要等下一次请求或每 30 分钟的模型刷新（网关 README 也写了）；离线角标因此最多晚 30 分钟消失。
- 健康状态“离线”= 任一侧不可用（`GatewayHealth::offline`）。只配了大模型、没配 Jev 时也显示离线角标，符合 03 第 8 节“L2 Jev 不可用”的降级，但用户可能疑惑，设置页要说明。
- 出网日志写库走 `Mutex<Db>`，和感知任务抢同一把锁；`DbWriter`（C-01 剩余）接入后改走单写线程。
- `ai_usage_today` 只列网关自己计数的 Jev、大模型、对话、改写四类；日程初筛（C-05）和周信（C-10）由各自服务计数，做完再加。

## 6. 下一步（按优先级）

1. C-04 暖心话（P0，W5–W6）：`domain/comfort.rs`（触发、P-COMFORT 组装与 V1–V5/V7 校验、模板兜底与屏蔽列表、频率档位、`comfort_log`），
   `comfort:new` 事件，`submit_feedback` 加 `comfort` 目标；同时给 B 的自评回应留接口。设置键 `care.level`、`care.quiet_hours`、`care.dnd_apps` 走契约 PR；
2. C-01 剩余：`DbWriter` 单写线程；
3. C-08 危机安全（P0，W7）与 C-07 对话（P0，W7–W8）：D-06 对话窗口在等；
4. C-05 日程（P0，W7–W8），连同提醒调度 `scheduler`。

## 7. 修订记录

| 日期 | 改动 |
|---|---|
| 2026-10-04 | 初版：盘点 C-01～C-11 现状；C-03 第二部分（接手 #7 的网关接线，密钥 DPAPI 保存，AI 服务命令与 `gateway:health`，ADR 0011） |
