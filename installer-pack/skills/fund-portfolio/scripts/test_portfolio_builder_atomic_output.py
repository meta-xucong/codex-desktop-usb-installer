#!/usr/bin/env python3
"""Atomic output regressions for the Fund Portfolio builder CLI."""

from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[4]
SOURCE = Path(__file__).with_name("portfolio_builder.py")
MODEL = {
    "risk_level": "moderate",
    "description": "Synthetic offline fixture",
    "as_of": "2026-10-19",
    "sources": ["offline synthetic fixture"],
    "allocation": {"broad": 100},
    "scenario_annual_return": 5,
    "scenario_max_drawdown": 20,
    "fund_type_notes": {},
}
SENTINEL = b"PREVIOUS_REPORT_MUST_SURVIVE"


def load_builder(name: str):
    spec = importlib.util.spec_from_file_location(name, SOURCE)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def strict_json(text: str):
    def reject_constant(value: str):
        raise ValueError(f"invalid JSON constant: {value}")

    return json.loads(text, parse_constant=reject_constant)


class PartialWriteHandle:
    def __init__(self, wrapped):
        self.wrapped = wrapped

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_value, traceback):
        self.wrapped.close()

    def write(self, value):
        self.wrapped.write(value[:24])
        self.wrapped.flush()
        raise OSError("synthetic partial stage write failure")

    def flush(self):
        return self.wrapped.flush()

    def fileno(self):
        return self.wrapped.fileno()


class PortfolioBuilderAtomicOutputTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(prefix="portfolio-builder-atomic-", dir=ROOT / "tmp")
        self.root = Path(self.temp_dir.name)
        self.model_path = self.root / "model.json"
        self.output_path = self.root / "report.json"
        self.model_bytes = (json.dumps(MODEL, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
        self.model_path.write_bytes(self.model_bytes)
        self.model_hash = sha(self.model_path)
        self.addCleanup(self.temp_dir.cleanup)
        self.builder = load_builder(f"portfolio_builder_atomic_test_{id(self)}")

    def run_cli(self, output_path: Path):
        stdout, stderr = io.StringIO(), io.StringIO()
        argv = [
            "portfolio_builder.py", "--model", str(self.model_path), "--amount", "10000",
            "--period-years", "10", "--json", "--output", str(output_path),
        ]
        with patch.object(sys, "argv", argv), contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            code = self.builder.main()
        return code, stdout.getvalue(), stderr.getvalue()

    def inject_partial_stage_write(self):
        real_fdopen = self.builder.os.fdopen

        def partial_fdopen(descriptor, *args, **kwargs):
            return PartialWriteHandle(real_fdopen(descriptor, *args, **kwargs))

        return patch.object(self.builder.os, "fdopen", side_effect=partial_fdopen)

    def assert_no_stage_files(self) -> None:
        self.assertEqual(list(self.root.glob(f".{self.output_path.name}.*.tmp")), [])

    def assert_failed_without_success(self, output_path: Path, code: int, stdout: str, stderr: str) -> None:
        self.assertEqual(code, 1)
        self.assertNotIn("方案已保存", stdout + stderr)
        self.assertEqual(stderr, "")
        error = json.loads(stdout)
        self.assertIn("error", error)
        self.assertEqual(sha(self.model_path), self.model_hash)
        self.assert_no_stage_files()

    def test_partial_stage_write_preserves_existing_output_and_model(self) -> None:
        self.output_path.write_bytes(SENTINEL)
        before_hash = sha(self.output_path)

        with self.inject_partial_stage_write():
            code, stdout, stderr = self.run_cli(self.output_path)

        self.assert_failed_without_success(self.output_path, code, stdout, stderr)
        self.assertEqual(self.output_path.read_bytes(), SENTINEL)
        self.assertEqual(sha(self.output_path), before_hash)

    def test_replace_failure_preserves_existing_output_and_model(self) -> None:
        self.output_path.write_bytes(SENTINEL)
        before_hash = sha(self.output_path)

        with patch.object(self.builder.os, "replace", side_effect=OSError("synthetic replace failure")):
            code, stdout, stderr = self.run_cli(self.output_path)

        self.assert_failed_without_success(self.output_path, code, stdout, stderr)
        self.assertEqual(self.output_path.read_bytes(), SENTINEL)
        self.assertEqual(sha(self.output_path), before_hash)

    def test_partial_stage_failure_does_not_create_new_output(self) -> None:
        with self.inject_partial_stage_write():
            code, stdout, stderr = self.run_cli(self.output_path)

        self.assert_failed_without_success(self.output_path, code, stdout, stderr)
        self.assertFalse(self.output_path.exists())

    def test_success_publishes_strict_json_and_preserves_model(self) -> None:
        code, stdout, stderr = self.run_cli(self.output_path)

        self.assertEqual(code, 0, stdout + stderr)
        self.assertIn("方案已保存", stdout)
        self.assertEqual(stderr, "")
        report = strict_json(self.output_path.read_text(encoding="utf-8"))
        self.assertEqual(report["model_as_of"], MODEL["as_of"])
        self.assertEqual(sha(self.model_path), self.model_hash)
        self.assert_no_stage_files()


if __name__ == "__main__":
    unittest.main()
