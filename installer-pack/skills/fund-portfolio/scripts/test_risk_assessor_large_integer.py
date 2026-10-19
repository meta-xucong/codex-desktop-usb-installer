"""Offline regressions for oversized JSON integer validation."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import risk_assessor


SCRIPT = Path(risk_assessor.__file__).resolve()
ROOT = SCRIPT.parents[4]


def valid_portfolio():
    return {
        "as_of": "synthetic",
        "sources": ["offline fixture"],
        "allocations": {"F": 100},
        "annual_volatility": {"F": 10},
        "asset_classes": {"F": "equity"},
        "correlation": 0,
        "expected_return": 7,
        "risk_free_rate": 2,
        "drawdown_multiplier": 1.5,
    }


class RiskAssessorLargeIntegerTests(unittest.TestCase):
    def setUp(self):
        self.temp_root = ROOT / "tmp"
        self.temp_root.mkdir(exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="risk-large-int-", dir=self.temp_root)
        self.directory = Path(self.temporary.name)
        self.addCleanup(self.temporary.cleanup)

    def test_direct_validation_reports_field_error_for_unrepresentable_integer(self):
        data = valid_portfolio()
        data["annual_volatility"]["F"] = 10**400

        with self.assertRaisesRegex(ValueError, "volatility 必须是有限数字"):
            risk_assessor.validate_portfolio_data(data)

    def test_cli_returns_stable_json_error_without_traceback_or_output(self):
        portfolio = self.directory / "portfolio.json"
        output = self.directory / "assessment.json"
        data = valid_portfolio()
        data["annual_volatility"]["F"] = 10**400
        portfolio.write_text(json.dumps(data), encoding="utf-8")
        original = portfolio.read_bytes()
        digest = hashlib.sha256(original).hexdigest()
        env = os.environ.copy()
        env["TEMP"] = str(self.directory)
        env["TMP"] = str(self.directory)

        completed = subprocess.run(
            [sys.executable, "-B", str(SCRIPT), "--portfolio", str(portfolio), "--output", str(output), "--json"],
            cwd=SCRIPT.parent,
            env=env,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )

        self.assertEqual(completed.returncode, 1, completed.stdout + completed.stderr)
        self.assertEqual(json.loads(completed.stdout), {
            "success": False,
            "error": "volatility 必须是有限数字。",
        })
        self.assertNotIn("Traceback", completed.stderr)
        self.assertFalse(output.exists())
        self.assertEqual(hashlib.sha256(portfolio.read_bytes()).hexdigest(), digest)
        self.assertEqual(portfolio.read_bytes(), original)


if __name__ == "__main__":
    unittest.main(verbosity=2)
