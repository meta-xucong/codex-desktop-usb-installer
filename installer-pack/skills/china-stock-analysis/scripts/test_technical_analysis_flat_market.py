"""Offline regressions for flat-market trend and RSI classification."""

from __future__ import annotations

import unittest

import technical_analysis


class TechnicalAnalysisFlatMarketTests(unittest.TestCase):
    def test_constant_prices_are_neutral_not_downtrend_or_overbought(self):
        prices = [100.0] * 60

        report = technical_analysis.generate_technical_analysis(
            "000001", prices, prices, prices, [0.0] * 60, "daily"
        )

        self.assertEqual(report["trend"], {"trend": "横盘整理", "strength": 0})
        self.assertEqual(report["indicators"]["rsi"], 50.0)
        self.assertFalse(report["rule_observations"]["rsi_above_70"])
        self.assertFalse(report["rule_observations"]["rsi_below_30"])

    def test_strictly_falling_prices_remain_a_downtrend_with_zero_rsi(self):
        prices = [160.0 - index for index in range(60)]

        report = technical_analysis.generate_technical_analysis(
            "000001", prices, prices, prices, [1000.0] * 60, "daily"
        )

        self.assertEqual(report["trend"], {"trend": "下降趋势", "strength": -3})
        self.assertEqual(report["indicators"]["rsi"], 0.0)

    def test_strictly_rising_prices_remain_an_uptrend_with_hundred_rsi(self):
        prices = [100.0 + index for index in range(60)]

        report = technical_analysis.generate_technical_analysis(
            "000001", prices, prices, prices, [1000.0] * 60, "daily"
        )

        self.assertEqual(report["trend"], {"trend": "上升趋势", "strength": 3})
        self.assertEqual(report["indicators"]["rsi"], 100.0)

    def test_mixed_moving_average_slopes_are_not_a_strong_downtrend(self):
        ma5 = [9.0] * 19 + [10.0]
        ma20 = [12.0] * 10 + [11.0] * 10

        trend = technical_analysis.identify_trend([10.0] * 20, ma5, ma20)

        self.assertEqual(trend, {"trend": "偏空震荡", "strength": -1})

    def test_falling_averages_do_not_override_price_above_long_average(self):
        ma5 = [10.0] * 19 + [9.0]
        ma20 = [11.0] * 11 + [10.0] * 9

        trend = technical_analysis.identify_trend([11.0] * 20, ma5, ma20)

        self.assertEqual(trend, {"trend": "横盘整理", "strength": 0})


if __name__ == "__main__":
    unittest.main(verbosity=2)
