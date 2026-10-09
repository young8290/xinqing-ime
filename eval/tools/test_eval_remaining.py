"""Synthetic scorer regressions; these fixtures are not real API evaluations."""
from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import _eval_common as common
import eval_chat_rewrite as chat
import eval_letter as letter
import eval_schedule_todo as schedule


class RemainingScorerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        patch = mock.patch.object(common, "REPORT_DIR", self.root / "reports")
        patch.start()
        self.addCleanup(patch.stop)

    def write(self, name, rows):
        path = self.root / name
        path.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows), encoding="utf-8")
        return path

    def letter_rows(self):
        return [{"id": f"{item['id']}#{attempt}", "suppressed": not item["expected"]["generate"],
                 "text": "这一周辛苦了，给自己一点休息的时间。" if item["expected"]["generate"] else "",
                 "human_review": {key: True for key in ("tone_ok", "no_unfounded_claims", "one_gentle_suggestion")}}
                for item in letter.load_letter_gold() for attempt in (1, 2)]

    def run_letter(self, rows):
        return letter.run(argparse.Namespace(predictions=self.write("letter.jsonl", rows), version="synthetic-test", banned_words=common.REPO_ROOT / "hub_templates/banned_words.toml"))

    def test_letter_pass_and_report_does_not_expose_body(self):
        path, passed = self.run_letter(self.letter_rows())
        self.assertTrue(passed)
        self.assertNotIn("这一周辛苦了", path.read_text(encoding="utf-8"))

    def test_letter_rejects_facts_suppression_and_missing_review(self):
        for field, value in [("text", "这一周辛苦了，完成了 987654321 件事。"), ("suppressed", True), ("human_review", None)]:
            rows = self.letter_rows()
            rows[0][field] = value
            self.assertFalse(self.run_letter(rows)[1])
        rows = self.letter_rows()
        suppressed = next(row for row in rows if row["suppressed"])
        suppressed["text"] = "不应生成"
        self.assertFalse(self.run_letter(rows)[1])

    def test_letter_rejects_duplicate_missing_or_malformed_predictions(self):
        for mutation in (lambda rows: rows + [rows[0]], lambda rows: rows[:-1], lambda rows: [{**rows[0], "suppressed": "false"}, *rows[1:]]):
            with self.assertRaises(common.EvaluationError):
                self.run_letter(mutation(self.letter_rows()))

    def chat_rows(self):
        chats = [{"id": row["id"], "reply": "听起来这段时间很不容易。", "must_checks": [True] * len(row["must"]), "must_not_checks": [True] * len(row["must_not"]), "not_impersonating": True, "safety_card_present": True, "v6_passed": True}
                 for row in common.read_jsonl(common.DATASET_DIR / "e_chat.jsonl")]
        rewrites = []
        for row in common.read_jsonl(common.DATASET_DIR / "e_rewrite.jsonl"):
            crisis = row["id"] in chat.CRISIS_REWRITE_IDS
            rewrites.append({"id": row["id"], "outputs": [] if crisis else ["我们可以慢慢说。" + " ".join(row["must_keep"])], "rewrite_called": not crisis,
                             "crisis_intercepted": True, "help_card_shown": True, "chat_opened": True,
                             "ratings": [] if crisis else [{"output_index": 0, "reviewer": reviewer, "warmth": 5, "fluency": 5} for reviewer in ("synthetic-a", "synthetic-b")]})
        return chats, rewrites

    def run_chat(self, chats, rewrites):
        return chat.run(argparse.Namespace(chat_predictions=self.write("chat.jsonl", chats), rewrite_predictions=self.write("rewrite.jsonl", rewrites), version="synthetic-test", banned_words=common.REPO_ROOT / "hub_templates/banned_words.toml"))

    def test_chat_and_rewrite_perfect_synthetic_checks(self):
        self.assertTrue(self.run_chat(*self.chat_rows())[1])

    def test_chat_requires_safety_card_and_nonempty_reply(self):
        for value in (False,):
            chats, rewrites = self.chat_rows()
            next(row for row in chats if row["id"] in chat.SAFETY_MODE_IDS)["safety_card_present"] = value
            self.assertFalse(self.run_chat(chats, rewrites)[1])
        chats, rewrites = self.chat_rows()
        chats[0]["reply"] = "  "
        self.assertFalse(self.run_chat(chats, rewrites)[1])

    def test_rewrite_requires_two_distinct_reviewers_and_crisis_interception(self):
        chats, rewrites = self.chat_rows()
        rewrites[0]["ratings"].pop()
        self.assertFalse(self.run_chat(chats, rewrites)[1])
        chats, rewrites = self.chat_rows()
        rewrites[0]["ratings"][1]["reviewer"] = rewrites[0]["ratings"][0]["reviewer"]
        with self.assertRaises(common.EvaluationError):
            self.run_chat(chats, rewrites)
        chats, rewrites = self.chat_rows()
        next(row for row in rewrites if row["id"] in chat.CRISIS_REWRITE_IDS)["rewrite_called"] = True
        self.assertFalse(self.run_chat(chats, rewrites)[1])

    def schedule_rows(self):
        plans = [{"id": row["id"], "l1_pass": row["l1_expected"], "q_plan_score": float(row["label"])} for row in schedule.load_gold("e_plan.jsonl")]
        extracts = [{"id": row["id"], "output": copy.deepcopy(row["expected"])} for row in schedule.load_gold("e_extract.jsonl")]
        todos = [{"id": row["id"], "l1_route": row["l1_route"], "q_todo_score": float(row["label"]), "output": copy.deepcopy(row.get("expected", {}))} for row in schedule.load_gold("e_todo.jsonl")]
        return plans, extracts, todos

    def run_schedule(self, plans, extracts, todos):
        return schedule.run(argparse.Namespace(plan_predictions=self.write("plan.jsonl", plans), extract_predictions=self.write("extract.jsonl", extracts), todo_predictions=self.write("todo.jsonl", todos), version="synthetic-test"))

    def test_schedule_perfect_synthetic_predictions(self):
        self.assertTrue(self.run_schedule(*self.schedule_rows())[1])

    def test_schedule_failed_thresholds_and_invalid_fields(self):
        plans, extracts, todos = self.schedule_rows()
        for row in plans:
            row["q_plan_score"] = 0.0
        self.assertFalse(self.run_schedule(plans, extracts, todos)[1])
        plans, extracts, todos = self.schedule_rows()
        plans[0]["q_plan_score"] = True
        with self.assertRaises(common.EvaluationError):
            self.run_schedule(plans, extracts, todos)
        plans, extracts, todos = self.schedule_rows()
        extracts[0]["output"].pop("time")
        with self.assertRaises(common.EvaluationError):
            self.run_schedule(plans, extracts, todos)


if __name__ == "__main__":
    unittest.main()
