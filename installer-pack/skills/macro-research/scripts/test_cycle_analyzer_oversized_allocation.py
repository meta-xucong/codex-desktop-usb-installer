"""Cycle rule numeric conversion must fail cleanly for oversized JSON integers."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import cycle_analyzer


ROOT = Path(__file__).resolve().parents[4]
SCRIPT = Path(cycle_analyzer.__file__).resolve()
PHASE_KEYS = cycle_analyzer.PHASE_KEYS


def rules_with_allocation(allocation: dict[str, object] | None = None) -> dict[str, object]:
    policies = {}
    for key in PHASE_KEYS:
        policy = {
            "label": f"Synthetic {key}",
            "description": "synthetic reviewed scenario",
            "characteristics": ["synthetic characteristic"],
        }
        if allocation is not None and key == PHASE_KEYS[0]:
            policy["allocation"] = allocation
            policy["allocation_basis"] = "synthetic reviewed basis"
        policies[key] = policy
    return {
        "as_of": "2026-10-19",
        "sources": ["synthetic rules source"],
        "phase_policies": policies,
    }


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class CycleAnalyzerOversizedAllocationTests(unittest.TestCase):
    def test_cli_401_digit_allocation_fails_without_traceback_or_output(self):
        with tempfile.TemporaryDirectory(prefix="cycle-oversized-allocation-", dir=ROOT / "tmp") as temporary:
            root = Path(temporary)
            rules_path = root / "rules.json"
            output_path = root / "report.json"
            rules_path.write_text(
                json.dumps(rules_with_allocation({"cash": 10 ** 400})) + "\n",
                encoding="utf-8",
            )
            before_hash = sha256(rules_path)
            env = os.environ.copy()
            env["TEMP"] = str(root)
            env["TMP"] = str(root)

            result = subprocess.run(
                [
                    sys.executable, "-B", str(SCRIPT),
                    "--gdp-growth", "2", "--inflation", "0",
                    "--gdp-trend", "1", "--inflation-trend", "1",
                    "--data-as-of", "2026-10-19", "--source", "synthetic observation source",
                    "--rules", str(rules_path), "--output", str(output_path),
                ],
                cwd=ROOT,
                env=env,
                capture_output=True,
                text=True,
                check=False,
                timeout=30,
            )

            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertIn("growth_up_inflation_down.allocation.cash 必须是有限数值", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertEqual(result.stdout, "")
            self.assertFalse(output_path.exists())
            self.assertEqual(sha256(rules_path), before_hash)

    def test_finite_allocation_is_accepted_and_used_in_normal_calculation(self):
        rules = rules_with_allocation({"cash": 40, "bonds": 60})
        report = cycle_analyzer.generate_cycle_analysis(
            2, 0, 1, 1,
            "2026-10-19",
            ["synthetic observation source"],
            rules,
        )
        self.assertEqual(report["observation"]["phase_key"], PHASE_KEYS[0])
        self.assertEqual(report["phase"]["allocation"], {"cash": 40.0, "bonds": 60.0})


if __name__ == "__main__":
    unittest.main()
