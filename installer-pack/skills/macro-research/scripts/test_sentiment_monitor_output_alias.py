#!/usr/bin/env python3
"""Regression tests preventing sentiment reports from replacing their input."""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import sentiment_monitor as analyzer  # noqa: E402


ROOT = Path(__file__).resolve().parents[4]
SCRIPT = Path(__file__).with_name("sentiment_monitor.py")
SAMPLE = {
    "as_of": "synthetic",
    "sources": ["synthetic source"],
    "indicators": {key: 50 for key in analyzer.INDICATOR_KEYS},
    "weights": {key: 1 / len(analyzer.INDICATOR_KEYS) for key in analyzer.INDICATOR_KEYS},
    "breadth": {"advancing_stocks": 100, "declining_stocks": 80},
    "volume": {"current_volume": 120, "avg_volume": 100, "price_change": 1.5},
    "fund_flow": {"northbound_flow": 1, "main_force_flow": 2, "retail_flow": -0.5},
}


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class SentimentOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        self.root = Path(self.temp_dir.name)
        self.input_path = self.root / "sentiment-input.json"
        self.input_path.write_text(json.dumps(SAMPLE, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        self.before = digest(self.input_path)
        self.addCleanup(self.temp_dir.cleanup)

    def run_cli(self, output: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "-B", str(SCRIPT), "--index", "synthetic", "--input", str(self.input_path),
             "--output", str(output), "--json"],
            cwd=self.root, capture_output=True, text=True, check=False, timeout=20,
        )

    def assert_alias_rejected(self, output: Path) -> None:
        result = self.run_cli(output)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("报告已保存", result.stdout + result.stderr)
        self.assertIn("情绪输入文件", result.stdout + result.stderr)
        self.assertEqual(digest(self.input_path), self.before)

    def test_exact_input_alias_fails_before_report_generation(self) -> None:
        with patch.object(analyzer, "generate_sentiment_report") as generate:
            self.assert_alias_rejected(self.input_path)
        generate.assert_not_called()

    def test_normalized_input_alias_is_rejected(self) -> None:
        self.assert_alias_rejected(self.root / "." / self.input_path.name)

    def test_samefile_alias_fails_before_report_generation(self) -> None:
        alias = self.root / "existing-alias.json"
        alias.write_text("synthetic placeholder", encoding="utf-8")
        args = ["--index", "synthetic", "--input", str(self.input_path), "--output", str(alias), "--json"]
        stdout = io.StringIO()
        with patch.object(Path, "samefile", return_value=True), patch.object(
            analyzer, "generate_sentiment_report"
        ) as generate, contextlib.redirect_stdout(stdout):
            result = analyzer.main(args)
        self.assertNotEqual(result, 0)
        self.assertNotIn("报告已保存", stdout.getvalue())
        self.assertIn("情绪输入文件", stdout.getvalue())
        self.assertEqual(digest(self.input_path), self.before)
        generate.assert_not_called()

    def test_distinct_output_remains_valid_and_preserves_input(self) -> None:
        output = self.root / "sentiment-report.json"
        result = self.run_cli(output)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("报告已保存", result.stdout)
        report = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(report["weighted_index"]["value"], 50)
        self.assertEqual(digest(self.input_path), self.before)


if __name__ == "__main__":
    unittest.main()
