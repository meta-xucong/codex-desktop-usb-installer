"""Offline regressions for oversized provider numeric values."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS_DIR))
import data_fetcher  # noqa: E402


class FakeFrame:
    empty = False

    def __init__(self, rows):
        self.rows = rows

    def to_dict(self, orient):
        if orient != "records":
            raise ValueError(orient)
        return list(self.rows)


class FakeProvider:
    def stock_individual_info_em(self, *, symbol):
        if symbol != "000001":
            raise AssertionError(f"unexpected stock code: {symbol}")
        return FakeFrame([
            {"item": "股票简称", "value": "合成样本"},
            {"item": "行业", "value": "合成行业"},
            {"item": "总市值", "value": 10**400},
            {"item": "流通市值", "value": 2_500_000_000},
            {"item": "总股本", "value": "1,234"},
            {"item": "市盈率(动态)", "value": "12.5"},
        ])


class DataFetcherSafeFloatOverflowTests(unittest.TestCase):
    def test_safe_float_maps_oversized_integer_to_none(self):
        self.assertIsNone(data_fetcher.safe_float(10**400))

    def test_safe_float_preserves_finite_and_invalid_value_behavior(self):
        self.assertEqual(data_fetcher.safe_float(12), 12.0)
        self.assertEqual(data_fetcher.safe_float("1,234.5亿"), 1234.5)
        for value in (None, "", "--", True, "not-a-number", float("nan"), float("inf")):
            with self.subTest(value=value):
                self.assertIsNone(data_fetcher.safe_float(value))

    def test_stock_info_returns_record_with_oversized_optional_market_cap_null(self):
        info = data_fetcher.get_stock_info("000001", FakeProvider())

        self.assertEqual(info["name"], "合成样本")
        self.assertEqual(info["industry"], "合成行业")
        self.assertIsNone(info["market_cap"])
        self.assertEqual(info["float_cap"], 2_500_000_000.0)
        self.assertEqual(info["total_shares"], 1234.0)
        self.assertEqual(info["pe_ttm"], 12.5)


if __name__ == "__main__":
    unittest.main(verbosity=2)
