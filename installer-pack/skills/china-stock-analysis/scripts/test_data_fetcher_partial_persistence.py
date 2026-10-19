"""CLI regressions for explicit retention of partial stock data."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
SCRIPT_DIR = Path(__file__).resolve().parent


def result(status: str) -> dict:
    payload = {
        "schema_version": 2,
        "code": "600519",
        "requested_components": ["basic", "financial"],
        "sources": {"basic": "synthetic"},
        "data": {"basic": {"name": "Synthetic Co"}},
        "completeness": {
            "status": status,
            "successful_components": ["basic"] if status != "failed" else [],
            "errors": [{"component": "financial", "error": "synthetic offline failure"}],
        },
    }
    return payload


class DataFetcherPartialPersistenceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="data-fetcher-partial-", dir=ROOT / "tmp")
        self.tmp = Path(self.temporary.name)
        self.env = os.environ.copy()
        self.env.update(
            {
                "TEMP": str(self.tmp),
                "TMP": str(self.tmp),
                "CODEX_DATA_DIR": str(self.tmp / "codex-data"),
                "DATA_FETCHER_TEST_RESULT": json.dumps(result("partial")),
            }
        )
        self.addCleanup(self.temporary.cleanup)

    def run_cli(self, status: str, *arguments: str) -> subprocess.CompletedProcess[str]:
        self.env["DATA_FETCHER_TEST_RESULT"] = json.dumps(result(status))
        runner = (
            "import json, os, sys; "
            f"sys.path.insert(0, {str(SCRIPT_DIR)!r}); "
            "import data_fetcher; "
            "from unittest.mock import patch; "
            "payload=json.loads(os.environ['DATA_FETCHER_TEST_RESULT']); "
            "patcher=patch.object(data_fetcher, '_provider', return_value=object()); "
            "patcher.start(); "
            "patcher2=patch.object(data_fetcher, 'fetch_stock_data', return_value=payload); "
            "patcher2.start(); "
            "raise SystemExit(data_fetcher.main())"
        )
        return subprocess.run(
            [sys.executable, "-c", runner, "--code", "600519", "--data-type", "all", "--no-cache", *arguments],
            cwd=SCRIPT_DIR,
            env=self.env,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_partial_without_opt_in_preserves_existing_output_and_reports_errors(self):
        output = self.tmp / "report.json"
        output.write_text("SENTINEL", encoding="utf-8")

        completed = self.run_cli("partial", "--output", str(output))

        self.assertEqual(completed.returncode, 1)
        self.assertEqual(output.read_text(encoding="utf-8"), "SENTINEL")
        self.assertEqual(completed.stdout, "")
        self.assertIn("--allow-partial", completed.stderr)
        self.assertIn("financial", completed.stderr)
        self.assertIn("synthetic offline failure", completed.stderr)
        self.assertNotIn("Traceback", completed.stderr)
        self.assertNotIn("已保存", completed.stdout + completed.stderr)

    def test_partial_without_opt_in_does_not_create_output(self):
        output = self.tmp / "new-report.json"

        completed = self.run_cli("partial", "--output", str(output))

        self.assertEqual(completed.returncode, 1)
        self.assertFalse(output.exists())
        self.assertEqual(completed.stdout, "")
        self.assertIn("financial", completed.stderr)
        self.assertNotIn("Traceback", completed.stderr)

    def test_partial_without_opt_in_and_without_output_is_not_printed(self):
        completed = self.run_cli("partial")

        self.assertEqual(completed.returncode, 1)
        self.assertEqual(completed.stdout, "")
        self.assertIn("financial", completed.stderr)
        self.assertNotIn('"status": "partial"', completed.stdout + completed.stderr)
        self.assertNotIn("Traceback", completed.stderr)

    def test_partial_with_opt_in_saves_valid_payload_and_succeeds(self):
        output = self.tmp / "partial-report.json"

        completed = self.run_cli("partial", "--allow-partial", "--output", str(output))

        self.assertEqual(completed.returncode, 0)
        self.assertEqual(completed.stdout, f"数据结果已保存到：{output.resolve()}\n")
        self.assertEqual(json.loads(output.read_text(encoding="utf-8")), result("partial"))
        self.assertEqual(completed.stderr, "")

    def test_complete_path_remains_successful(self):
        completed = self.run_cli("complete")

        self.assertEqual(completed.returncode, 0)
        self.assertEqual(json.loads(completed.stdout)["completeness"]["status"], "complete")
        self.assertEqual(completed.stderr, "")

    def test_failed_result_never_writes_an_empty_looking_file(self):
        output = self.tmp / "failed.json"

        completed = self.run_cli("failed", "--allow-partial", "--output", str(output))

        self.assertEqual(completed.returncode, 1)
        self.assertFalse(output.exists())
        self.assertEqual(completed.stdout, "")
        self.assertIn("数据获取失败", completed.stderr)
        self.assertIn("financial", completed.stderr)
        self.assertNotIn("Traceback", completed.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
