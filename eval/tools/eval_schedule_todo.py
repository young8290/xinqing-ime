"""Score E-PLAN, E-EXTRACT and E-TODO from offline JSONL predictions."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

from _eval_common import (
    DATASET_DIR,
    EvaluationError,
    classification,
    details_section,
    percent,
    prediction_index,
    rate,
    require_bool,
    require_score,
    write_report,
)


def load_gold(name: str) -> list[dict[str, Any]]:
    path = DATASET_DIR / name
    from _eval_common import read_jsonl

    return read_jsonl(path)


def run(args: argparse.Namespace) -> tuple[Path, bool]:
    plans = load_gold("e_plan.jsonl")
    extracts = load_gold("e_extract.jsonl")
    todos = load_gold("e_todo.jsonl")
    plan_predictions = prediction_index(args.plan_predictions, (row["id"] for row in plans))
    extract_predictions = prediction_index(args.extract_predictions, (row["id"] for row in extracts))
    todo_predictions = prediction_index(args.todo_predictions, (row["id"] for row in todos))

    plan_l1_mismatch: list[str] = []
    plan_positive = [row for row in plans if row["label"] == 1]
    l1_hits = 0
    plan_gold: list[bool] = []
    plan_guess: list[bool] = []
    for gold in plans:
        prediction = plan_predictions[gold["id"]]
        l1_pass = require_bool(prediction, "l1_pass", gold["id"])
        score = require_score(prediction, "q_plan_score", gold["id"])
        if l1_pass != gold["l1_expected"]:
            plan_l1_mismatch.append(gold["id"])
        if gold["label"] == 1 and l1_pass:
            l1_hits += 1
        plan_gold.append(gold["label"] == 1)
        plan_guess.append(l1_pass and score >= 0.80)
    plan_metrics = classification(plan_gold, plan_guess)
    plan_l1_recall = rate(l1_hits, len(plan_positive))

    extraction_primary = 0
    extraction_fields: dict[str, list[str]] = {
        key: [] for key in ("end_time", "all_day", "is_deadline", "location", "flags")
    }
    for gold in extracts:
        prediction = extract_predictions[gold["id"]]
        output = prediction.get("output")
        if not isinstance(output, dict):
            raise EvaluationError(f"{gold['id']}: output 必须是 JSON 对象")
        expected = gold["expected"]
        if "date" not in output or "time" not in output:
            raise EvaluationError(f"{gold['id']}: output 必须包含 date 和 time（无时刻用 null）")
        if output["date"] == expected["date"] and output["time"] == expected["time"]:
            extraction_primary += 1
        for key in ("end_time", "all_day", "is_deadline", "location"):
            if key not in output:
                raise EvaluationError(f"{gold['id']}: output 缺少 {key}")
            if output[key] != expected[key]:
                extraction_fields[key].append(gold["id"])
        if not isinstance(output.get("flags"), list) or any(not isinstance(flag, str) for flag in output["flags"]):
            raise EvaluationError(f"{gold['id']}: output.flags 必须是字符串数组")
        # `adjusted` 只说明代码改过模型的日期或时刻（FR-SCH-04 校验第 1 条），不是卡片该带的标记，不比
        if sorted(set(output["flags"]) - {"adjusted"}) != sorted(set(expected["flags"])):
            extraction_fields["flags"].append(gold["id"])
    extraction_rate = rate(extraction_primary, len(extracts))

    todo_l1_mismatch: list[str] = []
    todo_due_mismatch: list[str] = []
    todo_gold: list[bool] = []
    todo_guess: list[bool] = []
    for gold in todos:
        prediction = todo_predictions[gold["id"]]
        route = prediction.get("l1_route")
        if route not in {"todo", "schedule", "none"}:
            raise EvaluationError(f"{gold['id']}: l1_route 必须是 todo、schedule 或 none")
        score = require_score(prediction, "q_todo_score", gold["id"])
        if route != gold["l1_route"]:
            todo_l1_mismatch.append(gold["id"])
        todo_gold.append(gold["label"] == 1)
        todo_guess.append(route == "todo" and score >= 0.80)
        if gold["label"] == 1:
            output = prediction.get("output")
            if not isinstance(output, dict) or "due_date" not in output:
                raise EvaluationError(f"{gold['id']}: 正例必须提供 output.due_date（无截止日期用 null）")
            if output["due_date"] != gold["expected"]["due_date"]:
                todo_due_mismatch.append(gold["id"])
    todo_metrics = classification(todo_gold, todo_guess)
    todo_positive_count = sum(1 for row in todos if row["label"] == 1)
    todo_due_accuracy = rate(todo_positive_count - len(todo_due_mismatch), todo_positive_count)

    passed = (
        plan_l1_recall >= 0.98
        and plan_metrics["f1"] >= 0.90
        and extraction_rate >= 0.90
        and todo_metrics["f1"] >= 0.85
    )
    body = [
        "## E-PLAN",
        "",
        f"- L1 正例召回：{l1_hits}/{len(plan_positive)}（{percent(plan_l1_recall)}；门槛 ≥ 98%）",
        f"- L1+L2：P={percent(plan_metrics['precision'])}，R={percent(plan_metrics['recall'])}，F1={percent(plan_metrics['f1'])}（门槛 ≥ 90%）",
        "",
        "## E-EXTRACT",
        "",
        f"- 日期与时刻同时准确：{extraction_primary}/{len(extracts)}（{percent(extraction_rate)}；门槛 ≥ 90%）",
    ]
    for field, mismatches in extraction_fields.items():
        body.append(f"- `{field}` 准确：{len(extracts) - len(mismatches)}/{len(extracts)}（{percent(rate(len(extracts) - len(mismatches), len(extracts)))}）")
    body += ["", "## E-TODO", "", f"- 分类：P={percent(todo_metrics['precision'])}，R={percent(todo_metrics['recall'])}，F1={percent(todo_metrics['f1'])}（门槛 ≥ 85%）", f"- 正例 due_date 准确：{todo_positive_count - len(todo_due_mismatch)}/{todo_positive_count}（{percent(todo_due_accuracy)}）", "", f"## 总结：{'通过' if passed else '未达到全部门槛'}", ""]
    body += details_section("E-PLAN L1 与标注不一致", plan_l1_mismatch)
    body += details_section("E-EXTRACT 字段不一致", [f"{row_id} ({field})" for field, ids in extraction_fields.items() for row_id in ids])
    body += details_section("E-TODO 分流与标注不一致", todo_l1_mismatch)
    body += details_section("E-TODO due_date 不一致", todo_due_mismatch)
    report = write_report("schedule-todo", "日程、抽取与待办评测", body, args.version)
    return report, passed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan-predictions", type=Path, required=True, help="E-PLAN 预测 JSONL")
    parser.add_argument("--extract-predictions", type=Path, required=True, help="E-EXTRACT 预测 JSONL")
    parser.add_argument("--todo-predictions", type=Path, required=True, help="E-TODO 预测 JSONL")
    parser.add_argument("--version", required=True, help="Q/P 问题、提示词或规则版本")
    args = parser.parse_args()
    try:
        report, passed = run(args)
    except (EvaluationError, KeyError, TypeError) as error:
        print(f"评测输入错误：{error}", file=sys.stderr)
        return 2
    print(f"报告：{report}")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
