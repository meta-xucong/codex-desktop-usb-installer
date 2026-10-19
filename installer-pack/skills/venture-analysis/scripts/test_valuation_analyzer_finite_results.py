#!/usr/bin/env python3
"""Regression tests preventing non-finite venture valuation JSON results."""

from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import valuation_analyzer  # noqa: E402


class ValuationFiniteResultTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(
            dir=Path(__file__).resolve().parents[4] / "tmp"
        )
        self.assumptions_path = Path(self.temp_dir.name) / "assumptions.json"
        self.assumptions_path.write_text(
            json.dumps(
                {
                    "as_of": "synthetic",
                    "sources": ["synthetic source"],
                    "scorecard_weights": {key: 0.2 for key in valuation_analyzer.SCORE_KEYS},
                    "comparable_valuation_range": [1, 10],
                    "revenue_multiple_range": [1, 2],
                    "scorecard_base_valuation": 100,
                    "scorecard_factor_range": [0.5, 1.5],
                    "range_factor": 0.2,
                }
            ),
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def _run(self, revenue: str, growth: str, years: str) -> tuple[int, str, str]:
        args = [
            "--assumptions", str(self.assumptions_path),
            "--stage", "seed",
            "--industry", "synthetic",
            "--revenue", revenue,
            "--growth-rate", growth,
            "--team-score", "5",
            "--product-score", "5",
            "--market-score", "5",
            "--competition-score", "5",
            "--timing-score", "5",
            "--target-return", "10000",
            "--exit-multiple", "10000",
            "--years", years,
            "--json",
        ]
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = valuation_analyzer.main(args)
        return result, stdout.getvalue(), stderr.getvalue()

    def test_rejects_overflowed_derived_values_without_infinity_json(self) -> None:
        result, stdout, stderr = self._run("1e200", "10000", "100")
        self.assertEqual(result, 1)
        self.assertNotIn("Infinity", stdout)
        self.assertNotIn("NaN", stdout)
        error = json.loads(stdout)
        self.assertFalse(error["success"])
        self.assertIn("有限数值范围", error["error"])
        self.assertEqual(stderr, "")

    def test_normal_finite_inputs_still_generate_valid_json(self) -> None:
        result, stdout, stderr = self._run("1000", "20", "5")
        self.assertEqual(result, 0, stderr)
        report = json.loads(stdout)
        self.assertTrue(report["methods"])
        self.assertNotIn("Infinity", stdout)
        self.assertNotIn("NaN", stdout)


if __name__ == "__main__":
    unittest.main()
