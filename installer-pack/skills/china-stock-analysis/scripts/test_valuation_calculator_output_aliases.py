#!/usr/bin/env python3
"""CLI regression tests protecting valuation input files from output aliases."""

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
import valuation_calculator  # noqa: E402


class ValuationOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(
            dir=Path(__file__).resolve().parents[4] / "tmp"
        )
        self.root = Path(self.temp_dir.name)
        self.data_path = self.root / "data.json"
        self.assumptions_path = self.root / "assumptions.json"
        self.data = {"code": "600000", "basic_info": {"name": "synthetic", "pe_ttm": 12}}
        self.assumptions = {"as_of": "synthetic", "sources": ["synthetic source"]}
        self._write_inputs()

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def _write_inputs(self) -> None:
        self.data_path.write_text(json.dumps(self.data), encoding="utf-8")
        self.assumptions_path.write_text(json.dumps(self.assumptions), encoding="utf-8")

    def _hash(self, path: Path) -> str:
        return hashlib.sha256(path.read_bytes()).hexdigest()

    def _run(self, output: Path) -> tuple[int, str, str]:
        stdout, stderr = io.StringIO(), io.StringIO()
        argv = [
            "--input", str(self.data_path),
            "--assumptions", str(self.assumptions_path),
            "--methods", "relative",
            "--data-as-of", "synthetic",
            "--data-source", "synthetic source",
            "--output", str(output),
        ]
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = valuation_calculator.main(argv)
        return result, stdout.getvalue(), stderr.getvalue()

    def test_rejects_output_alias_to_stock_data(self) -> None:
        before = self._hash(self.data_path)
        result, stdout, stderr = self._run(self.data_path)
        self.assertEqual(result, 1)
        self.assertIn("不能覆盖", stderr)
        self.assertEqual(self._hash(self.data_path), before)

    def test_rejects_normalized_alias_to_reviewed_assumptions(self) -> None:
        before = self._hash(self.assumptions_path)
        alias = self.root / "nested" / ".." / "assumptions.json"
        result, stdout, stderr = self._run(alias)
        self.assertEqual(result, 1)
        self.assertIn("不能覆盖", stderr)
        self.assertEqual(self._hash(self.assumptions_path), before)

    def test_allows_independent_output(self) -> None:
        output = self.root / "report.json"
        before = (self._hash(self.data_path), self._hash(self.assumptions_path))
        result, stdout, stderr = self._run(output)
        self.assertEqual(result, 0, stdout + stderr)
        self.assertIn("methods", json.loads(output.read_text(encoding="utf-8")))
        self.assertEqual(
            (self._hash(self.data_path), self._hash(self.assumptions_path)), before
        )

    def test_rejects_samefile_alias(self) -> None:
        output = self.root / "different-name.json"
        output.write_text("{}", encoding="utf-8")
        before = self._hash(self.data_path)
        with patch.object(valuation_calculator.os.path, "samefile", return_value=True):
            result, stdout, stderr = self._run(output)
        self.assertEqual(result, 1)
        self.assertIn("不能覆盖", stderr)
        self.assertEqual(self._hash(self.data_path), before)

    def test_fails_closed_when_file_identity_check_is_denied(self) -> None:
        output = self.root / "report.json"
        before = self._hash(self.data_path)
        with patch.object(
            valuation_calculator.os.path, "samefile", side_effect=PermissionError("denied")
        ):
            result, stdout, stderr = self._run(output)
        self.assertEqual(result, 1)
        self.assertIn("cannot verify", stderr)
        self.assertFalse(output.exists())
        self.assertEqual(self._hash(self.data_path), before)

    def test_fails_closed_when_path_resolution_fails(self) -> None:
        output = self.root / "report.json"
        before = self._hash(self.data_path)
        with patch.object(Path, "resolve", side_effect=RuntimeError("symlink loop")):
            result, stdout, stderr = self._run(output)
        self.assertEqual(result, 1)
        self.assertIn("cannot verify", stderr)
        self.assertFalse(output.exists())
        self.assertEqual(self._hash(self.data_path), before)


if __name__ == "__main__":
    unittest.main()
