"""Offline regressions for financial-history cache horizon separation."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import data_fetcher


ROOT = Path(__file__).resolve().parents[4]
REPORTS = ("balance_sheet", "income_statement", "cash_flow")


class FakeFrame:
    def __init__(self, rows):
        self.rows = rows

    @property
    def empty(self):
        return not self.rows

    def head(self, limit):
        return FakeFrame(self.rows[:limit])

    def to_dict(self, orient):
        if orient != "records":
            raise ValueError(orient)
        return list(self.rows)


class FakeFinancialProvider:
    __version__ = "synthetic"

    def __init__(self, rows=40):
        self.rows = rows
        self.calls = 0

    def _frame(self, prefix):
        self.calls += 1
        return FakeFrame([{"period": f"{prefix}-{index}", "value": index} for index in range(self.rows)])

    def stock_balance_sheet_by_report_em(self, **kwargs):
        return self._frame("balance")

    def stock_profit_sheet_by_report_em(self, **kwargs):
        return self._frame("income")

    def stock_cash_flow_sheet_by_report_em(self, **kwargs):
        return self._frame("cash")

    def stock_financial_abstract(self, **kwargs):
        return self._frame("indicator")


class FinancialCacheYearsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="financial-cache-years-", dir=ROOT / "tmp")
        self.cache_dir = Path(self.temporary.name) / "cache"
        self.env_patch = patch.dict("os.environ", {"CHINA_STOCK_CACHE_DIR": str(self.cache_dir)})
        self.env_patch.start()
        self.addCleanup(self._cleanup)

    def _cleanup(self):
        self.env_patch.stop()
        self.temporary.cleanup()

    def _counts(self, result):
        return {key: len(result["financial_data"][key]) for key in REPORTS}

    def test_one_year_cache_does_not_satisfy_ten_year_request(self):
        first_provider = FakeFinancialProvider()
        one_year = data_fetcher.fetch_stock_data("000001", "financial", years=1, provider=first_provider)
        second_provider = FakeFinancialProvider()
        ten_year = data_fetcher.fetch_stock_data("000001", "financial", years=10, provider=second_provider)

        self.assertEqual(self._counts(one_year), {key: 4 for key in REPORTS})
        self.assertFalse(ten_year["cache"]["hit"])
        self.assertEqual(self._counts(ten_year), {key: 40 for key in REPORTS})
        self.assertEqual(second_provider.calls, 4)

    def test_same_horizon_hits_cache_without_provider_calls(self):
        data_fetcher.fetch_stock_data("000002", "financial", years=3, provider=FakeFinancialProvider())
        unused_provider = FakeFinancialProvider()

        cached = data_fetcher.fetch_stock_data("000002", "financial", years=3, provider=unused_provider)

        self.assertTrue(cached["cache"]["hit"])
        self.assertEqual(self._counts(cached), {key: 12 for key in REPORTS})
        self.assertEqual(unused_provider.calls, 0)

    def test_ten_year_cache_does_not_overstate_one_year_request(self):
        data_fetcher.fetch_stock_data("000003", "financial", years=10, provider=FakeFinancialProvider())
        one_year_provider = FakeFinancialProvider()

        one_year = data_fetcher.fetch_stock_data("000003", "financial", years=1, provider=one_year_provider)

        self.assertFalse(one_year["cache"]["hit"])
        self.assertEqual(self._counts(one_year), {key: 4 for key in REPORTS})
        self.assertEqual(one_year_provider.calls, 4)

    def test_nonfinancial_cache_key_ignores_years_and_bool_years_are_rejected(self):
        self.assertEqual(
            data_fetcher.get_cache_path("000004", "basic", years=1),
            data_fetcher.get_cache_path("000004", "basic", years=10),
        )
        for years in (True, False):
            with self.subTest(years=years):
                with self.assertRaisesRegex(ValueError, "years 必须"):
                    data_fetcher.fetch_stock_data("000004", "financial", years=years, use_cache=False,
                                                  provider=FakeFinancialProvider())

    def test_all_cache_key_tracks_year_horizon_and_rejects_invalid_values(self):
        one_year = data_fetcher.get_cache_path("000005", "all", years=1)
        ten_years = data_fetcher.get_cache_path("000005", "all", years=10)

        self.assertNotEqual(one_year, ten_years)
        self.assertIn("_y1_", one_year.name)
        self.assertIn("_y10_", ten_years.name)
        for years in (True, 0, 11, 1.5):
            with self.subTest(years=years):
                with self.assertRaisesRegex(ValueError, "years 必须"):
                    data_fetcher.get_cache_path("000005", "all", years=years)


if __name__ == "__main__":
    unittest.main(verbosity=2)
