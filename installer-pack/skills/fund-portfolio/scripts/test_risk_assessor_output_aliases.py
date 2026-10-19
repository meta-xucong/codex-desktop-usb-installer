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
SCRIPT = SCRIPTS_DIR / "risk_assessor.py"
sys.path.insert(0, str(SCRIPTS_DIR))
import risk_assessor  # noqa: E402


class RiskAssessorOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        temp_root = Path.cwd() / "tmp"
        temp_root.mkdir(exist_ok=True)
        self._temporary = tempfile.TemporaryDirectory(prefix="risk-assessor-alias-", dir=temp_root)
        self.root = Path(self._temporary.name)
        self.portfolio = self.root / "portfolio.json"
        self.portfolio_data = {
            "as_of": "synthetic",
            "sources": ["offline fixture"],
            "allocations": {"AAA": 100},
            "annual_volatility": {"AAA": 20},
            "asset_classes": {"AAA": "equity"},
            "correlation": 0,
            "expected_return": 7,
            "risk_free_rate": 2,
            "drawdown_multiplier": 1.5,
        }
        self.portfolio_bytes = (json.dumps(self.portfolio_data, indent=2) + "\n").encode()
        self.portfolio.write_bytes(self.portfolio_bytes)

    def tearDown(self) -> None:
        self._temporary.cleanup()

    def _run(self, output: Path | None) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["TEMP"] = str(self.root)
        env["TMP"] = str(self.root)
        command = [sys.executable, "-B", str(SCRIPT), "--portfolio", str(self.portfolio), "--json"]
        if output is not None:
            command.extend(["--output", str(output)])
        return subprocess.run(
            command, cwd=str(SCRIPTS_DIR), env=env, capture_output=True,
            text=True, timeout=30, check=False,
        )

    def _assert_rejected_and_preserved(self, output: Path) -> None:
        before = self.portfolio.read_bytes()
        digest = hashlib.sha256(before).hexdigest()
        result = self._run(output)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertFalse(payload["success"])
        self.assertIn("--output must differ from portfolio input", payload["error"])
        self.assertEqual(self.portfolio.read_bytes(), before)
        self.assertEqual(hashlib.sha256(self.portfolio.read_bytes()).hexdigest(), digest)

    def test_cli_rejects_portfolio_as_output_before_overwrite(self) -> None:
        self._assert_rejected_and_preserved(self.portfolio)

    def test_cli_rejects_normalized_portfolio_alias(self) -> None:
        alias = self.root / "nested" / ".." / "portfolio.json"
        self._assert_rejected_and_preserved(alias)

    def test_guard_rejects_samefile_alias(self) -> None:
        alias = self.root / "other-name.json"
        alias.write_text("unrelated fixture", encoding="utf-8")
        with patch.object(Path, "samefile", return_value=True):
            with self.assertRaisesRegex(ValueError, "--output must differ from portfolio input"):
                risk_assessor._reject_output_overwrite(alias, self.portfolio)
        self.assertEqual(self.portfolio.read_bytes(), self.portfolio_bytes)
        self.assertEqual(alias.read_text(encoding="utf-8"), "unrelated fixture")

    def test_guard_fails_closed_when_identity_check_is_denied(self) -> None:
        output = self.root / "identity-unknown.json"
        output.write_text("preserve", encoding="utf-8")
        with patch.object(Path, "samefile", side_effect=PermissionError("synthetic denial")):
            with self.assertRaisesRegex(ValueError, "Cannot verify --output is distinct"):
                risk_assessor._reject_output_overwrite(output, self.portfolio)
        self.assertEqual(output.read_text(encoding="utf-8"), "preserve")
        self.assertEqual(self.portfolio.read_bytes(), self.portfolio_bytes)

    def test_cli_writes_independent_output_and_preserves_source(self) -> None:
        output = self.root / "risk-assessment.json"
        result = self._run(output)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        assessment = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(assessment["data_as_of"], "synthetic")
        self.assertEqual(self.portfolio.read_bytes(), self.portfolio_bytes)

    def test_cli_stdout_remains_valid_without_output_path(self) -> None:
        result = self._run(None)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(result.stdout)["data_as_of"], "synthetic")


if __name__ == "__main__":
    unittest.main()
