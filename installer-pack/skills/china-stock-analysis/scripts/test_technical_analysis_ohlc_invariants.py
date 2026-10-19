"""Offline checks for OHLC candle ordering and aligned input arrays."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPTS_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS_DIR))
import technical_analysis  # noqa: E402


class TechnicalAnalysisOhlcInvariantTests(unittest.TestCase):
    def _inputs(self):
        closes = [100.0 + index for index in range(60)]
        return closes, [value + 1 for value in closes], [value - 1 for value in closes], [1000.0] * 60

    def test_rejects_close_above_high_with_row_number_before_indicators(self):
        closes, highs, lows, volumes = self._inputs()
        highs[17] = closes[17] - 1
        with patch.object(technical_analysis, "calculate_ma") as calculate_ma:
            with self.assertRaisesRegex(ValueError, "第18条K线.*最低价 <= 收盘价 <= 最高价"):
                technical_analysis.generate_technical_analysis(
                    "SYNTHETIC", closes, highs, lows, volumes, "daily"
                )
        calculate_ma.assert_not_called()

    def test_rejects_close_below_low_with_row_number(self):
        closes, highs, lows, volumes = self._inputs()
        lows[4] = closes[4] + 1
        with self.assertRaisesRegex(ValueError, "第5条K线.*最低价 <= 收盘价 <= 最高价"):
            technical_analysis.generate_technical_analysis(
                "SYNTHETIC", closes, highs, lows, volumes, "daily"
            )

    def test_rejects_high_below_low_with_row_number(self):
        closes, highs, lows, volumes = self._inputs()
        lows[29] = closes[29] + 2
        highs[29] = closes[29] + 1
        with self.assertRaisesRegex(ValueError, "第30条K线.*最低价 <= 收盘价 <= 最高价"):
            technical_analysis.generate_technical_analysis(
                "SYNTHETIC", closes, highs, lows, volumes, "daily"
            )

    def test_rejects_each_array_length_mismatch_before_indicators(self):
        for series_index, field in enumerate(("收盘价", "最高价", "最低价", "成交量")):
            with self.subTest(field=field):
                inputs = [list(values) for values in self._inputs()]
                inputs[series_index].pop()
                with patch.object(technical_analysis, "calculate_ma") as calculate_ma:
                    with self.assertRaisesRegex(ValueError, rf"K线数组长度不一致：.*{field}=59"):
                        technical_analysis.generate_technical_analysis(
                            "SYNTHETIC", *inputs, "daily"
                        )
                calculate_ma.assert_not_called()

    def test_valid_normal_and_equal_boundary_bars_generate_reports(self):
        report = technical_analysis.generate_technical_analysis(
            "SYNTHETIC", *self._inputs(), "daily"
        )
        self.assertEqual(report["current_price"], 159.0)

        equal = [100.0] * 60
        equal_report = technical_analysis.generate_technical_analysis(
            "SYNTHETIC", equal, equal, equal, [0.0] * 60, "daily"
        )
        self.assertEqual(equal_report["current_price"], 100.0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
