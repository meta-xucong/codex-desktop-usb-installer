"""Oversized numeric inputs should fail as field-level validation errors."""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import business_evaluator


ROOT = Path(__file__).resolve().parents[4]
SCRIPT = Path(business_evaluator.__file__).resolve()


def valid_input(tam):
    return {
        "project_name": "synthetic venture",
        "industry": "software",
        "as_of": "2026-10-19",
        "sources": ["synthetic source"],
        "evidence": ["synthetic evidence"],
        "market": {
            "tam": tam,
            "sam": 500,
            "som": 100,
            "currency": "USD",
            "as_of": "2026-10-19",
            "sources": ["synthetic market source"],
            "basis": "synthetic market basis",
        },
        "criteria": [{
            "id": "market-fit",
            "label": "Market fit",
            "weight": 1,
            "score": 8,
            "basis": "synthetic user-reviewed score",
            "risks": [],
        }],
    }


class BusinessEvaluatorNumericOverflowTests(unittest.TestCase):
    def test_number_reports_field_error_for_huge_integer(self):
        with self.assertRaisesRegex(ValueError, r"market\.tam.*有限数值"):
            business_evaluator._number(10 ** 400, "market.tam", 0)

    def test_cli_huge_integer_fails_without_traceback_or_output_file(self):
        with tempfile.TemporaryDirectory(prefix="business-evaluator-overflow-", dir=ROOT / "tmp") as temporary:
            root = Path(temporary)
            input_path = root / "input.json"
            output_path = root / "evaluation.json"
            input_path.write_text(json.dumps(valid_input(10 ** 400)), encoding="utf-8")

            result = subprocess.run(
                [sys.executable, "-B", str(SCRIPT), "--input", str(input_path), "--output", str(output_path)],
                cwd=ROOT, capture_output=True, text=True, check=False, timeout=30,
            )

            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertIn("market.tam 必须是有限数值", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertEqual(result.stdout, "")
            self.assertFalse(output_path.exists())

    def test_finite_number_is_accepted(self):
        self.assertEqual(business_evaluator._number(12.5, "market.tam", 0), 12.5)


if __name__ == "__main__":
    unittest.main()
