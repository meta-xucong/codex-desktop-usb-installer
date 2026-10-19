"""Focused source-contract checks for the Canvas agent routing metadata."""

from pathlib import Path
import re
import unittest


YAML_PATH = Path(__file__).resolve().parents[1] / "agents" / "openai.yaml"


def scalar(name: str, text: str) -> str:
    match = re.search(rf"^  {re.escape(name)}: \"(.*)\"$", text, re.MULTILINE)
    if not match:
        raise AssertionError(f"missing quoted YAML scalar: {name}")
    return match.group(1)


class CanvasAgentSourceContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = YAML_PATH.read_text(encoding="utf-8")
        cls.short_description = scalar("short_description", cls.source)
        cls.default_prompt = scalar("default_prompt", cls.source).lower()

    def test_metadata_does_not_assume_runtime_is_unavailable(self):
        blocked_phrases = (
            "当前没有兼容",
            "runtime unavailable",
            "unavailable canvas runtime",
        )
        combined = (self.short_description + " " + self.default_prompt).lower()
        for phrase in blocked_phrases:
            with self.subTest(phrase=phrase):
                self.assertNotIn(phrase.lower(), combined)

    def test_prompt_probes_and_uses_backend_when_available(self):
        self.assertIn("first probe", self.default_prompt)
        self.assertIn("connected and permitted", self.default_prompt)
        self.assertIn("if it is, create the requested canvas", self.default_prompt)

    def test_prompt_falls_back_for_unavailable_or_denied_and_stays_truthful(self):
        self.assertIn("unavailable or permission is denied", self.default_prompt)
        self.assertIn("no external canvas was created", self.default_prompt)
        self.assertIn("safe local html prototype", self.default_prompt)
        self.assertIn("never claim an external action that did not happen", self.default_prompt)


if __name__ == "__main__":
    unittest.main()
