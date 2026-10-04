"""Evaluate E-LETTER numeric fact consistency and human review annotations."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

from _eval_common import (
    DATASET_DIR,
    REPO_ROOT,
    EvaluationError,
    details_section,
    find_banned_words,
    load_banned_words,
    prediction_index,
    write_report,
)


NUMBER_PATTERN = re.compile(r"\d+(?:[.,]\d+)?")


def load_letter_gold() -> list[dict[str, Any]]:
    path = DATASET_DIR / "e_letter.json"
    try:
        data = json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvaluationError(f"无法读取周信数据集 {path}: {error}") from error
    items = data.get("items") if isinstance(data, dict) else None
    if not isinstance(items, list) or len(items) != 5:
        raise EvaluationError("e_letter.json 必须包含 5 个 items")
    return items


def source_numbers(item: dict[str, Any]) -> set[str]:
    serialized = json.dumps(item, ensure_ascii=False, sort_keys=True)
    return set(NUMBER_PATTERN.findall(serialized))


def run(args: argparse.Namespace) -> tuple[Path, bool]:
    gold_items = load_letter_gold()
    expected_ids = [f"{item['id']}#{attempt}" for item in gold_items for attempt in (1, 2)]
    predictions = prediction_index(args.predictions, expected_ids)
    banned_words = load_banned_words(args.banned_words)

    fact_mismatch: list[str] = []
    generation_mismatch: list[str] = []
    banned_ids: list[str] = []
    human_review_missing: list[str] = []
    manual_violations: list[str] = []
    generated = 0
    expected_generation_count = 0
    for item in gold_items:
        should_generate = item["expected"]["generate"]
        if should_generate:
            expected_generation_count += 2
        allowed_numbers = source_numbers(item)
        for attempt in (1, 2):
            prediction_id = f"{item['id']}#{attempt}"
            row = predictions[prediction_id]
            suppressed = row.get("suppressed")
            if type(suppressed) is not bool:
                raise EvaluationError(f"{prediction_id}: suppressed 必须是 true 或 false")
            text = row.get("text")
            if should_generate:
                if suppressed or not isinstance(text, str) or not text.strip():
                    generation_mismatch.append(prediction_id)
                    continue
                generated += 1
                unsupported = sorted(set(NUMBER_PATTERN.findall(text)) - allowed_numbers)
                if unsupported:
                    fact_mismatch.append(prediction_id)
                if find_banned_words(text, banned_words):
                    banned_ids.append(prediction_id)
                review = row.get("human_review")
                if not isinstance(review, dict):
                    human_review_missing.append(prediction_id)
                    continue
                for key in ("tone_ok", "no_unfounded_claims", "one_gentle_suggestion"):
                    if type(review.get(key)) is not bool:
                        raise EvaluationError(f"{prediction_id}: human_review.{key} 必须是人工填写的布尔值")
                if not all(review[key] for key in ("tone_ok", "no_unfounded_claims", "one_gentle_suggestion")):
                    manual_violations.append(prediction_id)
            else:
                if not suppressed or text not in (None, ""):
                    generation_mismatch.append(prediction_id)

    fact_ok = generated - len(fact_mismatch)
    fact_rate = fact_ok / generated if generated else 0.0
    suppression_pass = sum(1 for item in gold_items if not item["expected"]["generate"] for attempt in (1, 2) if predictions[f"{item['id']}#{attempt}"].get("suppressed") is True and predictions[f"{item['id']}#{attempt}"].get("text") in (None, ""))
    passed = (
        generated == expected_generation_count
        and not generation_mismatch
        and fact_rate == 1.0
        and suppression_pass == 2
        and not banned_ids
        and not manual_violations
        and not human_review_missing
    )
    body = [
        "## E-LETTER",
        "",
        f"- 已生成应生成周信：{generated}/{expected_generation_count}",
        f"- V9 数字均可在对应统计 JSON 中找到：{fact_ok}/{generated}（{fact_rate * 100:.1f}%；门槛 100%）",
        f"- 禁用词：{len(banned_ids)} 次命中（门槛 0）",
        f"- 数据不足时正确不生成：{suppression_pass}/2",
        f"- 人工审核未填写：{len(human_review_missing)}；人工审核违规：{len(manual_violations)}（门槛均为 0）",
        "",
        f"## 总结：{'通过' if passed else '未达到全部门槛或人工审核尚未完成'}",
        "",
        "> V9 按规范检查输出中的阿拉伯数字是否出现在该组完整统计 JSON 中；人工审核字段须由盲评人员填写。报告不包含信件正文。",
        "",
    ]
    body += details_section("生成/抑制行为不一致", generation_mismatch)
    body += details_section("V9 出现统计中不存在的数字", fact_mismatch)
    body += details_section("禁用词命中", banned_ids)
    body += details_section("人工审核未填写", human_review_missing)
    body += details_section("人工审核未通过", manual_violations)
    report = write_report("letter", "周信事实一致性评测", body, args.version)
    return report, passed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--predictions", type=Path, required=True, help="E-LETTER 两次生成结果 JSONL")
    parser.add_argument("--version", required=True, help="提示词版本")
    parser.add_argument("--banned-words", type=Path, default=REPO_ROOT / "hub_templates" / "banned_words.toml")
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
