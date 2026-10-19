"""Regression tests for finite minute amount aggregates."""

from __future__ import annotations

import contextlib
import io
import json
import unittest
from unittest.mock import patch

import realtime_quote


def rows(amounts: list[float]) -> list[dict[str, object]]:
    return [
        {
            "time": f"2026-10-19 09:{30 + index:02d}:00",
            "open": 10.0,
            "high": 10.0,
            "low": 10.0,
            "close": 10.0,
            "volume_shares": 10,
            "amount_cny": amount,
        }
        for index, amount in enumerate(amounts)
    ]


def reject_json_constant(value: str):
    raise AssertionError(f"non-standard JSON constant: {value}")


class MinuteAmountAggregateFiniteTests(unittest.TestCase):
    def test_finite_rows_with_overflowing_total_return_cli_error_envelope(self):
        minute_rows = rows([1e308, 1e308])
        self.assertTrue(all(value < float("inf") for value in (row["amount_cny"] for row in minute_rows)))
        with self.assertRaisesRegex(ValueError, "总成交额"):
            realtime_quote.analyze_minute_volume(minute_rows)

        realtime_cache = {
            "sh600000": {
                "code": "600000", "name": "Synthetic", "exchange": "SH",
                "price": 10.0, "open": 10.0, "pre_close": 10.0, "high": 10.0,
                "low": 10.0, "bid_1": 10.0, "ask_1": 10.0, "volume_shares": 20,
                "amount_cny": 20.0, "change_amount": 0.0, "change_percent": 0.0,
                "market_timestamp": "2026-10-19 09:30:00", "retrieved_at_utc": "synthetic",
                "source_name": realtime_quote.SOURCE_NAME,
                "source_interface": realtime_quote.REALTIME_SOURCE_INTERFACE,
            }
        }
        stdout = io.StringIO()
        stderr = io.StringIO()
        with patch.object(realtime_quote, "fetch_realtime_sina", return_value=realtime_cache), \
             patch.object(realtime_quote, "fetch_minute_data_sina", return_value=minute_rows), \
             contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            exit_code = realtime_quote.main(["600000", "--minute", "--json"])

        envelope = json.loads(stdout.getvalue(), parse_constant=reject_json_constant)
        self.assertEqual(exit_code, 1)
        self.assertFalse(envelope["success"])
        self.assertEqual(envelope["results"], [])
        self.assertEqual(envelope["errors"][0]["code"], "600000")
        self.assertIn("总成交额", envelope["errors"][0]["error"])
        self.assertNotIn("Traceback", stdout.getvalue() + stderr.getvalue())

    def test_finite_amount_total_and_volume_buckets_remain_valid(self):
        report = realtime_quote.analyze_minute_volume(rows([12.5, 7.5]))
        self.assertEqual(report["total_amount_cny"], 20.0)
        self.assertEqual(report["total_volume_shares"], 20)
        self.assertEqual(report["distribution"]["open_30min"]["volume_shares"], 20)


if __name__ == "__main__":
    unittest.main(verbosity=2)
