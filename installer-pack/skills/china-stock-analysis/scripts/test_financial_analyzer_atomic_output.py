#!/usr/bin/env python3
"""Regression tests for atomic publication of financial analysis reports."""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import financial_analyzer  # noqa: E402


ROOT = Path(__file__).resolve().parents[4]
SAMPLE = {
    "retrieved_at_utc": "synthetic",
    "sources": ["synthetic fixture"],
    "financial_indicators": [{"日期": "2025", "净资产收益率": 10}],
}
SENTINEL = b"PREEXISTING REPORT MUST SURVIVE\n"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def strict_json(text: str):
    def reject_constant(value: str):
        raise ValueError(f"invalid JSON constant: {value}")

    return json.loads(text, parse_constant=reject_constant)


class FinancialAnalyzerAtomicOutputTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        self.root = Path(self.temp_dir.name)
        self.source = self.root / "input.json"
        self.target = self.root / "report.json"
        self.source.write_text(json.dumps(SAMPLE, ensure_ascii=False), encoding="utf-8")
        self.source_hash = digest(self.source)
        self.target.write_bytes(SENTINEL)
        self.addCleanup(self.temp_dir.cleanup)

    def run_cli(self):
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            code = financial_analyzer.main([
                "--input", str(self.source), "--level", "summary", "--output", str(self.target)
            ])
        return code, stdout.getvalue(), stderr.getvalue()

    def assert_no_staging_files(self) -> None:
        self.assertEqual(list(self.root.glob(f".{self.target.name}.*.tmp")), [])

    def test_partial_stage_write_failure_preserves_existing_report_and_input(self) -> None:
        original_write_text = Path.write_text

        def partial_then_fail(path: Path, data: str, *args, **kwargs):
            if path.parent == self.root and path.name.startswith(f".{self.target.name}.") and path.suffix == ".tmp":
                original_write_text(path, data[:12], encoding="utf-8")
                raise OSError("synthetic partial write failure")
            return original_write_text(path, data, *args, **kwargs)

        before_hash = digest(self.target)
        with patch.object(Path, "write_text", autospec=True, side_effect=partial_then_fail):
            code, stdout, stderr = self.run_cli()

        self.assertEqual(code, 1)
        self.assertNotIn("已保存", stdout + stderr)
        self.assertIn("synthetic partial write failure", stderr)
        self.assertEqual(self.target.read_bytes(), SENTINEL)
        self.assertEqual(digest(self.target), before_hash)
        self.assertEqual(digest(self.source), self.source_hash)
        self.assert_no_staging_files()

    def test_replace_failure_preserves_existing_report_and_cleans_stage(self) -> None:
        with patch.object(financial_analyzer.os, "replace", side_effect=OSError("synthetic replace failure")):
            code, stdout, stderr = self.run_cli()

        self.assertEqual(code, 1)
        self.assertNotIn("已保存", stdout + stderr)
        self.assertIn("synthetic replace failure", stderr)
        self.assertEqual(self.target.read_bytes(), SENTINEL)
        self.assertEqual(digest(self.source), self.source_hash)
        self.assert_no_staging_files()

    def test_success_publishes_strict_json_and_preserves_input(self) -> None:
        code, stdout, stderr = self.run_cli()

        self.assertEqual(code, 0, stdout + stderr)
        self.assertIn("已保存", stdout)
        self.assertEqual(stderr, "")
        report = strict_json(self.target.read_text(encoding="utf-8"))
        self.assertEqual(report["input_retrieved_at"], SAMPLE["retrieved_at_utc"])
        self.assertEqual(digest(self.source), self.source_hash)
        self.assert_no_staging_files()


if __name__ == "__main__":
    unittest.main()
