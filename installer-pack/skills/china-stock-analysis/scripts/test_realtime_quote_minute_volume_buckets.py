"""Offline regression tests for minute-volume bucket accounting."""

from __future__ import annotations

import unittest

import realtime_quote


def row(minute: str, volume: int = 100):
    return {
        "time": f"2026-10-19 {minute}:00",
        "open": 10.0,
        "high": 10.0,
        "low": 10.0,
        "close": 10.0,
        "volume_shares": volume,
        "amount_cny": float(volume) * 10,
    }


class MinuteVolumeBucketTests(unittest.TestCase):
    def test_market_session_boundaries_are_assigned_once_and_reconcile(self):
        minutes = (
            "09:25", "09:29", "09:30", "09:59", "10:00", "11:29",
            "13:00", "14:29", "14:30", "15:00",
        )
        report = realtime_quote.analyze_minute_volume([row(minute) for minute in minutes])

        distribution = report["distribution"]
        self.assertEqual(report["bar_count"], 10)
        self.assertEqual(report["total_volume_shares"], 1000)
        self.assertEqual(sum(item["volume_shares"] for item in distribution.values()), 1000)
        self.assertAlmostEqual(sum(item["percent"] for item in distribution.values()), 100.0, places=3)
        self.assertEqual(distribution["open_call_auction"]["volume_shares"], 200)
        self.assertEqual(distribution["open_30min"]["volume_shares"], 200)
        self.assertEqual(distribution["mid_am"]["volume_shares"], 200)
        self.assertEqual(distribution["mid_pm"]["volume_shares"], 200)
        self.assertEqual(distribution["close_30min"]["volume_shares"], 200)

    def test_lunch_break_and_after_close_rows_are_excluded_from_denominator(self):
        report = realtime_quote.analyze_minute_volume(
            [row("09:30"), row("12:00", 500), row("13:00"), row("15:01", 700)]
        )

        self.assertEqual(report["bar_count"], 2)
        self.assertEqual(report["total_volume_shares"], 200)
        self.assertEqual(sum(item["volume_shares"] for item in report["distribution"].values()), 200)
        self.assertAlmostEqual(sum(item["percent"] for item in report["distribution"].values()), 100.0)

    def test_only_nontrading_rows_is_a_clear_error(self):
        with self.assertRaisesRegex(ValueError, "没有有效交易时段"):
            realtime_quote.analyze_minute_volume([row("12:30"), row("15:01")])


if __name__ == "__main__":
    unittest.main(verbosity=2)
