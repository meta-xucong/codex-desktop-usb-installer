"""Fund screener report publication must preserve old output on write failure."""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import fund_screener


ROOT = Path(__file__).resolve().parents[4]


def synthetic_report() -> dict:
    return {
        "fetched_at": "2030-01-01T00:00:00+00:00",
        "source_interface": "synthetic.offline.fixture",
        "filters": {"fund_type": None, "min_return_1y": None, "min_return_3y": None, "top": 1},
        "funds": [{"code": "000001", "name": "fixture", "type": "mixed", "return_1y": 5.0, "return_3y": 10.0}],
        "matched_count_before_limit": 1,
        "notice": "synthetic fixture only",
    }


class FundScreenerAtomicWriteTests(unittest.TestCase):
    def setUp(self) -> None:
        temp_root = ROOT / "tmp"
        temp_root.mkdir(exist_ok=True)
        self._temporary = tempfile.TemporaryDirectory(
            prefix="fund-screener-atomic-write-", dir=temp_root
        )
        self.root = Path(self._temporary.name)

    def tearDown(self) -> None:
        self._temporary.cleanup()

    def _invoke(self, output: Path) -> tuple[int, str, str]:
        stdout = io.StringIO()
        stderr = io.StringIO()
        with patch.object(fund_screener, "screen_funds", return_value=synthetic_report()), \
             patch.object(sys, "argv", ["fund_screener.py", "--output", str(output)]), \
             contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            exit_code = fund_screener.main()
        return exit_code, stdout.getvalue(), stderr.getvalue()

    def test_partial_stage_write_failure_preserves_old_report_and_cleans_stage(self) -> None:
        output = self.root / "report.json"
        original = b"previous report that must survive publication failure\n"
        output.write_bytes(original)
        before_hash = hashlib.sha256(original).hexdigest()
        reached = {"value": False}
        real_fdopen = os.fdopen

        class PartialFailWriter:
            def __init__(self, fd: int, *args, **kwargs) -> None:
                self.handle = real_fdopen(fd, *args, **kwargs)

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, traceback):
                self.handle.close()
                return False

            def write(self, data: str) -> int:
                self.handle.write(data[:19])
                self.handle.flush()
                reached["value"] = True
                raise OSError("synthetic partial report write failure")

        stdout = io.StringIO()
        stderr = io.StringIO()
        with patch.object(fund_screener, "screen_funds", return_value=synthetic_report()), \
             patch.object(fund_screener.os, "fdopen", side_effect=PartialFailWriter), \
             patch.object(sys, "argv", ["fund_screener.py", "--output", str(output)]), \
             contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            exit_code = fund_screener.main()

        self.assertTrue(reached["value"])
        self.assertEqual(exit_code, 1)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("synthetic partial report write failure", stderr.getvalue())
        self.assertEqual(output.read_bytes(), original)
        self.assertEqual(hashlib.sha256(output.read_bytes()).hexdigest(), before_hash)
        self.assertEqual(list(self.root.glob(".report.json.*.tmp")), [])

    def test_successfully_publishes_complete_new_report(self) -> None:
        output = self.root / "new-report.json"

        exit_code, stdout, stderr = self._invoke(output)

        self.assertEqual(exit_code, 0, stderr)
        self.assertIn("结果已保存到", stdout)
        self.assertEqual(json.loads(output.read_text(encoding="utf-8")), synthetic_report())
        self.assertEqual(list(self.root.glob(".new-report.json.*.tmp")), [])

    def test_successfully_replaces_complete_existing_report(self) -> None:
        output = self.root / "report.json"
        output.write_bytes(b"old report\n")

        exit_code, stdout, stderr = self._invoke(output)

        self.assertEqual(exit_code, 0, stderr)
        self.assertIn("结果已保存到", stdout)
        self.assertEqual(json.loads(output.read_text(encoding="utf-8")), synthetic_report())
        self.assertEqual(list(self.root.glob(".report.json.*.tmp")), [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
