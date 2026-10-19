#!/usr/bin/env python3
"""Reject structured or scalar JSON values in policy text slots."""

from __future__ import annotations

import contextlib
import copy
import io
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import policy_analyzer as analyzer  # noqa: E402


ROOT = Path(__file__).resolve().parents[4]
SAMPLE = {
    "as_of": "2026-10-19",
    "sources": ["synthetic source"],
    "policy_type": "synthetic policy",
    "action": "synthetic action",
    "context": "synthetic context",
    "evidence": ["synthetic evidence"],
    "transmission_hypotheses": [{
        "statement": "synthetic hypothesis",
        "basis": "synthetic basis",
        "confidence": "low synthetic confidence",
    }],
    "scenario_implications": [{
        "scenario": "synthetic base",
        "implication": "synthetic implication",
        "monitor": ["synthetic indicator"],
    }],
    "risks": ["synthetic risk"],
}
INVALID_VALUES = ({"nested": "object"}, ["nested", "array"], 123, True)


def set_nested(data: dict, path: tuple[str | int, ...], value) -> None:
    current = data
    for part in path[:-1]:
        current = current[part]
    current[path[-1]] = value


class PolicyAnalyzerTextShapeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        self.root = Path(self.temp_dir.name)
        self.addCleanup(self.temp_dir.cleanup)

    def run_cli(self, raw: dict):
        input_path = self.root / "policy-input.json"
        input_path.write_text(json.dumps(raw, ensure_ascii=False), encoding="utf-8")
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            code = analyzer.main(["--input", str(input_path), "--json"])
        return code, stdout.getvalue(), stderr.getvalue()

    def test_all_text_slots_reject_object_array_number_and_boolean(self) -> None:
        slots = [
            (("as_of",), "as_of"),
            (("policy_type",), "policy_type"),
            (("action",), "action"),
            (("context",), "context"),
            (("sources", 0), "sources[]"),
            (("evidence", 0), "evidence[]"),
            (("risks", 0), "risks[]"),
            (("transmission_hypotheses", 0, "statement"), "transmission_hypotheses[0].statement"),
            (("transmission_hypotheses", 0, "basis"), "transmission_hypotheses[0].basis"),
            (("transmission_hypotheses", 0, "confidence"), "transmission_hypotheses[0].confidence"),
            (("scenario_implications", 0, "scenario"), "scenario_implications[0].scenario"),
            (("scenario_implications", 0, "implication"), "scenario_implications[0].implication"),
            (("scenario_implications", 0, "monitor", 0), "scenario_implications[0].monitor[]"),
        ]
        for path, field in slots:
            for invalid in INVALID_VALUES:
                with self.subTest(path=path, invalid=invalid):
                    raw = copy.deepcopy(SAMPLE)
                    set_nested(raw, path, invalid)
                    with self.assertRaisesRegex(ValueError, re.escape(field)):
                        analyzer.validate_policy_input(raw)

    def test_cli_returns_field_specific_json_error_for_malformed_slots(self) -> None:
        for path, field, invalid in (
            (("as_of",), "as_of", {"date": "2026-10-19"}),
            (("policy_type",), "policy_type", ["rate policy"]),
            (("sources", 0), "sources[]", {"url": "synthetic://source"}),
        ):
            with self.subTest(path=path):
                raw = copy.deepcopy(SAMPLE)
                set_nested(raw, path, invalid)
                code, stdout, stderr = self.run_cli(raw)
                self.assertEqual(code, 1)
                self.assertEqual(stderr, "")
                error = json.loads(stdout)
                self.assertFalse(error["success"])
                self.assertIn(field, error["error"])
                self.assertNotIn("Traceback", stdout + stderr)

    def test_valid_complete_text_input_still_generates_report(self) -> None:
        code, stdout, stderr = self.run_cli(copy.deepcopy(SAMPLE))
        self.assertEqual(code, 0, stdout + stderr)
        self.assertEqual(stderr, "")
        report = json.loads(stdout)
        self.assertEqual(report["data_as_of"], SAMPLE["as_of"])
        self.assertEqual(report["sources"], SAMPLE["sources"])
        self.assertEqual(report["policy_type"], SAMPLE["policy_type"])
        self.assertEqual(report["transmission_hypotheses"][0]["statement"], SAMPLE["transmission_hypotheses"][0]["statement"])


if __name__ == "__main__":
    unittest.main()
