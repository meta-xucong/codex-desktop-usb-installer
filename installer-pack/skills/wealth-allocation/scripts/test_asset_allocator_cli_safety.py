#!/usr/bin/env python3
"""CLI regression tests for asset allocator overflow and template protection."""

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
import asset_allocator  # noqa: E402


class AssetAllocatorCliSafetyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(
            dir=Path(__file__).resolve().parents[4] / "tmp"
        )
        self.root = Path(self.temp_dir.name)
        self.template_path = self.root / "policy.json"
        self.template = {
            "risk_level": "synthetic",
            "description": "synthetic policy",
            "as_of": "synthetic",
            "sources": ["synthetic source"],
            "allocation": {"survival": 100, "growth": 0, "aggressive": 0},
            "scenario_annual_return": 0,
            "scenario_max_drawdown": 0,
            "emergency_months": 120,
            "rebalance_threshold": 10,
        }
        self._write_template()

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def _write_template(self) -> None:
        self.template_path.write_text(json.dumps(self.template), encoding="utf-8")

    def _hash(self) -> str:
        return hashlib.sha256(self.template_path.read_bytes()).hexdigest()

    def _run(
        self,
        amount: str = "1000",
        monthly_expense: str = "10",
        output: Path | None = None,
    ) -> tuple[int, str, str]:
        args = [
            "asset_allocator.py",
            "--template", str(self.template_path),
            "--amount", amount,
            "--period-years", "1",
            "--monthly-expense", monthly_expense,
            "--json",
        ]
        if output is not None:
            args.extend(["--output", str(output)])
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(sys, "argv", args), contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = asset_allocator.main()
        return result, stdout.getvalue(), stderr.getvalue()

    def test_rejects_same_template_output_before_plan_generation(self) -> None:
        before = self._hash()
        with patch.object(asset_allocator, "generate_allocation_plan") as generate:
            result, stdout, _ = self._run(output=self.template_path)
        self.assertEqual(result, 1)
        self.assertIn("不能覆盖", json.loads(stdout)["error"])
        generate.assert_not_called()
        self.assertEqual(self._hash(), before)

    def test_rejects_normalized_template_alias(self) -> None:
        before = self._hash()
        alias = self.root / "nested" / ".." / "policy.json"
        result, stdout, _ = self._run(output=alias)
        self.assertEqual(result, 1)
        self.assertIn("不能覆盖", json.loads(stdout)["error"])
        self.assertEqual(self._hash(), before)

    def test_rejects_samefile_alias(self) -> None:
        output = self.root / "other.json"
        output.write_text("{}", encoding="utf-8")
        before = self._hash()
        with patch.object(asset_allocator.os.path, "samefile", return_value=True):
            result, stdout, _ = self._run(output=output)
        self.assertEqual(result, 1)
        self.assertIn("不能覆盖", json.loads(stdout)["error"])
        self.assertEqual(self._hash(), before)

    def test_fails_closed_when_file_identity_check_is_denied(self) -> None:
        output = self.root / "report.json"
        before = self._hash()
        with patch.object(asset_allocator.os.path, "samefile", side_effect=PermissionError("denied")):
            result, stdout, _ = self._run(output=output)
        self.assertEqual(result, 1)
        self.assertIn("cannot verify", json.loads(stdout)["error"])
        self.assertFalse(output.exists())
        self.assertEqual(self._hash(), before)

    def test_fails_closed_when_path_resolution_fails(self) -> None:
        output = self.root / "report.json"
        before = self._hash()
        with patch.object(Path, "resolve", side_effect=RuntimeError("symlink loop")):
            result, stdout, _ = self._run(output=output)
        self.assertEqual(result, 1)
        self.assertIn("cannot verify", json.loads(stdout)["error"])
        self.assertFalse(output.exists())
        self.assertEqual(self._hash(), before)

    def test_allows_independent_output_and_preserves_template(self) -> None:
        output = self.root / "allocation.json"
        before = self._hash()
        result, stdout, stderr = self._run(output=output)
        self.assertEqual(result, 0, stdout + stderr)
        plan = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(plan["allocation"]["survival"]["amount"], 1000)
        self.assertEqual(self._hash(), before)

    def test_rejects_overflowed_amounts_without_invalid_json_or_file(self) -> None:
        output = self.root / "overflow.json"
        result, stdout, stderr = self._run("1e307", "1e307", output)
        self.assertEqual(result, 1)
        self.assertNotIn("Infinity", stdout)
        self.assertNotIn("NaN", stdout)
        self.assertIn("有限数值范围", json.loads(stdout)["error"])
        self.assertFalse(output.exists())
        self.assertEqual(stderr, "")

    def test_normal_finite_json_still_works(self) -> None:
        result, stdout, stderr = self._run()
        self.assertEqual(result, 0, stderr)
        self.assertEqual(json.loads(stdout)["total_amount"], 1000)


if __name__ == "__main__":
    unittest.main()
