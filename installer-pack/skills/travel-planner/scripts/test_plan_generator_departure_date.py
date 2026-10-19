"""Offline validation for departure-relative preparation timelines."""

from __future__ import annotations

import unittest
from datetime import datetime as RealDateTime
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch

import plan_generator


FIXED_NOW = RealDateTime(2030, 1, 1, 12, 0, 0)
ROOT = Path(__file__).resolve().parents[4]
SCRIPT = Path(plan_generator.__file__).resolve()


class FrozenDateTime(RealDateTime):
    @classmethod
    def now(cls, tz=None):
        if tz is not None:
            return FIXED_NOW.replace(tzinfo=tz)
        return FIXED_NOW


class DepartureDateValidationTests(unittest.TestCase):
    def test_pre_trip_checklist_rejects_malformed_blank_and_nonstring_dates(self):
        for value in (
            "not-a-date", "2030-13-40", "2030-04-15T00:00:00+00:00",
            "", "   ", None, 20300415,
        ):
            with self.subTest(departure_date=value):
                with self.assertRaisesRegex(ValueError, "departure_date"):
                    plan_generator.generate_pre_trip_checklist("Synthetic Country", value)

    def test_trip_plan_rejects_missing_or_invalid_departure_before_reading_preferences(self):
        base = {
            "id": "synthetic-trip",
            "destination": {"city": "Synthetic City", "country": "Synthetic Country"},
            "return_date": "2030-04-20",
            "duration_days": 5,
            "budget": {"total": 500},
            "climate": "moderate",
            "activities": [],
        }
        for state in ("missing", "blank", "malformed", "null", "number"):
            with self.subTest(state=state):
                trip = dict(base)
                if state == "blank":
                    trip["departure_date"] = "   "
                elif state == "malformed":
                    trip["departure_date"] = "not-a-date"
                elif state == "null":
                    trip["departure_date"] = None
                elif state == "number":
                    trip["departure_date"] = 20300415
                with patch.object(plan_generator, "get_preferences", side_effect=AssertionError("must validate first")):
                    with self.assertRaisesRegex(ValueError, "departure_date"):
                        plan_generator.generate_trip_plan(trip)

    def test_cli_rejects_invalid_persisted_date_without_traceback_or_output(self):
        with tempfile.TemporaryDirectory(prefix="travel-invalid-date-", dir=ROOT / "tmp") as temporary:
            root = Path(temporary)
            db_dir = root / "data" / "travel_planner"
            db_dir.mkdir(parents=True)
            (db_dir / "preferences.json").write_text(json.dumps({"initialized": True}), encoding="utf-8")
            (db_dir / "trips.json").write_text(json.dumps({
                "current_trips": [{
                    "id": "synthetic-trip",
                    "destination": {"city": "Synthetic City", "country": "Synthetic Country"},
                    "departure_date": "not-a-date",
                    "return_date": "2030-04-20",
                    "duration_days": 5,
                    "budget": {"total": 500},
                    "climate": "moderate",
                    "activities": [],
                }],
                "past_trips": [],
                "trip_ideas": [],
            }), encoding="utf-8")
            output = root / "plan.json"
            env = os.environ.copy()
            env.update({
                "CODEX_DATA_DIR": str(root / "data"),
                "TEMP": str(root),
                "TMP": str(root),
                "PYTHONDONTWRITEBYTECODE": "1",
            })

            completed = subprocess.run(
                [sys.executable, "-B", str(SCRIPT), "--trip-id", "synthetic-trip", "--output", str(output)],
                cwd=ROOT,
                env=env,
                capture_output=True,
                text=True,
                check=False,
                timeout=30,
            )

            self.assertEqual(completed.returncode, 1, completed.stdout + completed.stderr)
            self.assertEqual(completed.stdout, "")
            self.assertIn("Error: invalid trip data:", completed.stderr)
            self.assertIn("departure_date", completed.stderr)
            self.assertNotIn("Traceback", completed.stderr)
            self.assertFalse(output.exists())

    def test_valid_iso_departure_generates_date_relative_timeline(self):
        trip = {
            "id": "synthetic-trip",
            "destination": {"city": "Synthetic City", "country": "Synthetic Country"},
            "departure_date": "2030-04-15",
            "return_date": "2030-04-20",
            "duration_days": 5,
            "budget": {"total": 500},
            "climate": "moderate",
            "activities": [],
        }
        with patch.object(plan_generator, "datetime", FrozenDateTime), \
             patch.object(plan_generator, "get_preferences", return_value={
                 "interests": [], "pace_preference": "moderate", "budget_level": "mid-range",
             }):
            result = plan_generator.generate_trip_plan(trip)

        timeline = [entry["timeline"] for entry in result["pre_trip_checklist"]]
        self.assertEqual(timeline[:3], ["3 months before", "2 months before", "1 month before"])
        self.assertIn("2 weeks before", timeline)
        self.assertIn("1 week before", timeline)
        self.assertIn("Day before departure", timeline)


if __name__ == "__main__":
    unittest.main(verbosity=2)
