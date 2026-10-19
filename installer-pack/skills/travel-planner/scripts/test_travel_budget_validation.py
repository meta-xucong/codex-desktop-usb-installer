from __future__ import annotations

import math
import contextlib
import io
import runpy
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import plan_generator
import travel_db


class TravelBudgetValidationTests(unittest.TestCase):
    def test_zero_and_positive_budgets_are_valid(self):
        zero = plan_generator.calculate_budget_breakdown(0, 2)
        positive = plan_generator.calculate_budget_breakdown(1000, 2)
        self.assertEqual(zero["total_budget"], 0)
        self.assertEqual(zero["daily_average"], 0)
        self.assertEqual(positive["daily_average"], 500)
        self.assertEqual(sum(item["total"] for item in positive["breakdown"].values()), 1000)

    def test_negative_boolean_nonfinite_and_nonnumeric_budgets_are_rejected(self):
        for budget in (-1, True, False, math.inf, -math.inf, math.nan, "1000", None, 10**10000):
            with self.subTest(budget=budget):
                with self.assertRaisesRegex(ValueError, "total_budget"):
                    plan_generator.calculate_budget_breakdown(budget, 2)

    def test_day_count_must_be_a_positive_nonboolean_integer(self):
        for days in (0, -1, 1.5, True, False, "2", None):
            with self.subTest(days=days):
                with self.assertRaisesRegex(ValueError, "num_days"):
                    plan_generator.calculate_budget_breakdown(1000, days)

    def test_negative_persisted_trip_budget_fails_plan_generation(self):
        trip = {
            "id": "synthetic-negative-budget",
            "destination": {"city": "Test City", "country": "Test Country"},
            "departure_date": "2030-06-01",
            "duration_days": 2,
            "budget": {"total": -1000},
            "climate": "moderate",
            "activities": [],
        }
        with patch.object(plan_generator, "get_preferences", return_value={}):
            with self.assertRaisesRegex(ValueError, "total_budget"):
                plan_generator.generate_trip_plan(trip)

    def test_invalid_duration_fails_before_plan_generation(self):
        trip = {
            "destination": {"city": "Test City"},
            "duration_days": 1.5,
            "budget": {"total": 1000},
        }
        with patch.object(plan_generator, "get_preferences", return_value={}):
            with self.assertRaisesRegex(ValueError, "duration_days"):
                plan_generator.generate_trip_plan(trip)

    def test_cli_reports_invalid_budget_without_traceback_or_output_file(self):
        bad_trip = {
            "id": "synthetic-negative-budget",
            "destination": {"city": "Test City", "country": "Test Country"},
            "departure_date": "2030-06-01",
            "duration_days": 2,
            "budget": {"total": -1000},
        }
        with tempfile.TemporaryDirectory(prefix="travel-budget-cli-") as directory:
            output = Path(directory) / "plan.json"
            stdout, stderr = io.StringIO(), io.StringIO()
            with patch.object(sys, "argv", [str(plan_generator.__file__), "--trip-id", bad_trip["id"], "--output", str(output)]), \
                    patch.object(travel_db, "get_trip_by_id", return_value=bad_trip), \
                    patch.object(travel_db, "get_preferences", return_value={}), \
                    contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                with self.assertRaises(SystemExit) as result:
                    runpy.run_path(str(plan_generator.__file__), run_name="__main__")

            self.assertEqual(result.exception.code, 1)
            self.assertIn("Error: invalid trip data: total_budget", stderr.getvalue())
            self.assertNotIn("Traceback", stderr.getvalue())
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
