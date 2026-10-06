# C AI 与后端 · 交接文档

> 负责范围（产品书 13 第 1 节）：AIG 网关、CMF 暖心话、CHT 对话、DIA 日记、SAF 危机安全、SCH 日程与待办、RWR 温柔改写的 Hub 部分、REV-01/02 晚间小结与周信、评测脚本；
> 另是数据库迁移（`xinqing_hub/core/migrations/`）与设置键注册表（`xinqing_hub/core/src/domain/settings.rs`）的契约负责人（13 第 3.1 节）。
> 任务清单与估算见产品书 16 第 2.3 节。本文件随 C 的每个 PR 更新，任务中途换人时按产品书 13 第 3.1 节“交接”直接看这里。
> 最后更新：2026-10-06（C-10 第二部分：周信，ADR 0030）

## 1. 任务状态

| 编号 | 任务 | 状态 | 代码 / PR | 还差什么 |
|---|---|---|---|---|
| C-01 | Hub 骨架：分层、事件总线、SQLite 迁移 v1、单写线程、settings | **基本完成** | `core/src/bus.rs`、`core/migrations/`（0001、0002）、`core/src/infra/store/`（单写入口 `writer.rs`，ADR 0020，[xinqing-ime#54](https://github.com/young8290/xinqing-ime/pull/54)（已合并））、`core/src/domain/settings.rs`（设置键 `KEYS`、内部键 `INTERNAL_KEYS`）；设置值的列表与格式类型：#44（已合并），ADR 0019；设置页用的 `settings_schema` 命令：[xinqing-ime#62](https://github.com/young8290/xinqing-ime/pull/62)（ADR 0019 第 9 条） | 设置键已登记 29 个，10 第 6.2 节其余的（`review.*`、`sch.*`、`hotkey.*` 等）随各功能补 |
| C-02 | mock-ai（Jev 三类、OpenAI 兼容含 SSE、各故障场景） | 完成 | `tools/mock-ai/` | 16 第 3 节建议移给 E，W1 评审会还没定 |
| C-03 | AI 网关：trait、Jev/LLM 客户端、熔断、重试、健康检查、隐私过滤、密钥 DPAPI、预算 | 主体完成 | HTTP 实现：`xinqing_hub/gateway/`；trait、熔断、预算、脱敏：`core/src/infra/gateway/`；接入外壳：[xinqing-ime#21](https://github.com/young8290/xinqing-ime/pull/21)（已合并，含 Hu-yiye 的 [#7](https://github.com/young8290/xinqing-ime/pull/7)），ADR 0012；最近出网记录命令：[xinqing-ime#48](https://github.com/young8290/xinqing-ime/pull/48) | 见第 3.2 节 |
| C-04 | 暖心话：触发、生成、校验、模板兜底、频率控制、反馈 | **后端完成** | 第一部分（主动关怀与自评回应）：[xinqing-ime#27](https://github.com/young8290/xinqing-ime/pull/27)（已合并）；第二部分（反馈与自动降档）：[xinqing-ime#29](https://github.com/young8290/xinqing-ime/pull/29)（已合并）；第三部分（安静时段、自定义勿扰应用）：[xinqing-ime#44](https://github.com/young8290/xinqing-ime/pull/44)（已合并）；ADR 0015、0019 | 见第 3.1 节：主动关怀要等 B 把 Jev 接进 `Sense`；系统专注助手不判断（ADR 0019 第 7 条）；E-COMFORT 评测 |
| C-05 | 日程识别：L1/L2/L3、代码校验、去重、提醒调度、冲突 | 本地初筛、结构化存储、AI 抽取校验与代码日期校验完成 | [xinqing-ime#42](https://github.com/young8290/xinqing-ime/pull/42)（已合并）：`core/src/domain/schedule.rs`、`core/src/infra/store/schedule.rs`；P-SCHEDULE / P-TODO 输出校验（08 第 5 节 V1～V3）与 FR-SCH-04 校验第 2～5 条的规范化（`validate_schedule_json`、`validate_todo_json`，同在 `domain/schedule.rs`）：Hu-yiye 的 [xinqing-ime#52](https://github.com/young8290/xinqing-ime/pull/52)；FR-SCH-04 校验第 1 条（按原句代码算日期时刻，`domain/when.rs`）、标题截断、V4 只拦模型带进来的词、`need_time` 标记：[xinqing-ime#57](https://github.com/young8290/xinqing-ime/pull/57)，ADR 0024；模板 `hub_templates/schedule_patterns.toml`、评测集 `eval/datasets/e_plan.jsonl`、`e_extract.jsonl` | L2/L3 网关接线（调 `validate_schedule_json` 时把原句和禁用词表传进去）、提醒调度与冲突处理待做；ADR 0024 待 E 评审；提醒调度 `scheduler` 也给 B-04（04:00 重算基线）用 |
| C-06 | 待办识别与清单、提醒 | 本地初筛与结构化存储完成 | #42（已合并）：`core/src/domain/schedule.rs`、`core/src/infra/store/schedule.rs`；`prompts/todo.md`、评测集 `eval/datasets/e_todo.jsonl` | AI 抽取的网关接线、提醒调度与前端清单待做（P1）；`validate_todo_json` 已按原句算截止日期（ADR 0024） |
| C-07 | AI 对话：会话、上下文、流式、记忆、历史、快捷指令 | **P1 部分完成** | [xinqing-ime#40](https://github.com/young8290/xinqing-ime/pull/40)（已合并）：纯函数 `core/src/domain/chat.rs`、服务 `core/src/chat.rs`、读写 `infra/store/chat.rs`、外壳 `src-tauri/src/chat.rs` 与 `commands/chat.rs`；`gateway/tests/chat_mock_ai.rs`；ADR 0018。记忆增删改、历史搜索、`chat.retention_days`：[xinqing-ime#49](https://github.com/young8290/xinqing-ime/pull/49)。快捷指令的对话方式：[xinqing-ime#51](https://github.com/young8290/xinqing-ime/pull/51)，ADR 0022 | 见第 3.3 节：两段快捷指令提示词待 E 评审；“写成情绪日记”随 C-10；设置页“晴晴记住的事”与历史搜索界面等 D |
| C-08 | 危机安全：双通道、求助卡片、安全模式、记录、误报处理 | **对话部分完成** | #40（已合并）：本地通道 `core/src/domain/safety.rs`（与 Python 对拍）；对话里的双通道、安全模式、固定回应、`safety_log`、V6、“我说的不是这个意思”在 `core/src/chat.rs`；ADR 0018 | 日记中的危机识别随 C-10；12 的对话安全用例集要真实接口跑 |
| C-09 | 温柔改写 Hub 服务：脱敏、P-REWRITE、保真校验、缓存 | **后端完成** | [xinqing-ime#65](https://github.com/young8290/xinqing-ime/pull/65)（已合并）：纯函数 `core/src/domain/rewrite.rs`（占位符、V1、V4/V8、提示词）、服务 `core/src/rewrite.rs`、外壳 `src-tauri/src/rewrite.rs`、`gateway/tests/rewrite_mock_ai.rs`；ADR 0028 | 见第 3.4 节：危机入口的界面（D）、首次使用说明（A/C/D 待定）、E-REWRITE 评测（真实接口）；禁用词表第九类 `abuse` 待 E 评审 |
| C-10 | 情绪日记、晚间小结、周信 | **进行中（C 认领）**：晚间小结、周信后端完成 | 第一部分晚间小结：[xinqing-ime#68](https://github.com/young8290/xinqing-ime/pull/68)（已合并），纯函数 `core/src/domain/evening.rs`、服务 `core/src/evening.rs`、外壳 `src-tauri/src/evening.rs`，ADR 0029；第二部分周信：本 PR，纯函数 `core/src/domain/letter.rs`、服务 `core/src/letter.rs`、外壳 `src-tauri/src/letter.rs` 与 `commands/letter.rs`、`infra/store/letter.rs`、`gateway/tests/letter_mock_ai.rs`，ADR 0030；模板 `evening.toml`、`letter_fallback.md`、新增 `letter_tips.toml`；日记还要用 `prompts/diary.md` | 见第 3.5、3.6 节：卡片与信件界面（D-04）；`letter_tips.toml` 待 E 评审；情绪日记（FR-DIA）。16 第 3 节建议移给 B，W1 评审会没定，C 先认领（ADR 0029 第 1 条），定给 B 时按本节交接 |
| C-11 | 评测脚本与报告 | 脚本完成，报告未出 | `eval/tools/`（[xinqing-ime#6](https://github.com/young8290/xinqing-ime/pull/6)，Hu-yiye）：日程 / 抽取 / 待办、对话安全 / 改写、周信、危机词表、E-COMFORT | E-COMFORT：`eval_comfort.py` 与评测集 `e_comfort.jsonl`（6 种状态摘要 × 若干次，共 50 条），[xinqing-ime#50](https://github.com/young8290/xinqing-ime/pull/50)；报告（`eval/reports/`）仍要等真实接口生成预测文件 |

## 2. 代码地图（C 负责的部分）

```
xinqing_hub/
├─ core/src/infra/gateway/   AiGateway trait 与类型、问题目录与字段白名单、熔断 breaker.rs、预算 budget.rs、脱敏 privacy.rs、离线网关
├─ core/src/domain/
│  ├─ schedule.rs            日程/待办逐句本地初筛，原句只在内存候选中保留
│  ├─ comfort.rs             暖心话：频率档位、触发判断 Trigger、状态摘要、P-COMFORT 拼装、V1–V7 校验、模板选句
│  ├─ validate.rs            08 第 5 节输出校验（V1、V3 计数、V4 禁用词、V5 重复、V7 语言；V2/V6/V8/V9 随功能补）
│  ├─ safety.rs              危机词表本地通道
│  ├─ evening.rs             晚间小结：何时出 due、主导状态与模板分组、天气色带 band、卡片内容 summarize
│  ├─ letter.rs              周信：哪一周 due_week、一周统计 stats（发给大模型的 JSON）、长度 / V4 / V9 校验 check、P-LETTER 拼装、兜底模板 LetterFallback
│  ├─ rewrite.rs             温柔改写：可还原占位符 mask/restore、V1 解析、V4/V8/长度校验 check/accept、P-REWRITE 拼装
│  ├─ dnd.rs                 勿扰应用与安静时段的解析、判断（暖心话用，休息提醒也可用）
│  └─ settings.rs            设置键注册表（契约）：值的四种形态与格式校验（ADR 0019）、`schema()`（给 `settings_schema`）、`cap_key` / `caps`
├─ core/src/evening.rs       晚间小结服务 EveningService：记最近输入，每 5 秒看一次该不该出 → EveningPort::show（review:evening）
├─ core/src/letter.rs        周信服务 LetterService：每 30 秒看一次该不该写 → P-LETTER（不过重写 1 次）或模板 → LetterPort::save / show（letter:new）
├─ core/src/rewrite.rs       温柔改写服务 RewriteService：rewrite_req → 同意 ⑥ / 危机词表 / 占位符 / P-REWRITE / V8 → rewrite_result；rewrite_done → rewrite_log
├─ core/src/care.rs          暖心话服务 ComfortService：订阅总线 → 触发 → 大模型（重试 1 次）或模板 → comfort_log → ComfortPort::show
├─ core/src/infra/store/     Db：迁移、只读连接 open_reader、单写入口 writer.rs（DbWriter）、窗口与状态、反馈、net_log（只留最近 200 条）；comfort.rs 是 comfort_log 的读写；schedule.rs 写结构化日程/待办并去重；letter.rs 是 letter 表的读写
├─ core/migrations/          0001_init.sql、0002_comfort_trigger.sql（契约；已发布的脚本不改，新改动按 main 上的最大编号顺延）
├─ gateway/src/
│  ├─ lib.rs                 HttpGateway：预算 → 选模型 → 隐私过滤 → 发送 → 熔断与统计 → 重试；health 订阅、换配置时接过预算
│  ├─ jev.rs / llm.rs / sse.rs   Jev 客户端；OpenAI 兼容客户端（分场景参数、换模型重试、/models、测试连接）；SSE 解析
│  ├─ config.rs              GatewayConfig 与默认值；ApiKey（Zeroizing，Debug 不输出，界面只露末 4 位）
│  └─ secrets.rs             AiSecrets：secrets.toml / secrets.bin 的内容格式、校验、“空密钥沿用旧值”、转 GatewayConfig
└─ src-tauri/src/
   ├─ gateway.rs             Ai（托管为 Arc<Ai>，本身实现 AiGateway）：选配置来源、出网日志写库、gateway:health、每 30 分钟刷新模型
   ├─ secrets.rs             secrets.bin 的 DPAPI 加解密（Windows）；dev 构建读明文 secrets.toml
   ├─ commands/ai.rs         ai_config_get、secrets_set、ai_test_connection、ai_usage_today
   └─ comfort.rs             启动暖心话服务、实现 ComfortPort（设置、同意 ④、前台全屏、写库）、推 comfort:new、工具栏小圆点
hub_templates/               comfort.toml、prompts/*.md、crisis_lexicon.toml、banned_words.toml、schedule_patterns.toml、evening.toml、letter_fallback.md、letter_tips.toml
tools/mock-ai/               模拟 Jev 与大模型（normal/slow/timeout/500/422/model_not_found/stream_cut）；P-COMFORT 请求回合法 JSON；P-REWRITE 回三个候选；P-LETTER 回一封能过校验的信
eval/                        评测集与离线评测脚本
```

验证：`cargo test -p xinqing-hub-core -p xinqing-hub-gateway -p xinqing-hub`（`gateway/tests/mock_ai.rs`、`comfort_mock_ai.rs` 在进程内起 mock-ai）。
看效果（Linux 也行，需要 WebKitGTK）：`cargo run -p mock-ai -- --port 18080 --scenario normal`，再 `cargo run -p xinqing-hub`；
dev 构建默认连这个 mock-ai，启动后 `net_log` 表里会出现一条 `llm/models`。换成真实接口：把 `secrets.example.toml` 拷成仓库根目录的
`secrets.toml` 填值（已被 `.gitignore` 忽略），或在 Windows 上用设置页保存。

## 3. 进行中

### 3.1 C-04 暖心话

| 部分 | 需求 | 状态 |
|---|---|---|
| 主动关怀触发：`need_comfort ≥ 0.75`、非流畅状态连续 ≥ 2 个窗口、冷却、每日上限、无痕 | FR-CMF-01 第 1–4、6 条 | 完成（#27）：`domain::comfort::Trigger`。**要 Jev 接进 `Sense` 后才会真正触发**：`MoodEvent::Sample` 新增 `need_comfort`、`valence`，现在 `sense.rs` 填 `None`，降级时不主动关怀（FR-STA-08 第 2 条）。接 Jev 的 B 把这两个值填上即可 |
| 勿扰：前台全屏（含放映）、勿扰应用、安静时段 | FR-CMF-01 第 5 条 | 完成（#27、#44）：全屏复用 D 的 `fullscreen::foreground_fullscreen`；勿扰应用读 `care.dnd_apps`（默认三个会议应用，用户可删），按 XQP `focus` 的进程名忽略大小写比较；安静时段读 `care.quiet_hours`（`"22:30-07:00"`，可跨午夜）。判断在 `domain::dnd`。系统专注助手没有公开接口，不判断（ADR 0019 第 7 条） |
| 状态摘要（不含原文）、P-COMFORT、校验 V1/V2/V3/V4/V5/V7、不通过重试 1 次、模板兜底 | FR-CMF-02/03 | 完成（#27）。未同意 ④ 时不调大模型；网关报错不重试；第一次已用 5 秒以上不重试（ADR 0015 第 5 条） |
| 模板：按风格与组随机、排除最近 10 条与用户屏蔽、深夜用 `late_night` 组、简洁风格取 ≤ 15 字 | FR-CMF-03、FR-CHT-10 | 完成（#27）；出厂 180 句都过长度与禁用词（单测） |
| 写 `comfort_log`（含 `trigger`）、推 `comfort:new {id, text, source, ai_generated}`、小组件隐藏时点亮工具栏小圆点 | FR-CMF-04、09 D-10 | 完成（#27），迁移 0002。打字机效果、`AI 生成` 标签、小精灵“靠近”、2 小时后淡出为“今日一句”都在前端，等 D 接（`play('approach')` 已有） |
| 负面自评立即回应，不受冷却与上限限制，不占名额 | FR-STA-10 第 2 条 | 完成（#27）：订阅 `HubEvent::SelfReport`。“和晴晴聊聊”按钮是界面（D），对话是 C-07 |
| 频率档位 `care.level`（多一些 8 次 / 20 分钟、适中 5 / 30、少一些 2 / 60、关闭） | FR-CMF-06 | 完成（#27），新增设置键 |
| 反馈 👍 / 👎 / 🔕、模板句屏蔽、连续 3 天负反馈自动降档并告知 | FR-CMF-05、FR-CMF-06 第 2 条 | 完成（#29）：`domain::comfort_feedback`、命令 `comfort_feedback(id, verdict)`（ADR 0015 第 9 条，没有并进 `submit_feedback`）；🔕 到当天 24:00；降档最低到“少一些”，推 `care:reduced` 与 `settings:changed`。等 D 在一句话区悬停时接三个按钮 |
| E-COMFORT 评测（50 次生成、人工违规率 < 5%） | 08 第 8 节、C-11 | 脚本完成（[xinqing-ime#50](https://github.com/young8290/xinqing-ime/pull/50)）：`eval/tools/eval_comfort.py`；评测集 `eval/datasets/e_comfort.jsonl` 是状态摘要，不是用户原话；报告待真实接口预测文件 |

### 3.2 C-03 剩余

| 部分 | 需求 | 状态 |
|---|---|---|
| 每日预算上限设置 → `set_cap` | FR-AIG-07、10 第 6.2 节 | 完成（#44）：`ai.daily_caps` 拆成 `ai.cap.jev` / `llm` / `chat` / `schedule` / `rewrite` 五个整数键（ADR 0019 第 4 条）；外壳启动时和 `settings_set` 改了这些键时调 `Ai::apply_caps`，立即生效、今日用量不清零。设置页控件等 D（FR-SET-08）。`ai.llm_models` 仍随 `secrets_set` 存（ADR 0012） |
| 设置页“最近 20 次出网请求”（`Db::net_log_recent` 已有） | FR-SET-09 | 命令完成（[xinqing-ime#48](https://github.com/young8290/xinqing-ime/pull/48)）：`ai_net_log_recent`；设置页隐私分类里的展示等 D 接入（FR-SET-09 只显示字段名和时间） |
| 演示者视图的网关指标（`HttpGateway::metrics` / `models` 已有） | FR-AIG-08 | 未做：随 B-10 演示模式 |
| Jev 判断接进实时感知（`Sense`） | FR-STA-05 | **B 的任务**（B 交接文档 B-06）。外壳托管的 `Arc<Ai>` 实现 `AiGateway`，`sensing::start` 里 `app.state::<Arc<Ai>>()` 拿到后转成 `Arc<dyn AiGateway>` 传给 `Sense` 即可；顺带把 `need_comfort`、`valence` 填进 `MoodEvent::Sample` |

### 3.3 C-07 对话与 C-08 危机安全（对话部分）

| 部分 | 需求 | 状态 |
|---|---|---|
| 会话：超过 6 小时或“新对话”开新会话，标题取前 12 字，列表按最近消息排序 | FR-CHT-02 | 完成 |
| P-CHAT / P-CHAT-SAFE，`{style_block}`、`{today_summary_block}`（同意 ④）、`{memory_block}`（`memory` 最近 10 条 ≤ 300 字），最近 20 轮 ≈ 3000 字 | FR-CHT-03/05/10 | 完成；摘要算法见 ADR 0018 第 7 条 |
| 流式：`chat:delta` / `chat:done` / `chat:error`，停止生成、重试（`chat_retry`），单条 ≤ 2000 字 | FR-CHT-04/09 | 完成；首字 15 秒、总长 90 秒的超时在网关（`gateway/src/config.rs` 的 Chat 参数），服务另有 95 秒兜底 |
| 复制带“（内容由 AI 生成）”（`chat_copy`）、每条 `AI 生成` 标签 | FR-CHT-04 第 4、5 条 | 完成；导出对话（E-01）时也要加，那边还没做 |
| 双通道：本地词表同步、先于大模型；Q-CRISIS 与大模型同时进行；任一命中进入安全模式，推 `safety:triggered`、发布 `HubEvent::Safety`、打开对话窗口 | FR-SAF-01/02 | 完成；并行方式见 ADR 0018 第 1 条 |
| 安全模式：P-CHAT-SAFE，大模型不可用时本地固定回应；`safety_log` 只记时间和通道 | FR-SAF-03/04/05 | 完成 |
| 输出校验 V3（600 字截断）、V4（替换为 `chat.replaced`）、V6（替换为固定回应） | 08 第 5 节 | 完成；V4 替换句见 ADR 0018 第 2 条 |
| “我说的不是这个意思”：`safety_dismiss`，`safe_mode = 2`，阈值改 `threshold_after_dismiss` | FR-SAF-06 | 完成 |
| 快捷指令：吐槽 / 理一理是会话的对话方式（`chat_session.mode`，迁移 0003），`chat_set_mode`、`chat_send` 的 `mode`，提示词 `prompts/chat_vent.md`、`chat_organize.md` 接在 P-CHAT 后，`prompt_ver` 带上版本 | FR-CHT-06 | 完成：后端 [xinqing-ime#51](https://github.com/young8290/xinqing-ime/pull/51)，ADR 0022；对话窗口的三个按钮、当前方式提示与“回到平常聊天”、呼吸引导（`BreathingGuide.vue`）：[xinqing-ime#57](https://github.com/young8290/xinqing-ime/pull/57)；提示词待 E 评审；“写成情绪日记”随 C-10 |
| 记忆增删改、历史搜索、`chat.retention_days` 设置 | FR-CHT-07/08 | 命令完成（[xinqing-ime#49](https://github.com/young8290/xinqing-ime/pull/49)）：`memory_list/add/update/delete`（最多 50 条、每条 ≤ 100 字，满了返回 `chat.memory_full`）、`chat_search`（最多 50 条）；自动清理读取 30 / 90 / 365 天或永久。对话里明确说“记住”（`domain::chat::memory_request`，`ChatSent.memory_candidate`）或点消息上的“让晴晴记住”都先确认再写入；满了 / 超长有专门提示（`error.memory_full` / `error.memory_invalid`）：[xinqing-ime#57](https://github.com/young8290/xinqing-ime/pull/57) |
| E-CHAT 对话安全评测（12 的用例集 100% 通过） | FR-CHT-03 验收 | 未做：要真实接口 |

### 3.4 C-09 温柔改写（Hub 部分）

| 部分 | 需求 | 状态 |
|---|---|---|
| 收 `rewrite_req`、回 `rewrite_result` / `rewrite_fail`（原因：`no_consent` / `invalid` / `crisis` / `timeout` / `budget` / `offline`） | FR-RWR-03 | 完成；Hub 9.5 秒到点给 `timeout`，比核心的 10 秒早一点 |
| 可还原占位符 `[号码1]` `[证件1]` `[卡号1]` `[邮箱1]` `[链接1]`，结果由代码换回；原文与结果只在内存 | FR-RWR-07 | 完成（ADR 0028 第 3 条） |
| P-REWRITE、V1 解析、V8 保真（数字整体、`@` 前缀兼容、占位符、≤ 1.5 倍 + 10 字）、V4 只拦新增的禁用词、新增辱骂词 | FR-RWR-05 第 1、2 条，08 第 5 节 | 完成（ADR 0028 第 5 条）；辱骂词放在 `banned_words.toml` 第九类 `abuse`，**待 E 评审** |
| 危机内容不改写、不出网，记 `safety_log`，推 `safety:invite {source: "rewrite"}` | FR-RWR-05 第 3 条，E 在 ADR 0009 的阻塞项 | 后端完成（ADR 0028 第 4 条，E 的方案 ②）；**一句话区的求助入口与点击打开对话 + 求助卡片是 D 的界面** |
| `rewrite_log`（来源、风格、第几个、长度、结果，不含文字） | FR-RWR-04，09 D-28 | 完成；`chosen` 记 1～3（ADR 0028 第 6 条），排队写入 |
| 每种风格缓存 5 分钟 | FR-RWR-02 | 核心一侧已做（A-08），Hub 不重复 |
| 首次使用说明 | FR-RWR-06 第 3 条 | 未做：归属待 A、C、D 商定 |
| E-REWRITE 评测 | 08 第 8 节 | 未做：要真实接口 |

### 3.5 C-10 晚间小结（第一部分）

| 部分 | 需求 | 状态 |
|---|---|---|
| 到设置的时刻、当天活跃 ≥ 30 分钟、人在电脑前（2 分钟内有按键或上屏）才出；人不在就等到凌晨 3 点前的下一次输入；每天一次 | FR-REV-01 第 1 条 | 完成（ADR 0029 第 2 条）；内部键 `review.evening.shown_on` |
| 打字时长、喝水、休息完成 / 出现、完成日程、完成待办、天气色带 | FR-REV-01 第 2 条 | 完成（ADR 0029 第 3 条）；日程没有“完成”状态，按“已加入且已开始”计 |
| 结束语按主导状态从 `evening.toml` 选，不调 AI、不加标识；23:30 以后用 `late` 组 | FR-REV-01 第 3 条 | 完成（ADR 0029 第 4 条） |
| 设置 `review.evening.enabled`、`review.evening.time`（21:00–23:30） | FR-REV-01 第 4 条、FR-SET-05 | 完成；设置页控件按 `settings_schema` 出（D） |
| 卡片（“看看今天的看板”“今天不用了”） | FR-WGT-07 | **D 的界面**：收 `review:evening`（内容 `EveningSummary`），后端不需要按钮回调 |

### 3.6 C-10 周信（第二部分）

| 部分 | 需求 | 状态 |
|---|---|---|
| 周日 20:00 写那周的信；错过了在下一周写信时刻前补写；每周只处理一次（内部键 `review.letter.last_week`，删掉的信不重写） | FR-REV-02 第 1 条 | 完成（ADR 0030 第 2 条） |
| 有效天数（活跃 ≥ 30 分钟）< 3 天不写 | FR-REV-02 第 1 条 | 完成；记为处理过，那周不再看 |
| 统计 JSON：有效天、日均打字、状态占比、最累 / 最轻松时段、休息完成率、喝水、日程 / 待办、停止打字时间（周一到周六晚）、自评次数 | FR-REV-02 第 2 条 | 完成（ADR 0030 第 3 条）；只有数字和时段名，出网的只有它 |
| 同意 ④ 时 P-LETTER；长度 150–300、V4、V9，不过重写 1 次；网关出错直接用模板 | FR-REV-02、08 V9 | 完成（ADR 0030 第 4 条）；V9 = 每段阿拉伯数字都在统计里出现，中文数字不查 |
| 兜底模板：缺数据整句去掉；评语与建议在新文件 `letter_tips.toml` | FR-REV-02 第 3 条 | 完成；**`letter_tips.toml` 是新内容模板，待 E 评审** |
| 存 `letter`（`write_sync`）、推 `letter:new {id, week_start, ai_generated}`；命令 `letters_list` / `letter_read` / `letter_delete`；设置 `review.letter.enabled` | 09 D-26、10 第 5 节 | 完成（ADR 0030 第 6～8 条） |
| 卡片提示、信纸页面、历史列表、`AI 生成` 标签 | FR-WGT-07 | **D 的界面**：收 `letter:new`，用 `letters_list` 取正文，打开时调 `letter_read` |
| E-LETTER 评测 | 08 第 8 节 | 未做：要真实接口 |

## 4. 关键决定与待评审

- **ADR 0030（周信的实现解释，9 条）**：提议，待 E（文案、校验口径、新模板 `letter_tips.toml`）、D（`letter:new` 与信件界面）评审，B 知会。
  要点：错过了在下一周写信时刻前补写，每周只处理一次（内部键，删掉的不重写）；有效天数不足跳过；统计 JSON 的字段与时段划分；
  作息只看周一到周六晚；长度 150–300、V9 按连续数字比对、不过重写 1 次；新事件 `letter:new`、三个命令、设置键 `review.letter.enabled`。

- **ADR 0029（晚间小结的实现解释，6 条）**：提议，待 D（`review:evening` 与卡片）、E（文案与验收）评审，B 知会。要点：C 先认领 C-10；
  “在电脑前”= 2 分钟内有按键或上屏；以凌晨 3 点为界；完成日程 = 已加入且已开始；主导状态占比 ≥ 40%，`late` 组按出小结的时刻；
  新增 `review.evening.*` 两个设置键和内部键 `review.evening.shown_on`。

- **ADR 0028（温柔改写 Hub 服务的实现解释，9 条）**：提议，待 E（隐私、危机安全、禁用词表第九类）、D（`safety:invite` 与界面）评审，A 知会。
  要点：Hub 不另做缓存（核心已做）；占位符种类与写法；危机只用本地词表、不出网、推 `safety:invite`，界面在一句话区给求助入口（E 的方案 ②），
  不自动弹出对话窗口；V8 的比较口径；V4 只拦新增的词；`chosen` 从 1 数；去掉 `sense.rs` 的 `offline` 占位；顺带收紧网关脱敏的查询参数规则
  （原规则会吞掉任何半角问号后的文字）；mock-ai 认 P-REWRITE。

- **ADR 0020（单写入口 DbWriter 的实现方式，6 条）**：提议，待 B、D、E 评审。要点：一个写连接 + 一个只读连接；写连接用互斥锁串行，
  **没有照 17 字面做写线程 + 通道**（领域函数都收 `&Db` 且读写交错，走通道要改写几十处，保证的性质相同）；普通数据（特征窗口与状态记录、
  出网日志、提醒记录）排队，满 100 条或排满 5 秒批量提交，每条一个保存点；用户确认类数据 `write_sync`，先提交队列保证先后顺序；
  读连接 debug 下 `query_only`；退出时提交队列。**给 B 的接口**：感知任务以后新加写库都经 `AppState::writer()`。

- **ADR 0019（设置值的列表与格式类型，8 条）**：提议，待 D（`bindings.ts`、设置页）、E（隐私、导出导入）评审，B 知会。要点：
  `SettingValue` 加字符串列表（前端 `string[]`），**整份读写**，出厂列表用户可清空；`Kind` 加 `Int`、`Text(Item)`、`List { item, max }`，
  文字只收已知格式（进程名、时段、电话），因为电话会进豁免禁用词的求助卡片；`ai.daily_caps` 拆成 `ai.cap.*`；安静时段只管主动关怀；
  系统专注助手不判断。**给 D 的接口**：`care.dnd_apps`（`string[]`，进程名）、`care.quiet_hours`（`string[]`，`"HH:MM-HH:MM"`）、
  `safety.school_phone`（`string`，空串 = 没填；求助卡片 `SafetyCard.vue` 现在固定显示“可在设置中添加”，读这个键即可）、
  `ai.cap.*`（整数），都走现有的 `settings_get` / `settings_set` / `settings:changed`，不合法返回 `settings.out_of_range`。

- **ADR 0018（AI 对话与对话中危机安全的实现解释，8 条）**：提议，待 D、E 评审。要点：Q-CRISIS 与大模型并行、只有 Jev 命中时从下一条起进入安全模式；V4 替换句 `chat.replaced`；`safe_mode` 用 0/1/2；新增 `chat_retry`、`chat_delete_all`、`safety_dismiss`；中途出错丢弃半截、停止保留已生成部分。
- **ADR 0015（暖心话的实现解释，12 条）**：#27 提出前 8 条，#29 补第 9–12 条（反馈命令、🔕 时限、降档计法、内部键），待 B、D、E 评审。要点：`comfort_log` 加 `trigger` 列，自评回应不占每日名额；
  降级时不主动关怀；持续按窗口数计；时段与分组、模板放宽顺序；重试时限 5 秒；勿扰先做全屏与默认应用；工具栏小圆点；新增 `care.level`。
  迁移 0002 是数据库契约改动，按 13 第 3.1 节本应单独提 PR；本会话只能推一个分支，所以放在同一个 PR 的独立提交里，请评审时单独看。
- **ADR 0012（AI 服务配置的存放与读取，8 条）**：提议，待 D（界面）与 E（隐私）评审。要点：`secrets.bin` 存整份配置（地址 + 密钥，TOML 后整份 DPAPI）；
  新增 `ai_config_get`；`secrets_set` 里空密钥沿用旧值；配置来源优先级 dev `secrets.toml` → `secrets.bin` → dev 连 mock-ai → 离线；
  非 Windows 不保存密钥；换配置不清零今日预算。接受后要改产品书 10 第 5.1 节、08 FR-AIG-06、14 第 4 节。
- **接手 #7 的方式**：没有改 Hu-yiye 的分支，而是在 #21 里合入它（保留其提交），在上面修编译错误并补完。#21 已合并，#7 的提交都在 main 里，可以关闭。
- **“测试连接”只测大模型**：Jev 没有不耗额度的探测接口，它的可用性看 `gateway:health` 和出网日志（ADR 0012 第 8 条）。

## 5. 已知问题

- 改写的 V8 不检查中文数字（“三点”“两千”）；`@某人` 只能按前缀判断同一个人（ADR 0028 第 5 条）。
- 改写的请求信息（来源、风格、长度）在内存里等 `rewrite_done`，Hub 在这期间重启，那次改写就不写 `rewrite_log`。
- 改写服务的模板加载失败时不启动，核心等 10 秒后提示“暂时改写不了”。
- `secrets_set` 的明文密钥经 Tauri IPC 的 JSON 反序列化进来，那一份中间缓冲清不掉；进入 Rust 后立即放进 `Zeroizing`。
- 熔断打开期结束后不会自己发健康变化，要等下一次请求或每 30 分钟的模型刷新（网关 README 也写了）；离线角标因此最多晚 30 分钟消失。
- 健康状态“离线”= 任一侧不可用（`GatewayHealth::offline`）。只配了大模型、没配 Jev 时也显示离线角标，符合 03 第 8 节“L2 Jev 不可用”的降级，但用户可能疑惑，设置页要说明。
- 写库一律经 `AppState::writer()`（ADR 0020）：用户确认类数据 `write_sync`，普通数据 `enqueue`（最多晚 5 秒才能读到）。`AppState::db()` 是只读连接，debug 构建下写入会报错；新加写库的地方别用它。
- 排队的普通数据在进程被强制结束时最多丢最近 5 秒；正常退出时 `RunEvent::Exit` 会提交。
- 对话服务读写交错，整段用写连接（`DbWriter::lock()`），持有期间别的写入要等；每段都很短（几条查询），没有跨 `.await`。
- `ai_usage_today` 只列网关自己计数的 Jev、大模型、对话、改写四类；日程初筛（C-05）由它的服务计数，做完再加；周信（C-10）计在“大模型”里。
- 暖心话目前只有负面自评会真正说出来：主动关怀等 Jev 接进 `Sense`（B）。
- 休息提醒（B-08）的勿扰现在只看前台全屏，没有读 `care.dnd_apps`；ADR 0019 第 6 条建议共用这个列表，`domain::dnd::is_dnd_app` 可直接用，等 B 决定。
- 设置值列表的重复按 ASCII 大小写判断：`Zoom.exe` 与 `ZOOM.EXE` 算重复，非 ASCII 的进程名（如 `企业微信.exe`）大小写不折叠，Windows 上也基本不会出现。
- 暖心话服务按总线顺序处理事件，生成一句话（最长约 8 秒）期间到来的窗口排队，总线容量 1024，不会丢；只影响触发时刻的计数，不影响结果。
- 🔕 与降档时间存在 `settings` 表的内部键（`care.muted_until`、`care.reduced_ts`），登记在 `settings::INTERNAL_KEYS`；E 做导出导入时只处理 `KEYS` 里的键（ADR 0015 第 12 条）。B 的 `baseline.reset_ts`（#25）合并后也要登记进去。
- 降档按“那句暖心话的日期”归天：今天才给昨天的暖心话点 👎，算昨天的反馈。
- 模板只读出厂目录（与 `sensing.rs` 一致）；10 第 7 节说的用户目录 `%LOCALAPPDATA%\XinQing\hub\templates\` 覆盖还没接，接时外壳统一改 `TemplateDirs`。
- 大模型返回的 `kind`（comfort / rest / cheer）只用于校验，没有落库，界面也没用到。
- 周信的 V9 只保证数字有出处，挡不住“把 3 天说成 3 小时”这类张冠李戴（ADR 0030 第 4 条），靠 E-LETTER 评测兜底。
- 周信每周最多调用 2 次大模型，计入总预算，没有单独的上限；`ai_usage_today` 没有单列周信。

## 6. 下一步（按优先级）

1. C-10 第三部分：情绪日记（FR-DIA-01～03，P-DIARY、日记里的危机识别、“写成情绪日记”）；跟进 ADR 0029、0030 的评审（D、E），`letter_tips.toml` 要 E 看；
2. 跟进 ADR 0028 的评审（E、D）；D 做一句话区的求助入口时按 `safety:invite` 接；与 A、D 定首次使用说明放哪；
3. 跟进 ADR 0019 的评审（D 已同意，待 E）；D 做设置页“关怀”“AI 服务”分类时用 `settings_schema` 出控件，如需改形状在本 ADR 上改；
4. 跟进 ADR 0020 的评审（B、D、E）；
5. C-07 剩余：设置页“晴晴记住的事”与历史搜索的界面（D）；日记的危机识别与“写成情绪日记”随 C-10；
6. C-05 日程剩余（P0，W7–W8）：本地初筛、存储、抽取结果校验（含代码日期校验）都有了，还差 L2/L3 的网关接线、提醒调度 `scheduler` 与冲突处理。

## 7. 修订记录

| 日期 | 改动 |
|---|---|
| 2026-10-04 | 初版：盘点 C-01～C-11 现状；C-03 第二部分（接手 #7 的网关接线，密钥 DPAPI 保存，AI 服务命令与 `gateway:health`，ADR 0012） |
| 2026-10-04 | C-03 已合并（#21）；C-04 第一部分：暖心话主动关怀与自评回应、迁移 0002、`care.level`、`comfort:new`、ADR 0015 |
| 2026-10-04 | C-04 第一部分已合并（#27）；C-04 第二部分：暖心话反馈（👍 / 👎 / 🔕）、自动降档、`care:reduced`、内部键 `INTERNAL_KEYS`，ADR 0015 补第 9–12 条 |
| 2026-10-05 | C-07 对话（P0 部分）与 C-08 危机安全的对话部分、D-06 对话窗口第一版，ADR 0018 |
| 2026-10-05 | C-07/C-08 已合并（#40）；设置值扩展（列表、整数、格式文本）、`care.dnd_apps` / `care.quiet_hours` 接进暖心话、`safety.school_phone`、`ai.cap.*` 接进网关，ADR 0019 |
| 2026-10-05 | #44 已合并；单写入口 `DbWriter`（一个写连接 + 只读连接，普通数据 5 秒 / 100 条批量，用户确认类同步写），ADR 0020 |
| 2026-10-05 | C-11 补 E-COMFORT 评测集（6 种状态摘要）、结构/长度/禁用词/人工违规率脚本与单测（#50） |
| 2026-10-05 | C-07 快捷指令：吐槽 / 理一理做成会话的对话方式（迁移 0003、`chat_set_mode`、两段新提示词），ADR 0022（#51） |
| 2026-10-05 | C-05 FR-SCH-04 校验第 1 条（`domain/when.rs`）、标题截断与 V4 口径（ADR 0024）；C-07 “记住”先确认、记忆的专门提示、快捷指令按钮与呼吸引导（#57） |
| 2026-10-05 | #54 已合并；按 D 在 ADR 0019 评审中的提议加只读命令 `settings_schema`（`SettingField` / `SettingFieldKind` / `SettingItem`），ADR 0019 第 9 条 |
| 2026-10-05 | #62 已合并；C-09 温柔改写的 Hub 服务（占位符、V8、危机入口事件 `safety:invite`、`rewrite_log`）、网关脱敏查询参数规则收紧、禁用词表第九类 `abuse`，ADR 0028 |
| 2026-10-05 | #65 已合并；认领 C-10，第一部分晚间小结（`review:evening`、`review.evening.*`），ADR 0029 |
| 2026-10-06 | #68 已合并；C-10 第二部分周信（`letter:new`、`letters_list` / `letter_read` / `letter_delete`、`review.letter.enabled`、新模板 `letter_tips.toml`、mock-ai 认 P-LETTER），ADR 0030 |
