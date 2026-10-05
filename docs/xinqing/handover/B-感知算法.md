# B 感知算法 · 交接文档

> 负责范围（产品书 13 第 1 节）：STA 状态识别、RST 休息提醒、REV-03 作息洞察、DMO 模拟器与演示模式、E-STATE 评测。
> 任务清单与估算见产品书 16 第 2.2 节。本文件随 B 的每个 PR 更新，任务中途换人时按产品书 13 第 3.1 节“交接”直接看这里。
> 最后更新：2026-10-05（B-03 / B-04：特征与基线的 Python 对拍）

## 1. 任务状态

| 编号 | 任务 | 状态 | 代码 / PR | 还差什么 |
|---|---|---|---|---|
| B-01 | xq-sim（回放、倍速、监听、--baseline、--start-at） | 大部分完成 | `tools/xq-sim/`；回放评测用 `xq-replay`（`xinqing_hub/core/src/bin/xq-replay.rs`） | `--baseline`、`--start-at` 目前只在 `xq-replay` 里。要让 `xq-sim` 也支持，得把参数送到 Hub：`hello.caps` 是枚举，需要改 XQP 契约（A），或者改由 Hub 的开发者选项读取；`--start-at` 还依赖演示时钟（B-10）。方案见第 6 节 |
| B-02 | 录制 7 个 E-STATE 脚本与双人标注 | 未开始（需要人） | 剧本：`eval/datasets/e_state_scripts.md` | 要全员真人录制，Claude 做不了；剧本待两人评审。现在只有 3 个合成脚本（`tools/xq-sim/scripts/`，由 `gen_synthetic.py` 生成） |
| B-03 | 窗口切分、特征计算 | 完成 | `domain/features/{window,calc,typo}.rs`，ADR 0008；Python 对拍：`eval/tools/features_ref.py` + `tests/features_parity.rs`（本 PR） | — |
| B-04 | 个人基线、冷启动默认值 | 完成 | `domain/features/baseline.rs`（分桶统计、`compute_stats`、`next_recompute_after`）、`domain/features/persist.rs`（读库重算、重置）、`hub_templates/baseline_default.toml`；[xinqing-ime#25](https://github.com/young8290/xinqing-ime/pull/25)，ADR 0014 | 默认值 `calibrated = false`，等 B-02 录制后用真实数据校准；7 天基线统计已与 Python 对拍（本 PR） |
| B-05 | 本地规则 R1–R6 | 完成（R1b 除外） | `domain/rules.rs`、`domain/features/typo.rs` | R1b 需要核心在 `comp` 里加 `invalid` 字段（ADR 0008 第 5 条），待 A 决定 |
| B-06 | 融合与滞回、降级运行 | 完成 | `domain/fusion.rs`、`pipeline.rs`、`sense.rs` | Jev 判断还没接进 `Sense`（等 C 的网关 PR），现在实时路径全部走降级 |
| B-07 | 状态解释、反馈校准、自评天气（后端） | 后端完成 | 第一部分（状态解释）：[xinqing-ime#11](https://github.com/young8290/xinqing-ime/pull/11)（已合并），ADR 0010；第二部分（`state_explain` 命令）：[xinqing-ime#12](https://github.com/young8290/xinqing-ime/pull/12)（已合并）；第三部分（反馈校准）：[xinqing-ime#15](https://github.com/young8290/xinqing-ime/pull/15)（已合并）；第四部分（自评天气）：[xinqing-ime#17](https://github.com/young8290/xinqing-ime/pull/17)（已合并），ADR 0011 | 见第 3 节 |
| B-08 | 使用时长与四类休息提醒等（P0） | 第一部分完成（PR 合并后） | 判断 `domain/rest.rs`、服务 `rest.rs`、`infra/store/rest.rs`；外壳 `src-tauri/src/rest.rs`；小组件 `windows/widget/{useRest.ts,RestCard.vue}` | 见第 3.1 节：系统通知、专注时段、`daily_summary` 统计、设置界面 |
| B-09 | 作息洞察统计 | 未开始 | — | 计划 W9 |
| B-10 | 演示模式（clock 替换、演示数据库） | 未开始 | `infra/clock.rs` 已有 `Clock` / `ManualClock` 可用 | 计划 W10 |
| B-11 | E-STATE 评测与报告 | 未开始 | — | 依赖 B-02 的录制数据；`xq-replay --json` 已能输出每个窗口的特征、状态和解释 |
| （C-10） | 情绪日记、晚间小结、周信 | 未认领 | — | 16 第 3 节建议从 C 移给 B，W1 评审会还没定 |

## 2. 代码地图（B 负责的部分）

```
xinqing_hub/core/src/
├─ domain/features/   窗口切分 window.rs、特征 calc.rs、打错字 R1 typo.rs、基线 baseline.rs
├─ domain/rules.rs    R2–R6
├─ domain/fusion.rs   融合与滞回、降级、“不准”阈值上调（record_unfit）
├─ domain/explain.rs  状态解释（FR-STA-09）
├─ pipeline.rs        XQP 事件 → 窗口 → 特征 → 规则 → 融合；replay() 供测试和 xq-replay
├─ sense.rs           实时感知任务（与外壳之间只走 SensePort）
└─ bin/xq-replay.rs   离线回放
tools/xq-sim/         扮演输入法核心的 XQP 模拟器
hub_templates/        baseline_default.toml、app_categories.toml、explain.toml
```

验证：`cargo test -p xinqing-hub-core`；看效果：
`cargo run -p xinqing-hub-core --bin xq-replay -- tools/xq-sim/scripts/hesitant.jsonl --mock-jev`（切换窗口下一行就是解释）。

## 3. 进行中：B-07

| 部分 | 需求 | 状态 |
|---|---|---|
| 状态解释生成 `explain::build`、按模板拼句 `ExplainCopy::render` | FR-STA-09 | 完成（#11） |
| 实时路径：状态切换时生成并缓存，经 `SensePort::explained` 交给外壳；`Sense::explanation()` | FR-STA-09 | 完成（#11）；外壳侧缓存在 `AppState::explanation`（#12） |
| `state_explain` 命令（10 第 5.1 节）：不带参数返回当前解释 | FR-STA-09 | 完成（#12），绑定 `commands.stateExplain(null)`，等 D 接到小组件悬停 |
| 看板时间线历史状态点的解释 `state_explain(mood_state_id)` | FR-STA-09 | 完成（#12）：`Db::explain_mood_state` 用 `mood_state.window_id` 找回窗口和上一个窗口重新生成，不另存解释、不加迁移。参数是 `u32`（specta 不导出 i64） |
| “准 / 不准”反馈：`submit_feedback` → `feedback` 表 → `SenseCmd::Unfit` → `Fusion::record_unfit` | FR-STA-07 | 完成（#15）：`domain/feedback.rs`；不带 `target_id` 时记到最近一条 `mood_state`；Hub 启动时用 `feedback::recent_unfit` 重放最近 7 天的“不准”恢复阈值上调。等 D 接到小组件悬停和看板时间线 |
| 自评天气：`self_report_set(weather, note?)` / `self_report_list(date)`、`self_report:changed`、`HubEvent::SelfReport` | FR-STA-10 | 完成（#17）：`domain/self_report.rs` 写库、按本地日期列出、判断校准；显示覆盖在 `Sense`（`SenseCmd::SelfReport`），60 分钟后或“说不上来”时回到自动判断；覆盖期间 `state_explain()` 返回 `source = self` 的解释。解释见 ADR 0011 |
| 负面自评后晴晴立即回应（P-COMFORT 或本地模板）、“和晴晴聊聊”按钮 | FR-STA-10 第 2 条 | 不归 B：C 的暖心话订阅 `HubEvent::SelfReport`（`weather.is_negative()`）处理，不受冷却和每日上限限制 |
| 研究模式的定时自评邀请（`source = esm`） | FR-DMO-04 | 未做（B-10 演示 / 研究模式时一起做）：`self_report_set` 目前固定写 `source = user` |

## 3.1 B-08 休息提醒

| 部分 | 需求 | 状态 |
|---|---|---|
| 使用时长：活跃分钟、≥2 分钟无操作断开连续使用 | FR-RST-01 | 完成：`RestEngine`。平时用 XQP 按键 / 组字 / 上屏；无痕、英文状态、没连上输入法时改为每 10 秒读系统空闲（Windows `GetLastInputInfo`，只有时间）；`rest.count_when_paused` 关掉则无痕期间不计 |
| 护眼 20 分钟连续、喝水 60 分钟累计、活动 50 分钟连续、夜间（默认 23:30 起，连续 >10 分钟，每晚最多 2 次、间隔 ≥60 分钟） | FR-RST-02～05 | 完成；设置键 `rest.eye/water/move.enabled|interval`、`rest.night.enabled|start`（默认值与范围见 `settings.rs`） |
| 时机：上屏后空闲 3 秒、其他 5 秒；组字中不弹；前台全屏不弹；同类每小时最多 1 次；同时到期按 夜间 > 活动 > 护眼 > 喝水，其余顺延 5 分钟 | FR-RST-06 | 完成 |
| 显示：光标旁气泡（XQP `tip`，2.5 秒）+ 小组件卡片（已完成 / 5 分钟后 / 今天不再提醒，Esc = 5 分钟后）；护眼“已完成”先倒数 20 秒再致谢 | FR-RST-02、06 | 完成；推送 `rest:due`，按钮走 `rest_action` 命令 |
| 疲惫时护眼提前（已连续 ≥10 分钟）并换文案 | FR-RST-07 | 完成：订阅 `MoodEvent::StateChanged` |
| `reminder_log` 记录用户操作 | FR-RST-08 | 完成：只记点了哪个按钮，不记“显示过” |
| 小组件隐藏时改用系统通知 | FR-NTF-01 | **未做**，随 D-09 接入；在此之前只有光标旁气泡 |
| 专注时段 `rest.focus_period` | FR-RST-06 | **未做**：设置键注册表还没有自由文本类型 |
| `daily_summary` 的 `rests_due` / `rests_done` / 喝水次数 | FR-RST-08 | **未做**，等 B-09 统计时一起做（`Db::reminder_counts` 已有） |
| 设置界面里的 `rest.*` | FR-RST-09 | **未做**：统一设置窗口（D-08）暂缓，目前只能改库里的默认值 |

## 4. 关键决定与待评审

- ADR 0008（状态识别规格的解释，7 条）：**已接受**（B、E 同意，产品书 V1.4 已同步；第 5 条 R1b 的协议字段等 A）。B 的意见：同意，尤其第 1 条（组字中停顿不切窗）不改的话 R2 永远不触发。
- ADR 0010（状态解释的信号挑选与拼句）：**已接受**（D、E 同意，产品书 V1.4 已同步）。D 的三点建议已处理：
  ① ADR 第 5 条写明界面自己定版式；② `Explanation.prob` 改名为 `prob_pct`（0–100），与快照的 `prob`（0–1）区分；
  ③ 实时路径改为先交出解释再推送 `status:changed`，界面收到状态变化后取到的一定是同一次切换的解释，界面也可以用 `Explanation.state` 与快照核对。
- ADR 0014（个人基线的持久化与重算：数据来源、重算时机、窗口数口径、最少样本数、重置基线）：#25 提出；C 已同意（#25 评论，编号由 0012 改为 0014），等 E 评审。
- ADR 0011（自评天气的实现解释：“说不上来”不覆盖、校准的计法、自评期间的解释、总线字段类型、覆盖不跨重启）：#17 提出，等 D 与 E 评审。
- 这几份 ADR 接受后都要回产品书仓库改 04 FR-STA-01/03/04/06/08/09/10、10 第 5.1/5.3 节、15 和 17 第 2.3 节，并写 00 第 5 节修订记录。

## 5. 已知问题

- 实时路径没有 Jev 结果，`Explanation.prob_pct` 恒为 `None`，界面不显示百分比；Jev 接入后要把显示状态的概率传给 `explain::build`（`sense.rs` 的 `on_window`）。
- 冷启动的“有效窗口数”口径（ADR 0014 第 3 条）：最近 7 天、上次重置之后的窗口数；启动时从库里重算，实时路径与历史解释口径一致。
- 历史解释的 `{p}` 用的是当前基线的中位数，不是当时的；“上一个窗口”取库里 id 紧挨着的那条，可能跨会话（实时路径在新会话时会清掉上一个窗口）。
- 阈值上调只作用于 Jev 路径（FR-STA-06 第 4、5 条的阈值）。现在实时路径全部走降级（只用本地规则），所以“不准”目前只写库、记入融合，暂时不改变显示结果，Jev 接入后自动生效。
- `feedback` 表要按 D-17 保留 90 天、`self_report` 按 D-24 保留 1 年，清理归 E-01（自动清理）。
- 时间戳（`SelfReportItem.ts`、`self_report:changed.until_ts`）用 f64 毫秒导出，因为 specta 不导出 i64；specta 把 f64 导出成 `number | null`，实际不会是 `null`。根治是在 `export_bindings` 里把 BigInt 导出为 number（毫秒时间戳小于 2^53，安全），那是 D 的导出配置，建议 D 评估后统一改。
- 自评覆盖只在内存里，重启 Hub 后回到自动判断（ADR 0011 第 5 条）。
- 外壳的 `Sensing` 保存一份基线拷贝（历史解释用），启动、04:00 重算和重置后都会同步。
- 04:00 重算目前由感知任务按时钟判断；C 的 `scheduler` 就绪后可以改由它触发（ADR 0014 第 2 条）。
- 重置时间存在 `settings` 表的内部键 `baseline.reset_ts`，不在设置键注册表里，登记在 `settings::INTERNAL_KEYS`（ADR 0014 第 5 条，C 已同意）。E 做导入（E-02）时只导 `KEYS` 里的设置键。
- 夜间提醒的设置键用 `rest.night.start`（开始时刻，22:00–01:00 每半小时一档），10 第 6.2 节列的 `rest.<…>.interval` 不适用于夜间；产品书下次修订时同步。
- 休息提醒每秒判断一次，设置每 10 秒重读，改完设置最多 10 秒生效。
- 只有合成脚本，阈值（如 R6、解释里的 z 阈值）没有用真实数据校验过，B-02 录完后要用 `xq-replay --json` 复核。

## 6. 下一步（按优先级）

1. B-08 剩余：系统通知（随 D-09）、专注时段、`daily_summary` 统计；
2. B-01 剩余：`xq-sim` 的 `--baseline`、`--start-at`。建议做法：不改 XQP，改为 Hub 在调试构建里读环境变量 `XQ_SIM_BASELINE`（固定基线文件，不读写真实基线）和 `XQ_SIM_START_AT`（演示时钟起点，和 B-10 一起做）；`xq-sim` 收到这两个参数时打印对应的 Hub 启动命令。另一种做法是在 `hello` 里加可选字段，那要先写 ADR，请 A 评审契约。和 A、E 商量后再定；
3. B-10 演示模式（时钟替换、演示数据库），与上一条的 `--start-at` 一起做。

## 7. 修订记录

| 日期 | 改动 |
|---|---|
| 2026-10-04 | 初版：盘点 B-01～B-11 现状；B-07 第一部分（状态解释）与 ADR 0010 |
| 2026-10-04 | B-07 第二部分：`state_explain` 命令（当前与历史）、外壳缓存解释；本地开发可装 WebKitGTK 编译外壳（包名见 `xinqing.yml`） |
| 2026-10-04 | B-07 第三部分：`submit_feedback`、`feedback` 表读写、启动时重放“不准” |
| 2026-10-04 | B-07 第四部分：自评天气后端与 ADR 0011；处理 D 对 ADR 0010 的三点建议（`prob_pct`、先交解释再推送状态） |
| 2026-10-04 | B-04：基线写读 `baseline` 表、启动和 04:00 重算、`baseline_reset` 命令与 ADR 0014；修复 #16 / #17 交叉合并后的前端字段名（#22） |
| 2026-10-05 | B-08 第一部分：使用时长、四类休息提醒、时机与勿扰、疲劳联动、`reminder_log`、小组件提醒卡片 |
| 2026-10-05 | B-03 / B-04：窗口切分、特征、基线统计的 Python 参考实现与 Rust 对拍（FR-STA-02/03 验收），CI 校验对拍文件 |
