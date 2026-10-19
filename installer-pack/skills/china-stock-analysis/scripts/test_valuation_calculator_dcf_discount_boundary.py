#!/usr/bin/env python3
"""Regression tests for DCF discount rates at or below -100 percent."""

from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import valuation_calculator  # noqa: E402


class DcfDiscountBoundaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(
            dir=Path(__file__).resolve().parents[4] / "tmp"
        )
        self.root = Path(self.temp_dir.name)
        self.data_path = self.root / "data.json"
        self.data_path.write_text("{}", encoding="utf-8")
        self.assumptions_path = self.root / "assumptions.json"

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def _write_assumptions(
        self,
        discount_rate: float,
        terminal_growth: float,
        base_free_cash_flow: float = 100,
        cash_flow_growth: float = 0,
        forecast_years: int = 1,
    ) -> None:
        self.assumptions_path.write_text(
            json.dumps(
                {
                    "as_of": "synthetic",
                    "sources": ["synthetic source"],
                    "dcf": {
                        "base_free_cash_flow": base_free_cash_flow,
                        "cash_flow_growth": cash_flow_growth,
                        "discount_rate": discount_rate,
                        "terminal_growth": terminal_growth,
                        "forecast_years": forecast_years,
                        "total_shares": 10,
                        "basis": "synthetic",
                    },
                }
            ),
            encoding="utf-8",
        )

    def _run(self, output: Path) -> tuple[int, str, str]:
        argv = [
            "--input", str(self.data_path),
            "--assumptions", str(self.assumptions_path),
            "--methods", "dcf",
            "--data-as-of", "synthetic",
            "--data-source", "synthetic source",
            "--output", str(output),
            "--json",
        ]
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = valuation_calculator.main(argv)
        return result, stdout.getvalue(), stderr.getvalue()

    def test_rejects_discount_rates_at_or_below_minus_100_without_traceback(self) -> None:
        for discount, terminal in [(-100, -101), (-101, -102)]:
            with self.subTest(discount=discount):
                self._write_assumptions(discount, terminal)
                output = self.root / f"report-{discount}.json"
                result, stdout, stderr = self._run(output)
                self.assertEqual(result, 1)
                self.assertNotIn("Traceback", stdout + stderr)
                error = json.loads(stdout)
                self.assertFalse(error["success"])
                self.assertIn("大于 -100%", error["error"])
                self.assertFalse(output.exists())

    def test_valid_dcf_discount_still_generates_report(self) -> None:
        self._write_assumptions(10, 2)
        output = self.root / "report-valid.json"
        result, stdout, stderr = self._run(output)
        self.assertEqual(result, 0, stdout + stderr)
        report = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(report["methods"]["dcf"]["method"], "dcf_scenario")

    def test_rejects_overflowed_dcf_values_without_writing_report(self) -> None:
        self._write_assumptions(20, 2, base_free_cash_flow=1e308, cash_flow_growth=1000, forecast_years=30)
        output = self.root / "report-overflow.json"
        result, stdout, stderr = self._run(output)
        self.assertEqual(result, 1)
        self.assertNotIn("Infinity", stdout)
        error = json.loads(stdout)
        self.assertFalse(error["success"])
        self.assertIn("有限数值范围", error["error"])
        self.assertFalse(output.exists())
        self.assertEqual(stderr, "")


if __name__ == "__main__":
    unittest.main()
