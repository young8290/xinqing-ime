# E 产品与质量（兼项目经理）· 交接文档

> 负责范围（产品书 13 第 1 节）：产品书、DAT（含导入）、隐私、测试、用户研究；兼项目经理（例会、风险登记、评审组织、看板）。
> RACI 中 E 为 A 的事项：产品书维护、隐私合规、测试与验收（A）、危机安全（评审 A）、导入与恢复。
> 任务清单与估算见产品书 16 第 2.5 节。本文件随 E 的每个 PR 更新，任务中途换人时按产品书 13 第 3.1 节“交接”直接看这里。
> 最后更新：2026-10-04（E-01 第二部分：导出数据包，core 部分）

## 1. 任务状态

| 编号 | 任务 | 状态 | 代码 / PR | 还差什么 |
|---|---|---|---|---|
| PM-01 | 产品书评审定稿、分工确认、看板建立与任务导入 | **未完成（需要人）** | — | W1 评审会没有记录；16 第 5 节人员指派表是空的；GitHub Projects 看板未建。看板任务可按 16 第 2 节编号批量导入，要人来建 |
| PM-02 | 每周例会、风险登记更新、周报 | 未开始（需要人） | 模板：16 第 6.1 节 | 纪要放 `docs/meetings/YYYY-MM-DD.md`（目录还没有）；风险状态见第 4.3 节 |
| PM-03 | 需求、设计、测试评审各一次 | 未开始 | 模板：16 第 6.2 节 | 计划 W2 / W4 / W6；ADR 评审在做（第 4.1 节），不替代正式评审会 |
| PM-04 | 一手调研：问卷与访谈 | 未开始（需要人） | 问卷与提纲：12 第 8 节 | 要真人做，Claude 只能帮整理结果 |
| E-01 | 数据导出、删除、撤回同意、自动清理 | **进行中** | 第一部分（自动清理）：[xinqing-ime#20](https://github.com/young8290/xinqing-ime/pull/20)（已合并），ADR 0013；第二部分（导出，core）：[xinqing-ime#32](https://github.com/young8290/xinqing-ime/pull/32)；Tauri `data_export(path)` 命令：本 PR | 见第 3 节 |
| E-02 | 数据导入与恢复（合并 / 覆盖、事务回滚） | 未开始 | — | 计划 W10，依赖 E-01 的导出格式 |
| E-03 | db-grep、隐私自动化测试（NFR-PRI） | 未开始 | — | 计划 W8。已有的隐私约束测试：`window_features_reject_text`（store）、Q-STATE 字段白名单与脱敏（`core/src/infra/gateway`） |
| E-04 | 测试用例维护与执行（18 全部用例）、缺陷管理 | 未开始 | — | 计划 W6–W12；目前各 PR 的单元测试按需求编号写，还没人按 TC 编号执行和记录 |
| E-05 | 用户研究（招募、知情同意、7 天试用、分析） | 未开始（需要人） | 知情同意书：21 第 2 节 | 计划 W8 招募（R-11） |
| E-06 | 隐私说明、用户手册、运维手册定稿 | 未开始 | 初稿：21、19、20 | 计划 W10，功能冻结后按实现核对 |
| E-07 | 测试报告、验收演示材料与演练 | 未开始 | 模板：18 第 4 节 | 计划 W12，`docs/test/report.md` |
| E-08 | 内容资产评审：危机词表、禁用词表、文案 | **E 已评审，v2 已合并，待第二评审人** | 评审记录 `docs/xinqing/reviews/2026-10-04-E-08-内容资产评审.md`；词表 v2、禁用词表 v2、扩充集 `eval/datasets/e_crisis_ext.jsonl`：[xinqing-ime#28](https://github.com/young8290/xinqing-ime/pull/28)，产品书 [xinqing#8](https://github.com/young8290/xinqing/pull/8) | 第二评审人确认前两份文件仍不得用于试用；真正的留出集要由没看过词表的人写；`ui_copy.toml` 只评了 `[safety]`（通过），其余待 D 与 E 共同评审 |
| （C-02） | mock-ai | 已由 C 完成 | `tools/mock-ai/` | 16 第 3 节建议移给 E，已经做完，不再移；后续加故障场景由测试驱动，E 提需求 |

## 2. 代码地图（E 负责的部分）

```
xinqing_hub/core/src/
├─ domain/retention.rs     FR-DAT-02 保留与自动清理：RULES 一条规则对应 09 第 1 节一行，run / next_run / Report
├─ infra/store/export.rs   FR-DAT-03 导出：TABLES 每张表的数据编号与说明（新增表必须登记，测试会查）、Manifest / FileEntry（导入 E-02 复用）
├─ domain/consent.rs       六项单独同意（C 写的；撤回同意 FR-DAT-05 要在这里补“撤回后的动作”）
└─ infra/store/mod.rs      Db::init 对新库开 auto_vacuum = INCREMENTAL（清理后回收空间）
xinqing_hub/src-tauri/src/
├─ cleanup.rs              启动 5 分钟后补跑一次，之后每天 04:30（ADR 0013 第 1 条）
└─ state.rs                open_or_rebuild：数据库损坏时备份为 .corrupt-<时间> 并重建（FR-DAT-01，C 写的）
docs/adr/0013-…            数据保留清理的实现解释
docs/xinqing/reviews/      评审记录（E-08 内容资产评审）
eval/datasets/e_crisis_ext.jsonl  危机词表扩充集（E-08 补充的开发集，45 正 / 45 负）+ e_crisis_ext.golden.json
```

验证：`cargo test -p xinqing-hub-core retention`（TC-DAT-08 口径：每张表一条刚过期、一条没过期的数据）。
危机词表：`python3 eval/tools/check_crisis.py`（E-CRISIS）和 `python3 eval/tools/check_crisis.py hub_templates/crisis_lexicon.toml eval/datasets/e_crisis_ext.jsonl`（扩充集），
改了词表要先跑 `python3 tools/gen_crisis_t2s.py`（新字进繁转简表，CI 的 `--check` 会查，需要 `pip install opencc-python-reimplemented`），再用 `--golden` 重新生成两份对拍文件，`cargo test -p xinqing-hub-core --test crisis_parity` 检查 Rust 与 Python 一致。

## 3. 进行中：E-01

| 部分 | 需求 | 状态 |
|---|---|---|
| 本地存储、数据库损坏后备份并重建 | FR-DAT-01、TC-DAT-07 | 完成（C，`state.rs`，有测试）；界面提示“已为你重建”等 D-02 一句话区 |
| 自动清理：按 09 第 1 节保留期删除、增量 VACUUM、失败只记日志 | FR-DAT-02、TC-DAT-08 | 完成（#20）。对话保留期暂用默认 90 天：`chat.retention_days` 还没登记（设置键注册表负责人 C），登记后在 `cleanup.rs` 的 `run_once` 里读 |
| 导出：zip，每表一个 JSON + 字段说明 + `manifest.json`（版本、SHA-256），AI 文本带标识，不导出密钥 | FR-DAT-03、TC-DAT-01、TC-PERF-11 | core 完成（#32，`infra/store/export.rs`）；本 PR 接入 Tauri `data_export(path)` 并生成 `bindings.ts`；同一读事务、`.part` 原子替换和性能测试保持不变。**还差** 设置页按钮（D-08） |
| 删除全部数据：输入“删除”确认，删库重建、删日志 Hub 部分、可选删密钥，回到引导 | FR-DAT-04、TC-DAT-02、NFR-PRI-06 | 未做。要在外壳里关闭连接 → 删 `xinqing.db*` → 重建 → 重新打开引导窗口；密钥已由 C 存进 `hub\secrets.bin`（ADR 0012），“含密钥”时删这一个文件 |
| 撤回同意：逐项撤回立即生效，② ③ ⑤ 提示“已发送的数据无法撤回”，撤回 ① 询问是否删情绪数据 | FR-DAT-05、TC-DAT-03、NFR-PRI-05 | 未做。`consent_set` 已能写撤回；要补：撤回后重新下发 XQP `cfg`、通知网关停止 Jev、撤回 ① 时删 D-06～D-10、D-16～D-18 |

## 4. 项目经理视角

### 4.1 评审队列（E 要看的）

| 对象 | 状态 | 说明 |
|---|---|---|
| ADR 0008 状态识别规格的解释 | **已接受**（B、E） | 已同步到产品书 V1.4（[xinqing#7](https://github.com/young8290/xinqing/pull/7)，04）。第 5 条 R1b 的协议字段等 A 决定 |
| ADR 0010 状态解释的信号挑选 | **已接受**（B、D、E） | 已同步到产品书 V1.4（04、15） |
| ADR 0011 自评天气的实现解释（B） | 提议 | **E 同意**（意见写在 ADR“评审”里）；还差 D。接受后改 04 FR-STA-10 第 1、3 条，10 第 5.3 节 |
| ADR 0012 AI 服务配置的存放与读取（C） | 提议 | **E 修改后同意**，两处要 C 改：release 构建只接受 `https://`（回环地址除外，09 SEC-02）；配置解析失败的错误信息不带原文（`toml` 错误会引用出错行，可能把密钥写进日志）。还差 D |
| ADR 0013 数据保留清理 | 提议（E） | 等 C 评审；接受后改 09 FR-DAT-02、D-11/D-12/D-25，10 第 6.2 节登记 `chat.retention_days` |
| ADR 0007 Hub 核心与外壳分离 | 提议 | 等 W1 评审会。产品书 13、09、10、20 的路径已按它写，17 第 2 节的目录图还是旧的（`src-tauri/src/domain`），接受后一起改 |
| ADR 0009 核心侧 XQP 服务端（A） | 提议 | A 请 E 看第 11 条：**E 意见已写**——前三点、第五点同意；改写路径的危机分支（气泡不能点，走不到求助卡片）**进试用前必须补上**。其余条目等 C 等人评审 |
| 危机词表、禁用词表 v2 | **E 已评审，待第二人** | 见第 1 节 E-08 和评审记录 |
| [xinqing-ime#27](https://github.com/young8290/xinqing-ime/pull/27) 暖心话（C-04） | 草稿 | 涉及提示词和暖心话模板，13 第 5 节要求至少 2 人评审、其中 1 人是 E；转为待评审后看 |

### 4.2 W1 评审会要拍板、还没拍板的事

1. 16 第 1 节的方案甲 / 乙 / 丙（工作量超出可用工时约 200–320 h，**这是最大的计划风险**）；
2. 16 第 3 节的负载调整：C-10 移给 B（B 交接文档里仍标“未认领”）；C-02 已由 C 做完，不再移；
3. 16 第 5 节人员指派表、看板；
4. ADR 0007、0009 的评审。

### 4.3 风险登记（13 第 9 节）当前判断

| 编号 | 状态 | 说明 |
|---|---|---|
| R-05 状态识别准确率 | 未缓解 | 只有合成脚本，B-02 真人录制没做，阈值没用真实数据校验 |
| R-07 隐私执行不到位 | 部分缓解 | 结构约束（features_json 只允许数值）、Q-STATE 字段白名单已有测试；db-grep（E-03）和 NFR-PRI 自动化测试还没有 |
| R-10 / R-13 进度与范围 | 未处理 | 见 4.2 第 1 条 |
| R-06 危机漏报 | **E-08 发现并部分处理** | E-CRISIS 的 20/20 不代表泛化：日常说法上 v1 召回只有 50%。v2 补到扩充集 45/45，不调参的一轮是 14/15。仍缺：第二评审人、真正的留出集、Jev 通道（Q-CRISIS）的评测结果；改写路径走不到求助卡片（ADR 0009 第 11 条） |
| 新增：危机安全链路的缺口 | 未处理 | 改写模式里判定为危机时只显示不能点的气泡（ADR 0009 第 11 条），FR-RWR-05 第 3 条要求能打开求助卡片。进试用前必须补，A 与 C 定方案 |
| 新增：密钥可能进日志 | 未处理 | ADR 0012 评审意见第 2 条，C 改 |

## 5. 已知问题

- `daily_summary` 的保留期 09 写“1 年（可调）”，没有对应设置键，现在固定 365 天。
- 已有的开发库是在开 `auto_vacuum` 之前建的，清理后不回收空间（文件不变小），删库重建即可；新库没有这个问题。
- 状态为 `expired` 的日程、`ignored` 的待办 09 没写保留期，暂不清理（ADR 0013 第 7 条）。
- 清理跑在外壳的 `spawn_blocking` 里，与命令共用一把数据库锁；1 年数据第一次清理可能持锁较久。17 第 2.9 节的单写线程 `DbWriter` 接入后改为投递到写线程。
- 清理时刻用系统时钟；演示模式（B-10）替换 `Clock` 后，演示里要看到清理效果需要让 `cleanup.rs` 也用同一个 `Clock`。

## 6. 下一步（按优先级）

1. E-08 收尾：找第二评审人确认词表 v2；请一位没看过词表的组员写危机留出集（≥ 20 正 / 20 负），只测不调；
2. E-01 第二部分收尾：契约 PR 加 `data_export(path)` 命令（D 评审），设置页按钮交给 D-08；
3. E-01 第三部分：撤回同意的后续动作（FR-DAT-05）与删除全部数据（FR-DAT-04）；
4. E-03：`tools/db-grep` 与 NFR-PRI-04 自动化测试（用 xq-sim 回放 50 句特征句后扫库）；
5. PM：W1 评审会议程（第 4.2 节），建 `docs/meetings/`。

## 7. 修订记录

| 日期 | 改动 |
|---|---|
| 2026-10-04 | 初版：盘点 PM-01～04、E-01～08；E-01 第一部分（自动清理）与 ADR 0013；ADR 0008、0010 评审通过，产品书 V1.4 |
| 2026-10-04 | E-08：危机词表与禁用词表评审、v2 补充、扩充集与 Rust 对拍；ADR 0009 第 11 条、0011、0012 的 E 评审意见；风险登记更新 |
| 2026-10-04 | E-01 第二部分：导出数据包（core）；记下改词表要重新生成繁转简表 |
