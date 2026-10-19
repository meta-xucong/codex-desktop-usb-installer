from __future__ import annotations

import json
import math
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPTS_DIR = Path(__file__).resolve().parent
MODEL = SCRIPTS_DIR / "financial_model.py"


class FinancialModelFiniteResultTests(unittest.TestCase):
    def setUp(self) -> None:
        temp_root = Path.cwd() / "tmp"
        temp_root.mkdir(exist_ok=True)
        self._temporary = tempfile.TemporaryDirectory(
            prefix="venture-financial-model-finite-", dir=temp_root
        )
        self.root = Path(self._temporary.name)

    def tearDown(self) -> None:
        self._temporary.cleanup()

    def _run(
        self, *, overflow: bool = False, initial_users: str = "1",
        output: Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["TEMP"] = str(self.root)
        env["TMP"] = str(self.root)
        command = [
            sys.executable, "-B", str(MODEL),
            "--years", "2",
            "--initial-active-users", initial_users,
            "--annual-net-user-growth-rate", "1e308" if overflow else "10",
            "--monthly-arpu", "1",
            "--cac-per-new-user", "0",
            "--gross-margin", "100",
            "--monthly-churn-rate", "50",
            "--monthly-fixed-costs", "0",
            "--monthly-other-cash-outflow", "0",
            "--initial-cash", "0",
            "--liquidity-buffer-months", "0",
            "--as-of", "synthetic",
            "--source", "offline fixture",
        ]
        if output is not None:
            command.extend(["--output", str(output)])
        else:
            command.append("--json")
        return subprocess.run(
            command, cwd=str(SCRIPTS_DIR), env=env, capture_output=True,
            text=True, timeout=30, check=False,
        )

    @staticmethod
    def _assert_finite_tree(test: unittest.TestCase, value: object) -> None:
        if isinstance(value, float):
            test.assertTrue(math.isfinite(value), value)
        elif isinstance(value, dict):
            for item in value.values():
                FinancialModelFiniteResultTests._assert_finite_tree(test, item)
        elif isinstance(value, list):
            for item in value:
                FinancialModelFiniteResultTests._assert_finite_tree(test, item)

    def test_overflow_json_stdout_is_a_stable_error_without_traceback(self) -> None:
        result = self._run(overflow=True)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        payload = json.loads(result.stdout)
        self.assertFalse(payload["success"])
        self.assertIn("有限数值范围", payload["error"])

    def test_oversized_initial_user_integer_is_a_stable_error(self) -> None:
        result = self._run(initial_users="1" + "0" * 400)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        payload = json.loads(result.stdout)
        self.assertFalse(payload["success"])
        self.assertIn("有限数值", payload["error"])

    def test_overflow_does_not_create_output_file(self) -> None:
        output = self.root / "new-report.json"
        result = self._run(overflow=True, output=output)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(output.exists())

    def test_overflow_preserves_existing_output_file(self) -> None:
        output = self.root / "existing-report.json"
        original = b"preserve this prior report"
        output.write_bytes(original)
        result = self._run(overflow=True, output=output)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(output.read_bytes(), original)

    def test_finite_json_stdout_remains_valid(self) -> None:
        result = self._run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertIn("projections", payload)
        self._assert_finite_tree(self, payload)

    def test_finite_file_output_remains_valid(self) -> None:
        output = self.root / "normal-report.json"
        result = self._run(output=output)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        payload = json.loads(output.read_text(encoding="utf-8"))
        self.assertIn("projections", payload)
        self._assert_finite_tree(self, payload)


if __name__ == "__main__":
    unittest.main()
