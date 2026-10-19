"""Policy scenario output must not overwrite its source evidence file."""

import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import policy_analyzer as analyzer


ROOT = Path(__file__).resolve().parents[4]
SCRIPT = Path(__file__).with_name("policy_analyzer.py")
SAMPLE = {
    "as_of": "2026-10-19",
    "sources": ["synthetic policy source"],
    "policy_type": "synthetic",
    "action": "synthetic action",
    "context": "synthetic context",
    "evidence": ["synthetic evidence"],
    "transmission_hypotheses": [{
        "statement": "synthetic hypothesis",
        "basis": "synthetic basis",
        "confidence": "low; synthetic",
    }],
    "scenario_implications": [{
        "scenario": "synthetic condition",
        "implication": "synthetic implication",
        "monitor": ["synthetic indicator"],
    }],
    "risks": ["synthetic risk"],
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class PolicyAnalyzerOutputAliasTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        self.root = Path(self.temp_dir.name)
        self.input_path = self.root / "policy-scenario.json"
        self.input_path.write_text(json.dumps(SAMPLE, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        self.addCleanup(self.temp_dir.cleanup)

    def run_cli(self, output_path):
        return subprocess.run(
            [sys.executable, "-B", str(SCRIPT), "--input", str(self.input_path), "--output", str(output_path), "--json"],
            cwd=self.root, capture_output=True, text=True, check=False, timeout=20,
        )

    def test_exact_input_output_alias_fails_without_changing_input(self):
        before = digest(self.input_path)

        result = self.run_cli(self.input_path)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("不能覆盖政策输入", result.stdout + result.stderr)
        self.assertEqual(digest(self.input_path), before)

    def test_normalized_path_alias_fails_without_changing_input(self):
        before = digest(self.input_path)
        alias = self.root / "." / self.input_path.name

        result = self.run_cli(alias)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("不能覆盖政策输入", result.stdout + result.stderr)
        self.assertEqual(digest(self.input_path), before)

    def test_distinct_output_succeeds_and_preserves_input(self):
        before = digest(self.input_path)
        output = self.root / "policy-report.json"

        result = self.run_cli(output)

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(output.is_file())
        report = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(report["data_as_of"], SAMPLE["as_of"])
        self.assertEqual(digest(self.input_path), before)

    def test_samefile_alias_fails_closed(self):
        alias = self.root / "same-file-alias.json"
        alias.write_bytes(self.input_path.read_bytes())
        with patch.object(Path, "samefile", return_value=True):
            with self.assertRaisesRegex(ValueError, "不能指向政策输入"):
                analyzer.ensure_distinct_paths(str(self.input_path), str(alias))

    def test_identity_check_error_fails_closed(self):
        alias = self.root / "identity-uncertain.json"
        alias.write_bytes(b"existing output")
        with patch.object(Path, "samefile", side_effect=PermissionError("identity denied")):
            with self.assertRaisesRegex(ValueError, "无法确认"):
                analyzer.ensure_distinct_paths(str(self.input_path), str(alias))


if __name__ == "__main__":
    unittest.main()
