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

import rebalance_planner


class RebalanceOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        temp_root = Path.cwd() / "tmp"
        temp_root.mkdir(exist_ok=True)
        self._temporary = tempfile.TemporaryDirectory(
            prefix="rebalance-output-alias-", dir=temp_root
        )
        self.root = Path(self._temporary.name)
        self.current = self.root / "current.json"
        self.target = self.root / "target.json"
        self.current_bytes = b'{"A":60,"B":40}\n'
        self.target_bytes = b'{"A":50,"B":50}\n'
        self.current.write_bytes(self.current_bytes)
        self.target.write_bytes(self.target_bytes)

    def tearDown(self) -> None:
        self._temporary.cleanup()

    def _run(
        self, output: Path | None, *, value: str = "10000", threshold: str = "1",
        transaction_cost_rate: str = "0.1",
    ) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["TEMP"] = str(self.root)
        env["TMP"] = str(self.root)
        command = [
            sys.executable, "-B", str(Path(rebalance_planner.__file__).resolve()),
            "--current", str(self.current), "--target", str(self.target),
            "--value", value, "--threshold", threshold,
            "--transaction-cost-rate", transaction_cost_rate, "--json",
        ]
        if output is not None:
            command.extend(["--output", str(output)])
        return subprocess.run(
            command,
            capture_output=True, text=True, check=False, env=env,
        )

    def _assert_rejected_without_mutation(self, output: Path, source: Path) -> None:
        current_before = hashlib.sha256(self.current.read_bytes()).hexdigest()
        target_before = hashlib.sha256(self.target.read_bytes()).hexdigest()
        source_bytes = source.read_bytes()

        result = self._run(output)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertFalse(payload["success"])
        self.assertIn("--output must differ from input JSON", payload["error"])
        self.assertEqual(source.read_bytes(), source_bytes)
        self.assertEqual(hashlib.sha256(self.current.read_bytes()).hexdigest(), current_before)
        self.assertEqual(hashlib.sha256(self.target.read_bytes()).hexdigest(), target_before)

    def test_cli_rejects_current_as_output_and_preserves_both_inputs(self) -> None:
        self._assert_rejected_without_mutation(self.current, self.current)

    def test_cli_rejects_target_normalized_alias_and_preserves_both_inputs(self) -> None:
        alias = self.root / "nested" / ".." / "target.json"
        self._assert_rejected_without_mutation(alias, self.target)

    def test_cli_writes_report_to_distinct_output_and_preserves_inputs(self) -> None:
        output = self.root / "plan.json"
        result = self._run(output)

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(output.read_text(encoding="utf-8"))["portfolio_value"], 10000.0)
        self.assertEqual(self.current.read_bytes(), self.current_bytes)
        self.assertEqual(self.target.read_bytes(), self.target_bytes)

    def test_guard_rejects_filesystem_samefile_alias(self) -> None:
        alias = self.root / "alias.json"
        alias.write_text("separate path for samefile simulation", encoding="utf-8")

        with patch.object(Path, "samefile", return_value=True):
            with self.assertRaisesRegex(ValueError, "--output must differ"):
                rebalance_planner._reject_output_overwrite(alias, [self.current])

        self.assertEqual(self.current.read_bytes(), self.current_bytes)
        self.assertEqual(alias.read_text(encoding="utf-8"), "separate path for samefile simulation")

    def _set_overflow_allocations(self) -> None:
        self.current.write_text('{"A": 100}\n', encoding="utf-8")
        self.target.write_text('{"B": 100}\n', encoding="utf-8")

    def test_cli_rejects_derived_overflow_without_invalid_json_or_new_output(self) -> None:
        self._set_overflow_allocations()
        output = self.root / "overflow-plan.json"
        result = self._run(output, value="1e308", threshold="0", transaction_cost_rate="1")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertFalse(payload["success"])
        self.assertIn("有限数值范围", payload["error"])
        self.assertNotIn("Infinity", result.stdout)
        self.assertNotIn("NaN", result.stdout)
        self.assertFalse(output.exists())

    def test_cli_preserves_existing_output_on_derived_overflow(self) -> None:
        self._set_overflow_allocations()
        output = self.root / "existing-plan.json"
        original = b"preserve previous plan"
        output.write_bytes(original)
        result = self._run(output, value="1e308", threshold="0", transaction_cost_rate="1")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(output.read_bytes(), original)

    def test_cli_stdout_remains_valid_for_finite_input(self) -> None:
        result = self._run(None)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(result.stdout)["portfolio_value"], 10000.0)


if __name__ == "__main__":
    unittest.main()
