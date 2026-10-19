"""Text boolean tokens must not fall through to Python string truthiness."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from compile_prompt import _optional_bool, compile_scene, example_spec  # noqa: E402


FULL_BODY_PROMPT = "Use full-body framing with the subject inside the frame."
FULL_BODY_QA = "Check full-body framing and crop boundaries."


class OptionalBoolTokenTests(unittest.TestCase):
    def test_explicit_string_tokens_are_normalized(self) -> None:
        for token in ("off", "OFF", " no ", "FALSE", "0"):
            with self.subTest(token=token):
                self.assertIs(_optional_bool(token), False)
        for token in ("on", "ON", " true ", "YES", "1"):
            with self.subTest(token=token):
                self.assertIs(_optional_bool(token), True)
        for token in ("", "unknown", "null", "none", "perhaps"):
            with self.subTest(token=token):
                self.assertIsNone(_optional_bool(token))

    def test_false_full_body_tokens_disable_prompt_and_qa(self) -> None:
        for token in ("off", "OFF", " no ", "false", "0"):
            with self.subTest(token=token):
                spec = example_spec("complex-outfit")
                spec["camera"]["full_body"] = token
                report = compile_scene(spec)
                self.assertNotIn(FULL_BODY_PROMPT, report["prompt"])
                self.assertNotIn(FULL_BODY_QA, report["qa_checklist"])
                self.assertIn("Use a locked camera.", report["prompt"])
                self.assertIn("luxury high-rise apartment", report["prompt"])

    def test_true_full_body_tokens_keep_prompt_and_qa(self) -> None:
        for token in ("on", "ON", "yes", "true", "1"):
            with self.subTest(token=token):
                spec = example_spec("complex-outfit")
                spec["camera"]["full_body"] = token
                report = compile_scene(spec)
                self.assertIn(FULL_BODY_PROMPT, report["prompt"])
                self.assertIn(FULL_BODY_QA, report["qa_checklist"])

    def test_unrecognized_nonempty_string_stays_unknown_not_truthy(self) -> None:
        spec = example_spec("complex-outfit")
        spec["camera"]["full_body"] = "maybe"

        report = compile_scene(spec)

        self.assertNotIn(FULL_BODY_PROMPT, report["prompt"])
        self.assertNotIn(FULL_BODY_QA, report["qa_checklist"])
        self.assertIn("Use a locked camera.", report["prompt"])
        self.assertIn("Requested transition: full_body_cutout_fly_in.", report["prompt"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
