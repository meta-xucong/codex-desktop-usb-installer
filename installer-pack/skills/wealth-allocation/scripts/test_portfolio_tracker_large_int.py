"""Oversized JSON integers must produce normal portfolio validation errors."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import portfolio_tracker


ROOT = Path(__file__).resolve().parents[4]
SCRIPT = Path(portfolio_tracker.__file__).resolve()
TEMP_ROOT = ROOT / "tmp"


class PortfolioTrackerLargeIntegerTests(unittest.TestCase):
    def setUp(self) -> None:
        self._temporary = tempfile.TemporaryDirectory(
            prefix="portfolio-tracker-large-int-", dir=TEMP_ROOT
        )
        self.root = Path(self._temporary.name)

    def tearDown(self) -> None:
        self._temporary.cleanup()

    def _run(
        self, history: Path, output: Path, *, current: str = "1"
    ) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env.update({"TEMP": str(self.root), "TMP": str(self.root), "PYTHONDONTWRITEBYTECODE": "1"})
        return subprocess.run(
            [
                sys.executable, "-B", str(SCRIPT),
                "--initial", "1", "--current", current,
                "--history", str(history), "--months", "1",
                "--risk-free-rate", "0", "--as-of", "2030-01-01",
                "--source", "synthetic fixture", "--json", "--output", str(output),
            ],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
            check=False,
            timeout=30,
        )

    def test_finite_helper_maps_huge_integer_to_field_specific_value_error(self) -> None:
        with self.assertRaisesRegex(ValueError, r"history\[1\]\.value 必须是有限数值"):
            portfolio_tracker._finite(10**400, "history[1].value")

    def test_cli_returns_json_error_without_traceback_or_output_for_huge_history_integer(self) -> None:
        history = self.root / "history.json"
        output = self.root / "report.json"
        history.write_text('[{"value":1},{"value":' + str(10**400) + '}]\n', encoding="utf-8")
        source_hash = hashlib.sha256(history.read_bytes()).hexdigest()

        result = self._run(history, output)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertFalse(payload["success"])
        self.assertIn("history[1].value", payload["error"])
        self.assertIn("有限数值", payload["error"])
        self.assertNotIn("Traceback", result.stdout + result.stderr)
        self.assertFalse(output.exists())
        self.assertEqual(hashlib.sha256(history.read_bytes()).hexdigest(), source_hash)

    def test_ordinary_finite_history_still_writes_a_report(self) -> None:
        history = self.root / "history.json"
        output = self.root / "report.json"
        history.write_text('[{"value":1},{"value":1.1}]\n', encoding="utf-8")
        source_hash = hashlib.sha256(history.read_bytes()).hexdigest()

        result = self._run(history, output, current="1.1")

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("追踪报告已保存", result.stdout)
        report = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(report["initial_value"], 1.0)
        self.assertEqual(report["current_value"], 1.1)
        self.assertEqual(report["monthly_returns_percent"], [10.0])
        self.assertEqual(hashlib.sha256(history.read_bytes()).hexdigest(), source_hash)


if __name__ == "__main__":
    unittest.main(verbosity=2)
