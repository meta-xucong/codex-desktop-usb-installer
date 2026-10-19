#!/usr/bin/env python3
"""CLI regression tests protecting financial input JSON from output aliases."""

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


class FinancialAnalyzerOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(
            dir=Path(__file__).resolve().parents[4] / "tmp"
        )
        self.root = Path(self.temp_dir.name)
        self.input_path = self.root / "data.json"
        self.input_value = {
            "retrieved_at_utc": "synthetic",
            "sources": ["synthetic source"],
            "financial_indicators": [{"日期": "2025", "净资产收益率": 10}],
        }
        self.input_path.write_text(json.dumps(self.input_value, ensure_ascii=False), encoding="utf-8")

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def _hash(self) -> str:
        return hashlib.sha256(self.input_path.read_bytes()).hexdigest()

    def _run(self, output: Path) -> tuple[int, str, str]:
        stdout, stderr = io.StringIO(), io.StringIO()
        argv = ["--input", str(self.input_path), "--level", "summary", "--output", str(output)]
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = financial_analyzer.main(argv)
        return result, stdout.getvalue(), stderr.getvalue()

    def test_rejects_exact_input_alias(self) -> None:
        before = self._hash()
        result, _, stderr = self._run(self.input_path)
        self.assertEqual(result, 1)
        self.assertIn("不能覆盖", stderr)
        self.assertEqual(self._hash(), before)

    def test_rejects_normalized_input_alias(self) -> None:
        before = self._hash()
        alias = self.root / "nested" / ".." / "data.json"
        result, _, stderr = self._run(alias)
        self.assertEqual(result, 1)
        self.assertIn("不能覆盖", stderr)
        self.assertEqual(self._hash(), before)

    def test_allows_separate_report_output(self) -> None:
        output = self.root / "analysis.json"
        before = self._hash()
        result, stdout, stderr = self._run(output)
        self.assertEqual(result, 0, stdout + stderr)
        self.assertTrue(output.is_file())
        self.assertTrue(json.loads(output.read_text(encoding="utf-8")))
        self.assertEqual(self._hash(), before)

    def test_rejects_samefile_alias(self) -> None:
        output = self.root / "other-name.json"
        output.write_text("{}", encoding="utf-8")
        before = self._hash()
        with patch.object(financial_analyzer.os.path, "samefile", return_value=True):
            result, _, stderr = self._run(output)
        self.assertEqual(result, 1)
        self.assertIn("不能覆盖", stderr)
        self.assertEqual(self._hash(), before)

    def test_fails_closed_when_identity_check_is_denied(self) -> None:
        output = self.root / "analysis.json"
        before = self._hash()
        with patch.object(
            financial_analyzer.os.path, "samefile", side_effect=PermissionError("denied")
        ):
            result, _, stderr = self._run(output)
        self.assertEqual(result, 1)
        self.assertIn("cannot verify", stderr)
        self.assertFalse(output.exists())
        self.assertEqual(self._hash(), before)

    def test_fails_closed_when_path_resolution_fails(self) -> None:
        output = self.root / "analysis.json"
        before = self._hash()
        with patch.object(Path, "resolve", side_effect=RuntimeError("symlink loop")):
            result, _, stderr = self._run(output)
        self.assertEqual(result, 1)
        self.assertIn("cannot verify", stderr)
        self.assertFalse(output.exists())
        self.assertEqual(self._hash(), before)


if __name__ == "__main__":
    unittest.main()
