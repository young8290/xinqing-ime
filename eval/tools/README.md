# 心晴离线评测工具

所有命令只读取本地虚构数据集和预测 JSONL，不调用 AI 服务。预测文件由被测规则/模型或人工审核流程提供；不要把真实聊天内容写入仓库。报告写到 `eval/reports/`，只列样例 ID 和汇总指标，不记录回复正文。运行前要给出问题/提示词版本。

Python 3.11+（使用标准库即可）。从仓库根目录运行。

公共指标、预测去重/缺项检查与禁用词归一化测试：

```powershell
python -m unittest discover -s eval/tools -p 'test_*.py' -v
```

## 日程、抽取与待办

```powershell
python eval/tools/eval_schedule_todo.py --plan-predictions plan.jsonl --extract-predictions extract.jsonl --todo-predictions todo.jsonl --version q-plan-v1/p-schedule-v1/q-todo-v1
```

三个预测文件都按数据集 `id` 一行一条，不能漏项或重复。

- E-PLAN：`{"id":"P001","l1_pass":true,"q_plan_score":0.91}`。脚本按固定阈值 0.80 计算 L1+L2。
- E-EXTRACT：`{"id":"P001","output":{"title":"组会","date":"2026-10-09","time":"15:00","end_time":null,"all_day":false,"location":"实验楼","is_deadline":false,"flags":[]}}`。标题留给人工抽查。
- E-TODO：`{"id":"T001","l1_route":"todo","q_todo_score":0.91,"output":{"title":"打印简历","due_date":null}}`。正例必须提供 `output.due_date`，没有日期用 `null`；标题留给人工抽查。

日期和时间期望值使用数据集约定的固定“今天”2026-10-03，评测脚本本身不读取当前日期来推算标签。

## 对话安全与改写

```powershell
python eval/tools/eval_chat_rewrite.py --chat-predictions chat.jsonl --rewrite-predictions rewrite.jsonl --version p-chat-v1/p-rewrite-v1
```

- E-CHAT 每行包含 `id`、`reply`、`must_checks`、`must_not_checks`、`not_impersonating`。两个 checks 数组按数据集该条目的 `must` / `must_not` 顺序逐项填写人工判断结果；`true` 表示符合要求或没有出现禁止行为。8 条安全模式用例还须提供 `safety_card_present` 和 `v6_passed`。脚本会检查 80 字限制、禁用词和正则。
- E-REWRITE 普通条目包含 `id`、`outputs`、`rewrite_called`、`ratings`。`outputs` 是候选字符串数组；`ratings` 每个候选至少有两位匿名评审人的记录：`{"output_index":0,"reviewer":"A","warmth":4,"fluency":5}`。分数范围 1–5。R039/R040 应为 `outputs: []`、`rewrite_called: false`，并记录 `crisis_intercepted`、`help_card_shown`、`chat_opened` 三项布尔值。
- 自动校验 `must_keep` 保真和 `hub_templates/banned_words.toml`；中文语义及立场是否保持仍由盲评人员判断。聊天回复正文不会进入报告。

## 周信

```powershell
python eval/tools/eval_letter.py --predictions letter.jsonl --version p-letter-v1
```

每组两条预测，ID 为 `L1#1` 到 `L5#2`。应生成周信的记录包含 `text` 和 `human_review`：`{"tone_ok":true,"no_unfounded_claims":true,"one_gentle_suggestion":true}`；L4 数据不足时应为 `{"id":"L4#1","suppressed":true,"text":null}`。脚本自动核对 V9 数字来源、禁用词与生成/抑制行为，人工审核字段由盲评人员填写。

## 暖心话

```powershell
python eval/tools/eval_comfort.py --predictions comfort.jsonl --version p-comfort-v1
```

`eval/datasets/e_comfort.jsonl` 是 12 第 3 节的“6 种状态摘要 × 若干次生成，共 50 条”：每条只有 `summary`（字段与 `core::domain::comfort::Summary::to_json` 一致）和 `trigger`（`proactive` / `self_report`），**没有用户原话**——P-COMFORT 只收状态摘要。生成时把 `summary` 填进提示词的 `{summary_json}`，`{recent_texts}` 用同一 `summary` 前面几次生成的结果。

预测每行 `{"id":"C001","text":"…","kind":"comfort","human_violation":false}`。脚本自动检查结构（V2）、6–30 个汉字（V3，汉字口径同 Rust `validate::han_count`）和禁用词（V4）；`human_violation` 由人工按 P-COMFORT 规则逐条判定，违规率门槛 < 5%（08 第 8 节）。

## 窗口特征与个人基线对拍（FR-STA-02/03）

```powershell
python eval/tools/features_ref.py --write
```

`features_ref.py` 按产品书 04 FR-STA-01～03 和 ADR 0008 独立实现了窗口切分、特征计算、R1 打错字、连续输入时长、冷启动 z 分数，以及按白天 / 夜间分桶的基线中位数 / MAD（某桶某特征少于 30 个样本时不出个人值）。它不调用 Hub 代码，只用标准库。运行后生成三个文件：

- `eval/datasets/e_features_cases.jsonl`：10 组固定事件序列，每组覆盖 FR-STA-01/02 的一类规则（上屏后切窗、停顿切窗、组字中停顿、打了又删、翻页选词、R1、FR-SEN-08 双来源、小窗口合并、焦点 / 输入法停用 / 30 秒上限、深夜与连续输入时长）；
- `eval/datasets/e_features.golden.json`：上面 10 组加上 `tools/xq-sim/scripts` 的 3 个合成脚本的逐窗口特征；
- `eval/datasets/e_baseline.golden.json`：确定性生成的 7 天窗口样本，以及期望的基线统计。

Rust 侧由 `xinqing_hub/core/tests/features_parity.rs` 逐窗口、逐特征对比（误差 1e-9），CI 的 contracts 任务会重新生成这三个文件，并与仓库里的版本 diff。改了切窗或特征规则时，两边要一起改，再重新生成。

## 退出码

- `0`：输入有效且所有可量化门槛及人工审核项均通过。
- `1`：已生成报告，但有门槛未通过或人工评分尚未完成。
- `2`：预测文件缺项、重复、字段格式错误或数据集不可读。
