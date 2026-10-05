from __future__ import annotations

import json
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path

import eval_comfort


class ComfortEvaluationTests(unittest.TestCase):
    def test_han_count_ignores_punctuation_and_ascii(self) -> None:
        self.assertEqual(eval_comfort.han_count("今天辛苦啦，rest!"), 5)

    def test_run_accepts_fifty_clean_predictions(self) -> None:
        dataset = Path(eval_comfort.DATASET_DIR) / "e_comfort.jsonl"
        rows = [json.loads(line) for line in dataset.read_text(encoding="utf-8").splitlines()]
        with tempfile.TemporaryDirectory() as directory:
            predictions = Path(directory) / "predictions.jsonl"
            predictions.write_text(
                "".join(
                    json.dumps(
                        {"id": row["id"], "text": "今天辛苦啦，先歇一会儿。", "kind": "comfort", "human_violation": False},
                        ensure_ascii=False,
                    )
                    + "\n"
                    for row in rows
                ),
                encoding="utf-8",
            )
            report, passed = eval_comfort.run(
                Namespace(
                    dataset=dataset,
                    predictions=predictions,
                    banned_words=Path(eval_comfort.REPO_ROOT) / "hub_templates" / "banned_words.toml",
                    version="P-COMFORT v1",
                )
            )
            self.assertTrue(passed)
            self.assertTrue(report.exists())
            self.assertNotIn("辛苦", report.read_text(encoding="utf-8"))

    def test_human_rate_must_be_below_five_percent(self) -> None:
        dataset = Path(eval_comfort.DATASET_DIR) / "e_comfort.jsonl"
        rows = [json.loads(line) for line in dataset.read_text(encoding="utf-8").splitlines()]
        with tempfile.TemporaryDirectory() as directory:
            predictions = Path(directory) / "predictions.jsonl"
            predictions.write_text(
                "".join(
                    json.dumps(
                        {"id": row["id"], "text": "今天辛苦啦，先歇一会儿。", "kind": "comfort", "human_violation": i < 3},
                        ensure_ascii=False,
                    )
                    + "\n"
                    for i, row in enumerate(rows)
                ),
                encoding="utf-8",
            )
            _, passed = eval_comfort.run(
                Namespace(
                    dataset=dataset,
                    predictions=predictions,
                    banned_words=Path(eval_comfort.REPO_ROOT) / "hub_templates" / "banned_words.toml",
                    version="P-COMFORT v1",
                )
            )
            self.assertFalse(passed)


if __name__ == "__main__":
    unittest.main()
