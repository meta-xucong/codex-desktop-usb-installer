"""Regression coverage for finite SIP results at and beyond domain extremes."""

from __future__ import annotations

import importlib.util
import json
import math
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("sip_calculator.py")
ROOT = SCRIPT.parents[4]
SPEC = importlib.util.spec_from_file_location("usb_sip_calculator", SCRIPT)
assert SPEC and SPEC.loader
SIP = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SIP)


class SipCalculatorOverflowTests(unittest.TestCase):
    def test_boolean_numeric_inputs_are_rejected(self) -> None:
        invalid_inputs = (
            (True, 1, 0),
            (1000, True, 0),
            (1000, 1, True),
            (1000, 1, False),
        )
        for args in invalid_inputs:
            with self.subTest(args=args), self.assertRaises(ValueError):
                SIP.calculate_regular_sip(*args)

    def test_upper_domain_extreme_fails_as_value_error(self) -> None:
        with self.assertRaisesRegex(ValueError, "超出可表示范围"):
            SIP.calculate_regular_sip(1000, 100, 1000)

    def test_finite_domain_control_matches_independent_annuity_formula(self) -> None:
        result = SIP.calculate_regular_sip(1000, 30, 8)
        monthly_rate = 0.08 / 12
        expected = 1000 * ((1 + monthly_rate) ** 360 - 1) / monthly_rate
        self.assertEqual(result["total_invested"], 360000)
        self.assertTrue(math.isfinite(result["future_value"]))
        self.assertAlmostEqual(result["future_value"], expected, delta=1e-8)
        self.assertTrue(all(math.isfinite(row["value"]) for row in result["details"]))

    def test_extreme_cli_returns_json_error_without_traceback_or_output_file(self) -> None:
        with tempfile.TemporaryDirectory(prefix="sip-overflow-", dir=ROOT / "tmp") as temp:
            destination = Path(temp) / "extreme.json"
            completed = subprocess.run(
                [sys.executable, str(SCRIPT), "--monthly", "1000", "--years", "100",
                 "--expected-return", "1000", "--json", "--output", str(destination)],
                text=True, capture_output=True, check=False,
            )
            self.assertEqual(completed.returncode, 1)
            self.assertNotIn("Traceback", completed.stderr + completed.stdout)
            payload = json.loads(completed.stdout)
            self.assertIn("error", payload)
            self.assertFalse(destination.exists())

    def test_normal_cli_writes_finite_json(self) -> None:
        with tempfile.TemporaryDirectory(prefix="sip-normal-", dir=ROOT / "tmp") as temp:
            destination = Path(temp) / "normal.json"
            completed = subprocess.run(
                [sys.executable, str(SCRIPT), "--monthly", "1000", "--years", "30",
                 "--expected-return", "8", "--json", "--output", str(destination)],
                text=True, capture_output=True, check=False,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            payload = json.loads(destination.read_text(encoding="utf-8"))
            self.assertEqual(payload["total_invested"], 360000)
            self.assertTrue(math.isfinite(payload["future_value"]))
            self.assertNotIn("Infinity", destination.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
