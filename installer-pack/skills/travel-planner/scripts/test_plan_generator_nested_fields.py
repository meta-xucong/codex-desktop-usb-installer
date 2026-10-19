from __future__ import annotations

import contextlib
import io
import json
import runpy
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import plan_generator
import travel_db


def valid_trip():
    return {
        "id": "synthetic-s045-nested",
        "destination": {"city": "Test City", "country": "Test Country"},
        "departure_date": "2030-06-01",
        "return_date": "2030-06-03",
        "duration_days": 2,
        "budget": {"total": 1200},
        "climate": "moderate",
        "activities": [],
    }


class TripNestedFieldValidationTests(unittest.TestCase):
    def test_non_mapping_trip_data_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "trip_data must be a mapping"):
            plan_generator.generate_trip_plan(None)

    def test_non_mapping_destination_and_budget_are_rejected(self):
        cases = (
            ("destination", None),
            ("destination", ["Test City"]),
            ("budget", None),
        )
        for field, invalid_value in cases:
            with self.subTest(field=field, invalid_value=invalid_value):
                trip = valid_trip()
                trip[field] = invalid_value
                with self.assertRaisesRegex(ValueError, rf"{field} must be a mapping"):
                    plan_generator.generate_trip_plan(trip)

    def test_valid_mapping_fields_preserve_plan_behavior(self):
        trip = valid_trip()
        trip["itinerary"] = [{"day": 1, "morning": {"activity": "Synthetic visit"}}]
        with patch.object(plan_generator, "get_preferences", return_value={
            "interests": [], "pace_preference": "moderate", "budget_level": "mid-range"
        }):
            plan = plan_generator.generate_trip_plan(trip)

        self.assertEqual(plan["destination"], trip["destination"])
        self.assertEqual(plan["itinerary"], trip["itinerary"])
        self.assertEqual(plan["budget"]["total_budget"], 1200)
        self.assertEqual(plan["dates"]["duration_days"], 2)

    def test_cli_reports_malformed_nested_fields_without_traceback_or_output(self):
        for field, invalid_value in (
            ("destination", None),
            ("destination", ["Test City"]),
            ("budget", None),
        ):
            with self.subTest(field=field, invalid_value=invalid_value):
                trip = valid_trip()
                trip[field] = invalid_value
                with tempfile.TemporaryDirectory(prefix="travel-nested-cli-") as directory:
                    output = Path(directory) / "plan.json"
                    stdout, stderr = io.StringIO(), io.StringIO()
                    with patch.object(sys, "argv", [
                        str(plan_generator.__file__), "--trip-id", trip["id"], "--output", str(output)
                    ]), patch.object(travel_db, "get_trip_by_id", return_value=trip), \
                            contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                        with self.assertRaises(SystemExit) as result:
                            runpy.run_path(str(plan_generator.__file__), run_name="__main__")

                    self.assertEqual(result.exception.code, 1)
                    self.assertIn(f"Error: invalid trip data: {field} must be a mapping", stderr.getvalue())
                    self.assertNotIn("Traceback", stderr.getvalue())
                    self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
