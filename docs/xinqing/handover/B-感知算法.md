# B 感知算法 · 交接文档

> 负责范围（产品书 13 第 1 节）：STA 状态识别、RST 休息提醒、REV-03 作息洞察、DMO 模拟器与演示模式、E-STATE 评测。
> 任务清单与估算见产品书 16 第 2.2 节。本文件随 B 的每个 PR 更新，任务中途换人时按产品书 13 第 3.1 节“交接”直接看这里。
> 最后更新：2026-10-04（B-07 第三部分：状态反馈校准）

## 1. 任务状态

| 编号 | 任务 | 状态 | 代码 / PR | 还差什么 |
|---|---|---|---|---|
| B-01 | xq-sim（回放、倍速、监听、--baseline、--start-at） | 大部分完成 | `tools/xq-sim/`；回放评测用 `xq-replay`（`xinqing_hub/core/src/bin/xq-replay.rs`） | `--baseline`、`--start-at` 目前只在 `xq-replay` 里；FR-DMO-01 要求 `xq-sim` 也支持（改写时间戳的会话起点、发给 Hub 的基线），未做 |
| B-02 | 录制 7 个 E-STATE 脚本与双人标注 | 未开始（需要人） | 剧本：`eval/datasets/e_state_scripts.md` | 要全员真人录制，Claude 做不了；剧本待两人评审。现在只有 3 个合成脚本（`tools/xq-sim/scripts/`，由 `gen_synthetic.py` 生成） |
| B-03 | 窗口切分、特征计算 | 完成 | `domain/features/{window,calc,typo}.rs`，ADR 0008 | 与 Python 对拍目前只有危机词表（`tests/crisis_parity.rs`），特征对拍脚本未写 |
| B-04 | 个人基线、冷启动默认值 | 部分完成 | `domain/features/baseline.rs`、`hub_templates/baseline_default.toml` | 每天 04:00 重算、写读 `baseline` 表（D-09）、设置里“重置基线”都没做；外壳目前每次启动都用出厂默认值（`src-tauri/src/sensing.rs`）；默认值 `calibrated = false`，等 B-02 录制后校准 |
| B-05 | 本地规则 R1–R6 | 完成（R1b 除外） | `domain/rules.rs`、`domain/features/typo.rs` | R1b 需要核心在 `comp` 里加 `invalid` 字段（ADR 0008 第 5 条），待 A 决定 |
| B-06 | 融合与滞回、降级运行 | 完成 | `domain/fusion.rs`、`pipeline.rs`、`sense.rs` | Jev 判断还没接进 `Sense`（等 C 的网关 PR），现在实时路径全部走降级 |
| B-07 | 状态解释、反馈校准、自评天气（后端） | **进行中** | 第一部分（状态解释）：[xinqing-ime#11](https://github.com/young8290/xinqing-ime/pull/11)（已合并），ADR 0010；第二部分（`state_explain` 命令）：[xinqing-ime#12](https://github.com/young8290/xinqing-ime/pull/12)（已合并）；第三部分（反馈校准）：本 PR | 见第 3 节 |
| B-08 | 使用时长与四类休息提醒等（P0） | 未开始 | — | 计划 W7–W8 |
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
| “准 / 不准”反馈：`submit_feedback` → `feedback` 表 → `SenseCmd::Unfit` → `Fusion::record_unfit` | FR-STA-07 | 完成（本 PR）：`domain/feedback.rs`；不带 `target_id` 时记到最近一条 `mood_state`；Hub 启动时用 `feedback::recent_unfit` 重放最近 7 天的“不准”恢复阈值上调。等 D 接到小组件悬停和看板时间线 |
| 自评天气：`self_report_set` / `self_report_list`、60 分钟显示覆盖、`self_report:changed`、与自动判断差异连续 3 次时调阈值 | FR-STA-10 | 未做。17 第 2.4 节：自评不经过 Fusion，由 `self_report` 服务直接设置显示覆盖；负面自评的晴晴回应是 C 的暖心话，需要和 C 约定接口 |

## 4. 关键决定与待评审

- ADR 0008（状态识别规格的解释，7 条）：状态“提议”，等 B 与 E 评审。B 的意见：同意，尤其第 1 条（组字中停顿不切窗）不改的话 R2 永远不触发。
- ADR 0010（状态解释的信号挑选与拼句）：#11 提出，等 D（界面拼句）与 E 评审。
- 两份 ADR 接受后都要回产品书仓库改 04 FR-STA-01/04/06/08/09 和 15，并写 00 第 5 节修订记录。

## 5. 已知问题

- 实时路径没有 Jev 结果，`Explanation.prob` 恒为 `None`，界面不显示百分比；Jev 接入后要把显示状态的概率传给 `explain::build`（`sense.rs` 的 `on_window`）。
- 解释里的冷启动判断看的是生成那一刻的 `Baseline::is_cold()`；B-04 接入持久化前，每次重启 Hub 都会回到冷启动。
  历史解释改用“截至该窗口库里已有的窗口数”判断，因此重启后两者可能不一致；B-04 应在启动时用库里的窗口数初始化 `Baseline::windows`，两边就一致了。
- 历史解释的 `{p}` 用的是当前基线的中位数，不是当时的；“上一个窗口”取库里 id 紧挨着的那条，可能跨会话（实时路径在新会话时会清掉上一个窗口）。
- 阈值上调只作用于 Jev 路径（FR-STA-06 第 4、5 条的阈值）。现在实时路径全部走降级（只用本地规则），所以“不准”目前只写库、记入融合，暂时不改变显示结果，Jev 接入后自动生效。
- `feedback` 表要按 D-17 保留 90 天，清理归 E-01（自动清理）。
- 外壳的 `Sensing::baseline` 是启动时的一份拷贝，B-04 做基线重算后要同步更新它（或改成从库里读）。
- 只有合成脚本，阈值（如 R6、解释里的 z 阈值）没有用真实数据校验过，B-02 录完后要用 `xq-replay --json` 复核。

## 6. 下一步（按优先级）

1. B-07：自评天气后端（`self_report_set` / `self_report_list`、60 分钟显示覆盖、`self_report:changed`、差异连续 3 次时调阈值）；
2. B-04 剩余：`baseline` 表读写、04:00 重算、重置基线（重算任务依赖 C 的 `scheduler`，没有就先在 `Sense` 里按 `Clock` 判断）；
3. B-08 休息提醒（P0，W7–W8）。

## 7. 修订记录

| 日期 | 改动 |
|---|---|
| 2026-10-04 | 初版：盘点 B-01～B-11 现状；B-07 第一部分（状态解释）与 ADR 0010 |
| 2026-10-04 | B-07 第二部分：`state_explain` 命令（当前与历史）、外壳缓存解释；本地开发可装 WebKitGTK 编译外壳（包名见 `xinqing.yml`） |
| 2026-10-04 | B-07 第三部分：`submit_feedback`、`feedback` 表读写、启动时重放“不准” |
