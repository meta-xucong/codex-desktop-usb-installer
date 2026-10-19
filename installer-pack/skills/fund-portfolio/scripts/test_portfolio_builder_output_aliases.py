from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPTS_DIR = Path(__file__).resolve().parent
BUILDER = SCRIPTS_DIR / "portfolio_builder.py"
sys.path.insert(0, str(SCRIPTS_DIR))
from portfolio_builder import _reject_output_overwrite  # noqa: E402


class PortfolioBuilderOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        temp_root = Path.cwd() / "tmp"
        temp_root.mkdir(exist_ok=True)
        self._temporary = tempfile.TemporaryDirectory(
            prefix="fund-portfolio-output-alias-", dir=temp_root
        )
        self.root = Path(self._temporary.name)
        self.model_path = self.root / "reviewed-model.json"
        self.model = {
            "risk_level": "moderate",
            "description": "Synthetic offline fixture",
            "as_of": "2026-10-19",
            "sources": ["offline fixture"],
            "allocation": {"broad": 100},
            "scenario_annual_return": 5,
            "scenario_max_drawdown": 20,
            "fund_type_notes": {},
        }
        self.model_bytes = (json.dumps(self.model, ensure_ascii=False, indent=2) + "\n").encode()
        self.model_path.write_bytes(self.model_bytes)

    def tearDown(self) -> None:
        self._temporary.cleanup()

    def _run(self, output: Path | None = None, amount: str = "10000") -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["TEMP"] = str(self.root)
        env["TMP"] = str(self.root)
        command = [
            sys.executable, "-B", str(BUILDER),
            "--model", str(self.model_path),
            "--amount", amount,
            "--period-years", "10",
            "--json",
        ]
        if output is not None:
            command.extend(["--output", str(output)])
        return subprocess.run(
            command, cwd=str(SCRIPTS_DIR), env=env, capture_output=True,
            text=True, timeout=30, check=False,
        )

    def test_cli_rejects_model_as_output_and_preserves_source(self) -> None:
        before = self.model_path.read_bytes()
        digest = hashlib.sha256(before).hexdigest()
        result = self._run(self.model_path)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertIn("--output must differ from model input", payload["error"])
        self.assertEqual(self.model_path.read_bytes(), before)
        self.assertEqual(hashlib.sha256(self.model_path.read_bytes()).hexdigest(), digest)

    def test_cli_rejects_normalized_model_alias(self) -> None:
        before = self.model_path.read_bytes()
        alias = self.root / "nested" / ".." / self.model_path.name
        result = self._run(alias)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertIn("--output must differ from model input", payload["error"])
        self.assertEqual(self.model_path.read_bytes(), before)

    def test_cli_writes_independent_output_and_preserves_model(self) -> None:
        output = self.root / "portfolio.json"
        result = self._run(output)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        report = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(report["total_amount"], 10000)
        self.assertEqual(self.model_path.read_bytes(), self.model_bytes)

    def test_guard_rejects_filesystem_samefile_alias(self) -> None:
        output = self.root / "different-path.json"
        output.write_bytes(b"leave this file unchanged")
        output_before = output.read_bytes()
        with patch.object(Path, "samefile", return_value=True):
            with self.assertRaisesRegex(ValueError, "--output must differ from model input"):
                _reject_output_overwrite(output, self.model_path)
        self.assertEqual(output.read_bytes(), output_before)
        self.assertEqual(self.model_path.read_bytes(), self.model_bytes)

    def test_guard_fails_closed_when_identity_cannot_be_checked(self) -> None:
        output = self.root / "identity-unknown.json"
        output.write_bytes(b"leave this file unchanged")
        output_before = output.read_bytes()
        with patch.object(Path, "samefile", side_effect=PermissionError("synthetic identity error")):
            with self.assertRaisesRegex(ValueError, "Cannot verify --output is distinct"):
                _reject_output_overwrite(output, self.model_path)
        self.assertEqual(output.read_bytes(), output_before)
        self.assertEqual(self.model_path.read_bytes(), self.model_bytes)

    def test_cli_rejects_derived_allocation_overflow_without_writing_output(self) -> None:
        output = self.root / "overflow-report.json"
        result = self._run(output, amount="1e308")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("派生金额超出有限数值范围", result.stdout)
        self.assertFalse(output.exists())
        self.assertEqual(self.model_path.read_bytes(), self.model_bytes)

    def test_cli_preserves_existing_output_when_derived_allocation_overflows(self) -> None:
        output = self.root / "existing-report.json"
        original = b"preserve this prior report"
        output.write_bytes(original)
        result = self._run(output, amount="1e308")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(output.read_bytes(), original)

    def test_cli_rejects_derived_allocation_overflow_in_json_stdout(self) -> None:
        result = self._run(amount="1e308")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertIn("派生金额超出有限数值范围", payload["error"])

    def test_cli_emits_valid_json_stdout_for_finite_input(self) -> None:
        result = self._run(amount="10000")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["allocation"]["broad"]["amount"], 10000)


if __name__ == "__main__":
    unittest.main()
