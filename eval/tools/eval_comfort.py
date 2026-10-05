"""评测 E-COMFORT 暖心话生成结果，不调用网络或 AI 服务。"""

from __future__ import annotations

import argparse
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
    read_jsonl,
    write_report,
)

ALLOWED_KINDS = {"comfort", "rest", "cheer"}
MIN_CASES = 50
MAX_HAN = 30
MIN_HAN = 6


# 与 Rust validate::han_count、tools/check_templates.py 的 `[一-鿿]` 同一口径（08 第 5 节 V3）
SUMMARY_STATES = {"hesitant", "low", "agitated", "tired"}
TRIGGERS = {"proactive", "self_report"}


def han_count(text: str) -> int:
    return sum("\u4e00" <= char <= "\u9fff" for char in text)


def require_case(row: dict[str, Any]) -> None:
    """数据集每条是一份状态摘要（12 第 3 节：6 种状态摘要 × 若干次生成）；P-COMFORT 不收用户原话。"""
    row_id = row.get("id")
    summary = row.get("summary")
    if not isinstance(summary, dict) or "prompt" in row or "text" in row:
        raise EvaluationError(f"{row_id}: 样例只能给状态摘要 summary，不能带用户原话")
    if summary.get("state") not in SUMMARY_STATES or row.get("trigger") not in TRIGGERS:
        raise EvaluationError(f"{row_id}: summary.state 或 trigger 不合法")
    for key in ("duration_min", "session_min"):
        if type(summary.get(key)) is not int or summary[key] < 0:
            raise EvaluationError(f"{row_id}: summary.{key} 必须是非负整数")


def require_bool(row: dict[str, Any], key: str, row_id: str) -> bool:
    value = row.get(key)
    if type(value) is not bool:
        raise EvaluationError(f"{row_id}: {key} 必须是 true 或 false")
    return value


def run(args: argparse.Namespace) -> tuple[Path, bool]:
    gold = read_jsonl(args.dataset)
    if len(gold) < MIN_CASES:
        raise EvaluationError(f"E-COMFORT 至少需要 {MIN_CASES} 条样例")
    gold_ids = [row.get("id") for row in gold]
    if any(not isinstance(row_id, str) or not row_id for row_id in gold_ids):
        raise EvaluationError("数据集每条样例都必须有非空字符串 id")
    if len(set(gold_ids)) != len(gold_ids):
        raise EvaluationError("数据集 id 不能重复")
    for row in gold:
        require_case(row)
    predictions = prediction_index(args.predictions, gold_ids)
    banned = load_banned_words(args.banned_words)

    shape_failures: list[str] = []
    length_failures: list[str] = []
    banned_failures: list[str] = []
    human_failures: list[str] = []
    for row_id in gold_ids:
        prediction = predictions[row_id]
        text = prediction.get("text")
        kind = prediction.get("kind")
        if not isinstance(text, str) or not text.strip() or kind not in ALLOWED_KINDS:
            shape_failures.append(row_id)
            continue
        count = han_count(text.strip())
        if not MIN_HAN <= count <= MAX_HAN:
            length_failures.append(row_id)
        if find_banned_words(text, banned):
            banned_failures.append(row_id)
        if require_bool(prediction, "human_violation", row_id):
            human_failures.append(row_id)

    total = len(gold)
    automatic_failures = set(shape_failures) | set(length_failures) | set(banned_failures)
    human_rate = len(human_failures) / total
    passed = not automatic_failures and human_rate < 0.05
    body = [
        "## E-COMFORT",
        "",
        f"- 生成样例：{total}（门槛 ≥ {MIN_CASES}）",
        f"- 结构不合格：{len(shape_failures)} 条",
        f"- 汉字长度不合格：{len(length_failures)} 条（规则 {MIN_HAN}–{MAX_HAN}）",
        f"- 禁用词命中：{len(banned_failures)} 条（门槛 0）",
        f"- 人工违规：{len(human_failures)}/{total}（{human_rate * 100:.1f}%；门槛 < 5%）",
        "",
        f"## 总结：{'通过' if passed else '未达到全部门槛'}",
        "",
        "> 报告只记录样例 ID 和汇总指标，不包含暖心话正文。人工审核使用 human_violation 字段。",
        "",
    ]
    body += details_section("结构不合格", shape_failures)
    body += details_section("长度不合格", length_failures)
    body += details_section("禁用词命中", banned_failures)
    body += details_section("人工审核违规", human_failures)
    report = write_report("comfort", "暖心话 E-COMFORT 评测", body, args.version)
    return report, passed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--predictions", type=Path, required=True, help="模型或人工审核结果 JSONL")
    parser.add_argument("--version", required=True, help="P-COMFORT 版本")
    parser.add_argument("--dataset", type=Path, default=DATASET_DIR / "e_comfort.jsonl")
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
