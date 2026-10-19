"""Trip status contract tests for travel_db.add_trip()."""

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import travel_db as db


ROOT = Path(__file__).resolve().parents[4]


class TravelTripStatusTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        root = Path(self.temp_dir.name)
        self.patches = [
            patch.object(db, "DB_DIR", root),
            patch.object(db, "PREFERENCES_FILE", root / "preferences.json"),
            patch.object(db, "TRIPS_FILE", root / "trips.json"),
        ]
        for patcher in self.patches:
            patcher.start()
        self.addCleanup(self._cleanup)
        db.ensure_db_files()

    def _cleanup(self):
        for patcher in reversed(self.patches):
            patcher.stop()
        self.temp_dir.cleanup()

    def test_ideas_alias_is_saved_and_readable_by_both_read_api_spellings(self):
        trip_id = db.add_trip({"name": "Synthetic idea"}, status="ideas")

        self.assertEqual(db.get_trip_by_id(trip_id)["name"], "Synthetic idea")
        self.assertEqual(db.get_trips("ideas")["trip_ideas"][0]["id"], trip_id)

    def test_supported_statuses_each_persist_to_expected_list(self):
        for status, key in (("current", "current_trips"), ("past", "past_trips"), ("idea", "trip_ideas")):
            with self.subTest(status=status):
                trip_id = db.add_trip({"name": f"Synthetic {status}"}, status=status)
                self.assertEqual(db.get_trips("all")[key][-1]["id"], trip_id)
                self.assertEqual(db.get_trip_by_id(trip_id)["name"], f"Synthetic {status}")

    def test_unknown_status_fails_before_mutating_input_or_database(self):
        before = db.TRIPS_FILE.read_bytes()
        trip = {"name": "Synthetic invalid status"}

        with self.assertRaisesRegex(ValueError, "status must be"):
            db.add_trip(trip, status="bogus")

        self.assertEqual(db.TRIPS_FILE.read_bytes(), before)
        self.assertEqual(trip, {"name": "Synthetic invalid status"})


if __name__ == "__main__":
    unittest.main()
