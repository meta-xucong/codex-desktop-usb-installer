#!/usr/bin/env python3
"""Regression tests preventing non-finite market sentiment JSON results."""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import sentiment_monitor  # noqa: E402


class SentimentFiniteResultTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(
            dir=Path(__file__).resolve().parents[4] / "tmp"
        )
        self.root = Path(self.temp_dir.name)
        self.input_path = self.root / "sentiment.json"
        keys = sentiment_monitor.INDICATOR_KEYS
        self.normal = {
            "as_of": "synthetic",
            "sources": ["synthetic source"],
            "indicators": {key: 50 for key in keys},
            "weights": {key: 1 / len(keys) for key in keys},
            "breadth": {"advancing_stocks": 100, "declining_stocks": 80},
            "volume": {"current_volume": 120, "avg_volume": 100, "price_change": 1.5},
            "fund_flow": {"northbound_flow": 1, "main_force_flow": 2, "retail_flow": -0.5},
        }

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def _run(self, data: dict, output: Path | None = None) -> tuple[int, str, str]:
        self.input_path.write_text(json.dumps(data), encoding="utf-8")
        stdout, stderr = io.StringIO(), io.StringIO()
        args = ["--index", "synthetic", "--input", str(self.input_path), "--json"]
        if output is not None:
            args.extend(["--output", str(output)])
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = sentiment_monitor.main(args)
        return result, stdout.getvalue(), stderr.getvalue()

    def _overflow_data(self) -> dict:
        data = json.loads(json.dumps(self.normal))
        data["volume"].update({"current_volume": 1e308, "avg_volume": 1e-308})
        data["fund_flow"].update(
            {"northbound_flow": 1e308, "main_force_flow": 1e308, "retail_flow": 1e308}
        )
        return data

    def _assert_overflow_rejected(self, data: dict) -> tuple[str, str]:
        raw = json.dumps(data).encode("utf-8")
        self.input_path.write_bytes(raw)
        before = hashlib.sha256(raw).hexdigest()
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = sentiment_monitor.main(
                ["--index", "synthetic", "--input", str(self.input_path), "--json"]
            )
        self.assertEqual(result, 1)
        self.assertNotIn("Infinity", stdout.getvalue())
        error = json.loads(stdout.getvalue())
        self.assertFalse(error["success"])
        self.assertIn("有限数值范围", error["error"])
        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(hashlib.sha256(self.input_path.read_bytes()).hexdigest(), before)
        return stdout.getvalue(), stderr.getvalue()

    def test_rejects_overflowed_volume_ratio(self) -> None:
        data = json.loads(json.dumps(self.normal))
        data["volume"].update({"current_volume": 1e308, "avg_volume": 1e-308})
        self._assert_overflow_rejected(data)

    def test_rejects_overflowed_fund_flow_total(self) -> None:
        data = json.loads(json.dumps(self.normal))
        data["fund_flow"].update(
            {"northbound_flow": 1e308, "main_force_flow": 1e308, "retail_flow": 1e308}
        )
        self._assert_overflow_rejected(data)

    def test_overflow_does_not_write_report_file(self) -> None:
        output = self.root / "report.json"
        data = self._overflow_data()
        self.input_path.write_text(json.dumps(data), encoding="utf-8")
        input_hash = hashlib.sha256(self.input_path.read_bytes()).hexdigest()
        result, stdout, stderr = self._run(data, output)
        self.assertEqual(result, 1)
        self.assertNotIn("Infinity", stdout)
        self.assertIn("有限数值范围", json.loads(stdout)["error"])
        self.assertFalse(output.exists())
        self.assertEqual(stderr, "")
        self.assertEqual(hashlib.sha256(self.input_path.read_bytes()).hexdigest(), input_hash)

    def test_normal_finite_data_still_generates_valid_json(self) -> None:
        result, stdout, stderr = self._run(self.normal)
        self.assertEqual(result, 0, stderr)
        report = json.loads(stdout)
        self.assertEqual(report["volume"]["volume_ratio"], 1.2)
        self.assertEqual(report["fund_flow"]["total_flow"], 2.5)
        self.assertNotIn("Infinity", stdout)


if __name__ == "__main__":
    unittest.main()
