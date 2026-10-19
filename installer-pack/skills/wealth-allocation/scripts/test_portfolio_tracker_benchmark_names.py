from __future__ import annotations

import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parent))
import portfolio_tracker


class PortfolioTrackerBenchmarkNameTests(unittest.TestCase):
    def calculate(self, benchmarks):
        return portfolio_tracker.track_portfolio(
            initial_value=100,
            current_value=110,
            history=[{"value": 100}, {"value": 105}, {"value": 110}],
            months=2,
            risk_free_rate=2,
            benchmarks=benchmarks,
            data_as_of="2026-10-19",
            sources=["synthetic test data"],
        )

    def test_duplicate_names_after_strip_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "去除首尾空白后重复.*Index"):
            self.calculate({"Index": 5.0, " Index ": 99.0})

    def test_distinct_names_keep_existing_comparison_values(self):
        report = self.calculate({"Index": 5.0, "Bond": 2.5})

        self.assertEqual(
            report["benchmark_comparison"],
            {
                "Index": {
                    "benchmark_return": 5.0,
                    "portfolio_annualized_return": report["annualized_return_percent"],
                    "difference_percentage_points": round(report["annualized_return_percent"] - 5.0, 4),
                },
                "Bond": {
                    "benchmark_return": 2.5,
                    "portfolio_annualized_return": report["annualized_return_percent"],
                    "difference_percentage_points": round(report["annualized_return_percent"] - 2.5, 4),
                },
            },
        )


if __name__ == "__main__":
    unittest.main()
