"""Shared helpers for XinQing's offline evaluation commands."""

from __future__ import annotations

import json
import re
import tomllib
import unicodedata
from datetime import date
from pathlib import Path
from typing import Any, Iterable


REPO_ROOT = Path(__file__).resolve().parents[2]
DATASET_DIR = REPO_ROOT / "eval" / "datasets"
REPORT_DIR = REPO_ROOT / "eval" / "reports"


class EvaluationError(ValueError):
    """An input dataset or prediction file does not satisfy its schema."""


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    try:
        with path.open("r", encoding="utf-8-sig") as source:
            for line_number, line in enumerate(source, 1):
                if not line.strip():
                    continue
                try:
                    row = json.loads(line)
                except json.JSONDecodeError as error:
                    raise EvaluationError(f"{path}:{line_number}: JSON 格式错误：{error.msg}") from error
                if not isinstance(row, dict):
                    raise EvaluationError(f"{path}:{line_number}: 每行必须是 JSON 对象")
                rows.append(row)
    except OSError as error:
        raise EvaluationError(f"无法读取文件 {path}: {error}") from error
    return rows


def prediction_index(path: Path, expected_ids: Iterable[str]) -> dict[str, dict[str, Any]]:
    expected = set(expected_ids)
    indexed: dict[str, dict[str, Any]] = {}
    for row in read_jsonl(path):
        row_id = row.get("id")
        if not isinstance(row_id, str) or not row_id:
            raise EvaluationError(f"{path}: 每条预测都必须有非空字符串 id")
        if row_id in indexed:
            raise EvaluationError(f"{path}: id 重复：{row_id}")
        if row_id not in expected:
            raise EvaluationError(f"{path}: 未知 id：{row_id}")
        indexed[row_id] = row
    missing = sorted(expected - indexed.keys())
    if missing:
        raise EvaluationError(f"{path}: 缺少 {len(missing)} 条预测：{', '.join(missing)}")
    return indexed


def require_bool(row: dict[str, Any], key: str, row_id: str) -> bool:
    value = row.get(key)
    if type(value) is not bool:
        raise EvaluationError(f"{row_id}: {key} 必须是 true 或 false")
    return value


def require_score(row: dict[str, Any], key: str, row_id: str) -> float:
    value = row.get(key)
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not 0 <= value <= 1:
        raise EvaluationError(f"{row_id}: {key} 必须是 0 到 1 之间的数字")
    return float(value)


def classification(gold: Iterable[bool], predicted: Iterable[bool]) -> dict[str, float | int]:
    pairs = list(zip(gold, predicted, strict=True))
    tp = sum(actual and guess for actual, guess in pairs)
    fp = sum(not actual and guess for actual, guess in pairs)
    fn = sum(actual and not guess for actual, guess in pairs)
    precision = tp / (tp + fp) if tp + fp else 0.0
    recall = tp / (tp + fn) if tp + fn else 0.0
    f1 = 2 * precision * recall / (precision + recall) if precision + recall else 0.0
    return {"tp": tp, "fp": fp, "fn": fn, "precision": precision, "recall": recall, "f1": f1}


def rate(correct: int, total: int) -> float:
    return correct / total if total else 0.0


def percent(value: float) -> str:
    return f"{value * 100:.1f}%"


def write_report(dataset: str, title: str, body: list[str], versions: str) -> Path:
    REPORT_DIR.mkdir(parents=True, exist_ok=True)
    report_path = REPORT_DIR / f"{date.today().isoformat()}-{dataset.lower()}.md"
    content = [
        f"# {title}",
        "",
        f"- 评测日期：{date.today().isoformat()}",
        f"- 问题/提示词版本：{versions}",
        "- 输入仅按样例 ID 关联；报告不包含模型回复正文。",
        "",
        *body,
        "",
    ]
    report_path.write_text("\n".join(content), encoding="utf-8")
    return report_path


def load_banned_words(path: Path) -> dict[str, list[str]]:
    try:
        with path.open("rb") as source:
            raw = tomllib.load(source)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise EvaluationError(f"无法读取禁用词表 {path}: {error}") from error
    result: dict[str, list[str]] = {}
    for group in ("diagnosis", "surveillance", "preachy", "dependency", "routine"):
        values = raw.get(group, [])
        if not isinstance(values, list) or any(not isinstance(value, str) for value in values):
            raise EvaluationError(f"禁用词表字段 {group} 必须是字符串数组")
        result[group] = values
    patterns = raw.get("patterns", [])
    if not isinstance(patterns, list) or any(not isinstance(value, str) for value in patterns):
        raise EvaluationError("禁用词表字段 patterns 必须是字符串数组")
    result["patterns"] = patterns
    allow = raw.get("chat_allow", [])
    if not isinstance(allow, list) or any(not isinstance(value, str) for value in allow):
        raise EvaluationError("禁用词表字段 chat_allow 必须是字符串数组")
    result["chat_allow"] = allow
    return result


def find_banned_words(text: str, words: dict[str, list[str]], chat_scene: bool = False) -> list[str]:
    normalized = "".join(unicodedata.normalize("NFKC", text).split()).casefold()
    allow = [word.casefold() for word in words["chat_allow"]] if chat_scene else []
    found: set[str] = set()
    for group in ("diagnosis", "surveillance", "preachy", "dependency", "routine"):
        for word in words[group]:
            if word.casefold() not in allow and word.casefold() in normalized:
                found.add(word)
    for pattern in words["patterns"]:
        try:
            if re.search(pattern, normalized, flags=re.IGNORECASE):
                found.add(f"正则规则:{pattern}")
        except re.error as error:
            raise EvaluationError(f"禁用词表正则无效 {pattern!r}: {error}") from error
    return sorted(found)


def details_section(title: str, ids: Iterable[str]) -> list[str]:
    values = sorted(set(ids))
    if not values:
        return [f"### {title}", "", "无。", ""]
    return [f"### {title}", "", *[f"- `{value}`" for value in values], ""]
