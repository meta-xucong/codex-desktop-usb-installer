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

import portfolio_tracker


class PortfolioTrackerOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        temp_root = Path.cwd() / "tmp"
        temp_root.mkdir(exist_ok=True)
        self._temporary = tempfile.TemporaryDirectory(
            prefix="portfolio-tracker-output-alias-", dir=temp_root
        )
        self.root = Path(self._temporary.name)
        self.history = self.root / "history.json"
        self.benchmarks = self.root / "benchmarks.json"
        self.history_bytes = b'[{"value":100},{"value":110}]\n'
        self.benchmarks_bytes = b'{"synthetic index":3.5}\n'
        self.history.write_bytes(self.history_bytes)
        self.benchmarks.write_bytes(self.benchmarks_bytes)

    def tearDown(self) -> None:
        self._temporary.cleanup()

    def _run(
        self, output: Path | None, *, benchmarks: Path | None = None,
        overflow: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["TEMP"] = str(self.root)
        env["TMP"] = str(self.root)
        history_path = self.history
        initial, current, months = "100", "110", "1"
        if overflow:
            history_path = self.root / "overflow-history.json"
            history_path.write_text('[{"value":1},{"value":1e308},{"value":1}]\n', encoding="utf-8")
            initial, current, months = "1", "1", "2"
        command = [
            sys.executable,
            "-B",
            str(Path(portfolio_tracker.__file__).resolve()),
            "--initial", initial,
            "--current", current,
            "--history", str(history_path),
            "--months", months,
            "--risk-free-rate", "2",
            "--as-of", "2026-10-19",
            "--source", "synthetic fixture",
            "--json",
        ]
        if benchmarks is not None:
            command.extend(["--benchmarks", str(benchmarks)])
        if output is not None:
            command.extend(["--output", str(output)])
        return subprocess.run(command, capture_output=True, text=True, check=False, env=env)

    def _assert_rejected_without_mutation(self, output: Path, source: Path, *, benchmarks: Path | None = None) -> None:
        source_before = source.read_bytes()
        source_hash = hashlib.sha256(source_before).hexdigest()
        result = self._run(output, benchmarks=benchmarks)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertFalse(payload["success"])
        self.assertIn("--output must differ from input JSON", payload["error"])
        self.assertEqual(source.read_bytes(), source_before)
        self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), source_hash)

    def test_cli_rejects_history_as_output_and_preserves_input(self) -> None:
        self._assert_rejected_without_mutation(self.history, self.history)

    def test_cli_rejects_normalized_benchmarks_alias_and_preserves_input(self) -> None:
        alias = self.root / "nested" / ".." / "benchmarks.json"
        self._assert_rejected_without_mutation(alias, self.benchmarks, benchmarks=self.benchmarks)

    def test_cli_writes_distinct_report_and_preserves_inputs(self) -> None:
        output = self.root / "report.json"
        result = self._run(output, benchmarks=self.benchmarks)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(output.read_text(encoding="utf-8"))["total_return_percent"], 10.0)
        self.assertEqual(self.history.read_bytes(), self.history_bytes)
        self.assertEqual(self.benchmarks.read_bytes(), self.benchmarks_bytes)

    def test_guard_rejects_filesystem_samefile_alias(self) -> None:
        output = self.root / "different-path.json"
        output.write_bytes(b"leave this file unchanged")
        original_output = output.read_bytes()
        with patch.object(Path, "samefile", return_value=True):
            with self.assertRaisesRegex(ValueError, "--output must differ from input JSON"):
                portfolio_tracker._reject_output_overwrite(output, [self.history])
        self.assertEqual(output.read_bytes(), original_output)
        self.assertEqual(self.history.read_bytes(), self.history_bytes)

    def test_guard_fails_closed_when_filesystem_identity_cannot_be_checked(self) -> None:
        output = self.root / "identity-unknown.json"
        output.write_bytes(b"leave this file unchanged")
        original_output = output.read_bytes()
        with patch.object(Path, "samefile", side_effect=PermissionError("synthetic identity error")):
            with self.assertRaisesRegex(ValueError, "Cannot verify --output is distinct"):
                portfolio_tracker._reject_output_overwrite(output, [self.history])
        self.assertEqual(output.read_bytes(), original_output)
        self.assertEqual(self.history.read_bytes(), self.history_bytes)

    def test_guard_fails_closed_when_output_metadata_cannot_be_checked(self) -> None:
        output = self.root / "metadata-unknown.json"
        output.write_bytes(b"leave this file unchanged")
        original_output = output.read_bytes()
        with patch.object(Path, "stat", side_effect=PermissionError("synthetic stat error")):
            with self.assertRaisesRegex(ValueError, "Cannot verify --output is distinct"):
                portfolio_tracker._reject_output_overwrite(output, [self.history])
        self.assertEqual(output.read_bytes(), original_output)
        self.assertEqual(self.history.read_bytes(), self.history_bytes)

    def test_cli_rejects_derived_overflow_in_json_stdout(self) -> None:
        result = self._run(None, overflow=True)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertFalse(payload["success"])
        self.assertIn("有限数值范围", payload["error"])
        self.assertNotIn("Infinity", result.stdout)
        self.assertNotIn("NaN", result.stdout)

    def test_cli_does_not_create_output_for_derived_overflow(self) -> None:
        output = self.root / "overflow-report.json"
        result = self._run(output, overflow=True)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertFalse(output.exists())

    def test_cli_preserves_existing_output_for_derived_overflow(self) -> None:
        output = self.root / "existing-report.json"
        original = b"preserve existing report"
        output.write_bytes(original)
        result = self._run(output, overflow=True)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(output.read_bytes(), original)

    def test_cli_stdout_remains_valid_for_finite_history(self) -> None:
        result = self._run(None)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(result.stdout)["total_return_percent"], 10.0)


if __name__ == "__main__":
    unittest.main()
