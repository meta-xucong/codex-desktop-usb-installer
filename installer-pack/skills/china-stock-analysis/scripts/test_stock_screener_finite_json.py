from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import pandas as pd

SCRIPTS_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS_DIR))
import stock_screener  # noqa: E402


def reject_json_constant(value: str):
    raise AssertionError(f"non-standard JSON constant: {value}")


class StockScreenerFiniteJsonTests(unittest.TestCase):
    @staticmethod
    def provider(frame):
        return SimpleNamespace(stock_zh_a_spot_em=lambda: frame)

    def test_nonfinite_optional_fields_are_null_and_report_is_strict_json(self) -> None:
        frame = pd.DataFrame([{
            "代码": "600000",
            "名称": "合成样本",
            "市盈率-动态": 12.3,
            "最新价": float("nan"),
            "涨跌幅": float("inf"),
            "市净率": float("-inf"),
            "总市值": float("inf"),
        }])

        report = stock_screener.screen_stocks(
            "all", {}, "pe", 5, self.provider(frame), pd
        )
        encoded = json.dumps(report, ensure_ascii=False, allow_nan=False)
        decoded = json.loads(encoded, parse_constant=reject_json_constant)

        self.assertEqual(decoded["count"], 1)
        row = decoded["results"][0]
        self.assertEqual(row["pe_dynamic"], 12.3)
        for field in ("latest_price", "change_percent", "pb", "market_cap_100m"):
            self.assertIsNone(row[field], field)

    def test_missing_optional_display_columns_remain_null(self) -> None:
        frame = pd.DataFrame([{
            "代码": "600000",
            "名称": "合成样本",
            "市盈率-动态": 12.3,
        }])

        report = stock_screener.screen_stocks(
            "all", {}, "pe", 5, self.provider(frame), pd
        )
        row = json.loads(json.dumps(report, allow_nan=False))["results"][0]

        for field in ("latest_price", "change_percent", "pb", "market_cap_100m"):
            self.assertIsNone(row[field], field)

    def test_nonfinite_sort_rows_are_excluded(self) -> None:
        frame = pd.DataFrame([
            {"代码": "600001", "名称": "finite", "市盈率-动态": 12.3},
            {"代码": "600002", "名称": "nan", "市盈率-动态": float("nan")},
            {"代码": "600003", "名称": "infinite", "市盈率-动态": float("inf")},
        ])

        report = stock_screener.screen_stocks(
            "all", {}, "pe", 5, self.provider(frame), pd
        )

        self.assertEqual(report["count"], 1)
        self.assertEqual([row["code"] for row in report["results"]], ["600001"])

    def test_cli_strict_json_guard_preserves_existing_output(self) -> None:
        temp_root = Path.cwd() / "tmp"
        temp_root.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="stock-screener-strict-json-", dir=temp_root) as temporary:
            output_path = Path(temporary) / "report.json"
            output_path.write_bytes(b"existing report\n")
            before = output_path.read_bytes()
            stderr = io.StringIO()
            with patch.object(stock_screener, "screen_stocks", return_value={"bad": float("nan")}):
                with patch.object(sys, "stderr", stderr):
                    result = stock_screener.main([
                        "--scope", "all", "--sort-by", "pe", "--output", str(output_path)
                    ])
            after = output_path.read_bytes()

        self.assertEqual(result, 1)
        self.assertEqual(after, before)
        error = json.loads(stderr.getvalue(), parse_constant=reject_json_constant)
        self.assertIn("Out of range float values", error["error"])


if __name__ == "__main__":
    unittest.main()
