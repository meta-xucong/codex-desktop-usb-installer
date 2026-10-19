"""Cycle analysis must not overwrite its reviewed rules file."""

import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import cycle_analyzer as analyzer


ROOT = Path(__file__).resolve().parents[4]
SCRIPT = Path(__file__).with_name("cycle_analyzer.py")
PHASES = (
    "growth_up_inflation_down",
    "growth_up_inflation_up",
    "growth_down_inflation_up",
    "growth_down_inflation_down",
)
RULES = {
    "as_of": "2026-10-19",
    "sources": ["synthetic source"],
    "phase_policies": {
        phase: {"label": phase, "description": "synthetic reviewed policy", "characteristics": ["synthetic"]}
        for phase in PHASES
    },
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class CycleAnalyzerOutputAliasTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        self.root = Path(self.temp_dir.name)
        self.rules_path = self.root / "reviewed-rules.json"
        self.rules_path.write_text(json.dumps(RULES, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        self.addCleanup(self.temp_dir.cleanup)

    def run_cli(self, output_path):
        return subprocess.run(
            [
                sys.executable, "-B", str(SCRIPT),
                "--gdp-growth", "2", "--inflation", "1",
                "--gdp-trend", "1", "--inflation-trend", "2",
                "--data-as-of", "2026-10-19", "--source", "synthetic observation",
                "--rules", str(self.rules_path), "--output", str(output_path), "--json",
            ],
            cwd=self.root, capture_output=True, text=True, check=False, timeout=20,
        )

    def test_exact_rules_output_alias_fails_without_changing_input(self):
        before = digest(self.rules_path)

        result = self.run_cli(self.rules_path)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("不能覆盖周期规则", result.stdout + result.stderr)
        self.assertEqual(digest(self.rules_path), before)

    def test_normalized_rules_alias_fails_without_changing_input(self):
        before = digest(self.rules_path)
        alias = self.root / "." / self.rules_path.name

        result = self.run_cli(alias)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("不能覆盖周期规则", result.stdout + result.stderr)
        self.assertEqual(digest(self.rules_path), before)

    def test_distinct_output_succeeds_and_preserves_rules(self):
        before = digest(self.rules_path)
        output = self.root / "cycle-analysis.json"

        result = self.run_cli(output)

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(output.is_file())
        report = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(report["phase"]["label"], "growth_up_inflation_down")
        self.assertEqual(digest(self.rules_path), before)

    def test_samefile_alias_fails_closed(self):
        alias = self.root / "same-file-alias.json"
        alias.write_bytes(self.rules_path.read_bytes())
        with patch.object(Path, "samefile", return_value=True):
            with self.assertRaisesRegex(ValueError, "不能指向周期规则"):
                analyzer.ensure_distinct_paths(str(self.rules_path), str(alias))

    def test_identity_check_error_fails_closed(self):
        alias = self.root / "identity-uncertain.json"
        alias.write_bytes(b"existing output")
        with patch.object(Path, "samefile", side_effect=PermissionError("identity denied")):
            with self.assertRaisesRegex(ValueError, "无法确认"):
                analyzer.ensure_distinct_paths(str(self.rules_path), str(alias))


if __name__ == "__main__":
    unittest.main()
