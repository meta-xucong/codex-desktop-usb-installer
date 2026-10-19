#!/usr/bin/env python3
"""Regression tests preventing valuation reports from replacing assumptions."""

from __future__ import annotations

import hashlib
import contextlib
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import valuation_analyzer  # noqa: E402


ROOT = Path(__file__).resolve().parents[4]
SCRIPT = Path(__file__).with_name("valuation_analyzer.py")
ASSUMPTIONS = {
    "as_of": "2026-10-19",
    "sources": ["synthetic source"],
    "scorecard_weights": {key: 0.2 for key in valuation_analyzer.SCORE_KEYS},
    "comparable_valuation_range": [100, 300],
    "revenue_multiple_range": [2, 5],
    "scorecard_base_valuation": 200,
    "scorecard_factor_range": [0.5, 1.5],
    "range_factor": 0.2,
}


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class ValuationOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        self.root = Path(self.temp_dir.name)
        self.input_path = self.root / "assumptions.json"
        self.input_path.write_text(json.dumps(ASSUMPTIONS, indent=2) + "\n", encoding="utf-8")
        self.before = digest(self.input_path)
        self.addCleanup(self.temp_dir.cleanup)

    def run_cli(self, output: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable, "-B", str(SCRIPT),
                "--assumptions", str(self.input_path),
                "--stage", "synthetic", "--industry", "synthetic",
                "--revenue", "1000", "--growth-rate", "20",
                "--team-score", "7", "--product-score", "7", "--market-score", "7",
                "--competition-score", "7", "--timing-score", "7",
                "--target-return", "30", "--exit-multiple", "5", "--years", "5",
                "--output", str(output), "--json",
            ],
            cwd=self.root, capture_output=True, text=True, check=False, timeout=20,
        )

    def assert_alias_rejected(self, output: Path) -> None:
        result = self.run_cli(output)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("已保存", result.stdout + result.stderr)
        self.assertIn("输入假设文件", result.stdout + result.stderr)
        self.assertEqual(digest(self.input_path), self.before)

    def test_exact_input_alias_is_rejected_before_generation(self) -> None:
        with patch.object(valuation_analyzer, "generate_valuation_report") as generate:
            self.assert_alias_rejected(self.input_path)
        generate.assert_not_called()

    def test_normalized_input_alias_is_rejected(self) -> None:
        self.assert_alias_rejected(self.root / "." / self.input_path.name)

    def test_samefile_alias_is_rejected_before_generation(self) -> None:
        alias = self.root / "existing-alias.json"
        alias.write_text("synthetic placeholder", encoding="utf-8")
        args = [
            "--assumptions", str(self.input_path),
            "--stage", "synthetic", "--industry", "synthetic",
            "--revenue", "1000", "--growth-rate", "20",
            "--team-score", "7", "--product-score", "7", "--market-score", "7",
            "--competition-score", "7", "--timing-score", "7",
            "--target-return", "30", "--exit-multiple", "5", "--years", "5",
            "--output", str(alias), "--json",
        ]
        stdout = io.StringIO()
        with patch.object(Path, "samefile", return_value=True), patch.object(
            valuation_analyzer, "generate_valuation_report"
        ) as generate, contextlib.redirect_stdout(stdout):
            result = valuation_analyzer.main(args)
        self.assertNotEqual(result, 0)
        self.assertNotIn("已保存", stdout.getvalue())
        self.assertIn("输入假设文件", stdout.getvalue())
        self.assertEqual(digest(self.input_path), self.before)
        generate.assert_not_called()

    def test_distinct_output_is_valid_and_source_is_preserved(self) -> None:
        output = self.root / "valuation-report.json"
        result = self.run_cli(output)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("已保存", result.stdout)
        report = json.loads(output.read_text(encoding="utf-8"))
        self.assertTrue(report["methods"])
        self.assertEqual(digest(self.input_path), self.before)


if __name__ == "__main__":
    unittest.main()
