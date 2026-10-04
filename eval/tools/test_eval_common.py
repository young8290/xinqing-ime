"""Unit tests for the offline evaluation helpers."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from _eval_common import (
    EvaluationError,
    classification,
    find_banned_words,
    load_banned_words,
    prediction_index,
)


class EvaluationHelpersTests(unittest.TestCase):
    def test_classification_counts_and_f1(self) -> None:
        metrics = classification([True, True, False, False], [True, False, True, False])
        self.assertEqual((metrics["tp"], metrics["fp"], metrics["fn"]), (1, 1, 1))
        self.assertAlmostEqual(metrics["precision"], 0.5)
        self.assertAlmostEqual(metrics["recall"], 0.5)
        self.assertAlmostEqual(metrics["f1"], 0.5)

    def test_prediction_index_rejects_duplicates_and_missing_ids(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            predictions = Path(temp_dir) / "predictions.jsonl"
            predictions.write_text('{"id":"A"}\n{"id":"A"}\n', encoding="utf-8")
            with self.assertRaisesRegex(EvaluationError, "重复"):
                prediction_index(predictions, ["A"])
            predictions.write_text('{"id":"A"}\n', encoding="utf-8")
            with self.assertRaisesRegex(EvaluationError, "缺少"):
                prediction_index(predictions, ["A", "B"])

    def test_banned_word_scan_normalizes_full_width_and_chat_allowlist(self) -> None:
        words_path = Path(__file__).resolve().parents[2] / "hub_templates" / "banned_words.toml"
        words = load_banned_words(words_path)
        self.assertTrue(find_banned_words("你　应该想开点", words))
        self.assertEqual(find_banned_words("用药时可以咨询医生", words, chat_scene=True), [])
        self.assertTrue(find_banned_words("用药时可以咨询医生", words))


if __name__ == "__main__":
    unittest.main()
