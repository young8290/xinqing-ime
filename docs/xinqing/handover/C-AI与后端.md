# C AI 与后端 · 交接文档

> 负责范围（产品书 13 第 1 节）：AIG 网关、CMF 暖心话、CHT 对话、DIA 日记、SAF 危机安全、SCH 日程与待办、RWR 温柔改写的 Hub 部分、REV-01/02 晚间小结与周信、评测脚本；
> 另是数据库迁移（`xinqing_hub/core/migrations/`）与设置键注册表（`xinqing_hub/core/src/domain/settings.rs`）的契约负责人（13 第 3.1 节）。
> 任务清单与估算见产品书 16 第 2.3 节。本文件随 C 的每个 PR 更新，任务中途换人时按产品书 13 第 3.1 节“交接”直接看这里。
> 最后更新：2026-10-05（设置值扩展：列表与格式类型，安静时段与自定义勿扰应用，`ai.cap.*`，ADR 0019）

## 1. 任务状态

| 编号 | 任务 | 状态 | 代码 / PR | 还差什么 |
|---|---|---|---|---|
| C-01 | Hub 骨架：分层、事件总线、SQLite 迁移 v1、单写线程、settings | 大部分完成 | `core/src/bus.rs`、`core/migrations/`（0001、0002）、`core/src/infra/store/`、`core/src/domain/settings.rs`（设置键 `KEYS`、内部键 `INTERNAL_KEYS`）；设置值的列表与格式类型：ADR 0019 | 单写线程 `DbWriter`（17 第 2.9 节）没做，外壳现在用 `Mutex<Db>` 串行；设置键已登记 27 个，10 第 6.2 节其余的（`chat.retention_days`、`review.*`、`sch.*` 等）随各功能补；设置页若要“键注册表 → schema”的命令，等 D 提 |
| C-02 | mock-ai（Jev 三类、OpenAI 兼容含 SSE、各故障场景） | 完成 | `tools/mock-ai/` | 16 第 3 节建议移给 E，W1 评审会还没定 |
| C-03 | AI 网关：trait、Jev/LLM 客户端、熔断、重试、健康检查、隐私过滤、密钥 DPAPI、预算 | 主体完成 | HTTP 实现：`xinqing_hub/gateway/`；trait、熔断、预算、脱敏：`core/src/infra/gateway/`；接入外壳：[xinqing-ime#21](https://github.com/young8290/xinqing-ime/pull/21)（已合并，含 Hu-yiye 的 [#7](https://github.com/young8290/xinqing-ime/pull/7)），ADR 0012 | 见第 3.2 节 |
| C-04 | 暖心话：触发、生成、校验、模板兜底、频率控制、反馈 | **后端完成** | 第一部分（主动关怀与自评回应）：[xinqing-ime#27](https://github.com/young8290/xinqing-ime/pull/27)（已合并）；第二部分（反馈与自动降档）：[xinqing-ime#29](https://github.com/young8290/xinqing-ime/pull/29)（已合并）；第三部分（安静时段、自定义勿扰应用）：[xinqing-ime#44](https://github.com/young8290/xinqing-ime/pull/44)；ADR 0015、0019 | 见第 3.1 节：主动关怀要等 B 把 Jev 接进 `Sense`；系统专注助手不判断（ADR 0019 第 7 条）；E-COMFORT 评测 |
| C-05 | 日程识别：L1/L2/L3、代码校验、去重、提醒调度、冲突 | 本地初筛与结构化存储完成 | [xinqing-ime#42](https://github.com/young8290/xinqing-ime/pull/42)（已合并）：`core/src/domain/schedule.rs`、`core/src/infra/store/schedule.rs`；模板 `hub_templates/schedule_patterns.toml`、评测集 `eval/datasets/e_plan.jsonl`、`e_extract.jsonl` | L2/L3 抽取、字段校验、提醒调度与冲突处理待做；提醒调度 `scheduler` 也给 B-04（04:00 重算基线）用 |
| C-06 | 待办识别与清单、提醒 | 本地初筛与结构化存储完成 | #42（已合并）：`core/src/domain/schedule.rs`、`core/src/infra/store/schedule.rs`；`prompts/todo.md`、评测集 `eval/datasets/e_todo.jsonl` | AI 抽取、提醒调度与前端清单待做（P1） |
| C-07 | AI 对话：会话、上下文、流式、记忆、历史、快捷指令 | **P0 部分完成** | [xinqing-ime#40](https://github.com/young8290/xinqing-ime/pull/40)（已合并）：纯函数 `core/src/domain/chat.rs`、服务 `core/src/chat.rs`、读写 `infra/store/chat.rs`、外壳 `src-tauri/src/chat.rs` 与 `commands/chat.rs`；`gateway/tests/chat_mock_ai.rs`；ADR 0018 | 见第 3.3 节：快捷指令、记忆增删改、历史搜索与保留期设置（P1） |
| C-08 | 危机安全：双通道、求助卡片、安全模式、记录、误报处理 | **对话部分完成** | #40（已合并）：本地通道 `core/src/domain/safety.rs`（与 Python 对拍）；对话里的双通道、安全模式、固定回应、`safety_log`、V6、“我说的不是这个意思”在 `core/src/chat.rs`；ADR 0018 | 日记中的危机识别随 C-10；12 的对话安全用例集要真实接口跑 |
| C-09 | 温柔改写 Hub 服务：脱敏、P-REWRITE、保真校验、缓存 | 未开始 | `prompts/rewrite.md`、`eval/datasets/e_rewrite.jsonl`；`sense.rs` 收到 `rewrite_req` 时先回 `RewriteFail::Offline` | 计划 W8–W9；依赖 A-08 改写模式；V8 校验、可还原占位符 `[号码1]` |
| C-10 | 情绪日记、晚间小结、周信 | 未开始 | `hub_templates/evening.toml`、`letter_fallback.md`、`prompts/diary.md`、`letter.md` | 16 第 3 节建议移给 B，W1 评审会还没定（B 的交接文档也记了“未认领”） |
| C-11 | 评测脚本与报告 | 脚本完成，报告未出 | `eval/tools/`（[xinqing-ime#6](https://github.com/young8290/xinqing-ime/pull/6)，Hu-yiye）：日程 / 抽取 / 待办、对话安全 / 改写、周信、危机词表、E-COMFORT | E-COMFORT 脚本与 50 条虚构评测集已补；报告（`eval/reports/`）仍要等真实接口生成预测文件 |

## 2. 代码地图（C 负责的部分）

```
xinqing_hub/
├─ core/src/infra/gateway/   AiGateway trait 与类型、问题目录与字段白名单、熔断 breaker.rs、预算 budget.rs、脱敏 privacy.rs、离线网关
├─ core/src/domain/
│  ├─ schedule.rs            日程/待办逐句本地初筛，原句只在内存候选中保留
│  ├─ comfort.rs             暖心话：频率档位、触发判断 Trigger、状态摘要、P-COMFORT 拼装、V1–V7 校验、模板选句
│  ├─ validate.rs            08 第 5 节输出校验（V1、V3 计数、V4 禁用词、V5 重复、V7 语言；V2/V6/V8/V9 随功能补）
│  ├─ safety.rs              危机词表本地通道
│  ├─ dnd.rs                 勿扰应用与安静时段的解析、判断（暖心话用，休息提醒也可用）
│  └─ settings.rs            设置键注册表（契约）：值的四种形态与格式校验（ADR 0019）、`cap_key` / `caps`
├─ core/src/care.rs          暖心话服务 ComfortService：订阅总线 → 触发 → 大模型（重试 1 次）或模板 → comfort_log → ComfortPort::show
├─ core/src/infra/store/     Db：迁移、窗口与状态、反馈、net_log（只留最近 200 条）；comfort.rs 是 comfort_log 的读写；schedule.rs 写结构化日程/待办并去重
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
hub_templates/               comfort.toml、prompts/*.md、crisis_lexicon.toml、banned_words.toml、schedule_patterns.toml、evening.toml、letter_fallback.md
tools/mock-ai/               模拟 Jev 与大模型（normal/slow/timeout/500/422/model_not_found/stream_cut）；P-COMFORT 请求回合法 JSON
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
| E-COMFORT 评测（50 次生成、人工违规率 < 5%） | 08 第 8 节、C-11 | 脚本完成：`eval/tools/eval_comfort.py`；评测集 `eval/datasets/e_comfort.jsonl`；报告待真实接口预测文件 |

### 3.2 C-03 剩余

| 部分 | 需求 | 状态 |
|---|---|---|
| 每日预算上限设置 → `set_cap` | FR-AIG-07、10 第 6.2 节 | 完成（#44）：`ai.daily_caps` 拆成 `ai.cap.jev` / `llm` / `chat` / `schedule` / `rewrite` 五个整数键（ADR 0019 第 4 条）；外壳启动时和 `settings_set` 改了这些键时调 `Ai::apply_caps`，立即生效、今日用量不清零。设置页控件等 D（FR-SET-08）。`ai.llm_models` 仍随 `secrets_set` 存（ADR 0012） |
| 设置页“最近 20 次出网请求”（`Db::net_log_recent` 已有） | FR-SET-09 | 未做：要一个命令，10 第 5.1 节没列，和 D 的设置页隐私分类一起加 |
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
| 快捷指令、记忆增删改、历史搜索、`chat.retention_days` 设置 | FR-CHT-06/07/08 | 未做（P1） |
| E-CHAT 对话安全评测（12 的用例集 100% 通过） | FR-CHT-03 验收 | 未做：要真实接口 |

## 4. 关键决定与待评审

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

- `secrets_set` 的明文密钥经 Tauri IPC 的 JSON 反序列化进来，那一份中间缓冲清不掉；进入 Rust 后立即放进 `Zeroizing`。
- 熔断打开期结束后不会自己发健康变化，要等下一次请求或每 30 分钟的模型刷新（网关 README 也写了）；离线角标因此最多晚 30 分钟消失。
- 健康状态“离线”= 任一侧不可用（`GatewayHealth::offline`）。只配了大模型、没配 Jev 时也显示离线角标，符合 03 第 8 节“L2 Jev 不可用”的降级，但用户可能疑惑，设置页要说明。
- 出网日志写库走 `Mutex<Db>`，和感知任务抢同一把锁；`DbWriter`（C-01 剩余）接入后改走单写线程。
- `ai_usage_today` 只列网关自己计数的 Jev、大模型、对话、改写四类；日程初筛（C-05）和周信（C-10）由各自服务计数，做完再加。
- 暖心话目前只有负面自评会真正说出来：主动关怀等 Jev 接进 `Sense`（B）。
- 休息提醒（B-08）的勿扰现在只看前台全屏，没有读 `care.dnd_apps`；ADR 0019 第 6 条建议共用这个列表，`domain::dnd::is_dnd_app` 可直接用，等 B 决定。
- 设置值列表的重复按 ASCII 大小写判断：`Zoom.exe` 与 `ZOOM.EXE` 算重复，非 ASCII 的进程名（如 `企业微信.exe`）大小写不折叠，Windows 上也基本不会出现。
- 暖心话服务按总线顺序处理事件，生成一句话（最长约 8 秒）期间到来的窗口排队，总线容量 1024，不会丢；只影响触发时刻的计数，不影响结果。
- 🔕 与降档时间存在 `settings` 表的内部键（`care.muted_until`、`care.reduced_ts`），登记在 `settings::INTERNAL_KEYS`；E 做导出导入时只处理 `KEYS` 里的键（ADR 0015 第 12 条）。B 的 `baseline.reset_ts`（#25）合并后也要登记进去。
- 降档按“那句暖心话的日期”归天：今天才给昨天的暖心话点 👎，算昨天的反馈。
- 模板只读出厂目录（与 `sensing.rs` 一致）；10 第 7 节说的用户目录 `%LOCALAPPDATA%\XinQing\hub\templates\` 覆盖还没接，接时外壳统一改 `TemplateDirs`。
- 大模型返回的 `kind`（comfort / rest / cheer）只用于校验，没有落库，界面也没用到。

## 6. 下一步（按优先级）

1. 跟进 ADR 0019 的评审（D、E）；D 做设置页“关怀”“AI 服务”分类时如需改形状，在本 ADR 上改；
2. C-01 剩余：`DbWriter` 单写线程；
3. C-07 剩余的 P1（快捷指令、记忆、历史搜索）；日记的危机识别随 C-10；
4. C-05 日程剩余（P0，W7–W8）：#42 做了本地初筛与存储，还差 L2/L3 抽取、字段校验、提醒调度 `scheduler` 与冲突处理。

## 7. 修订记录

| 日期 | 改动 |
|---|---|
| 2026-10-04 | 初版：盘点 C-01～C-11 现状；C-03 第二部分（接手 #7 的网关接线，密钥 DPAPI 保存，AI 服务命令与 `gateway:health`，ADR 0012） |
| 2026-10-04 | C-03 已合并（#21）；C-04 第一部分：暖心话主动关怀与自评回应、迁移 0002、`care.level`、`comfort:new`、ADR 0015 |
| 2026-10-04 | C-04 第一部分已合并（#27）；C-04 第二部分：暖心话反馈（👍 / 👎 / 🔕）、自动降档、`care:reduced`、内部键 `INTERNAL_KEYS`，ADR 0015 补第 9–12 条 |
| 2026-10-05 | C-07 对话（P0 部分）与 C-08 危机安全的对话部分、D-06 对话窗口第一版，ADR 0018 |
| 2026-10-05 | C-07/C-08 已合并（#40）；设置值扩展（列表、整数、格式文本）、`care.dnd_apps` / `care.quiet_hours` 接进暖心话、`safety.school_phone`、`ai.cap.*` 接进网关，ADR 0019 |
| 2026-10-05 | C-11 补 E-COMFORT 评测集、结构/长度/禁用词/人工违规率脚本与单测 |
