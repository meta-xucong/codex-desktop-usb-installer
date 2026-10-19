"""Fixed-clock regression for travel trip identifier collisions."""

import tempfile
import unittest
from datetime import datetime as RealDateTime
from pathlib import Path
from unittest.mock import patch

import travel_db as db


ROOT = Path(__file__).resolve().parents[4]
FIXED_NOW = RealDateTime(2026, 10, 19, 12, 0, 0)


class FrozenDateTime:
    @staticmethod
    def now():
        return FIXED_NOW


class TravelTripIdUniquenessTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        root = Path(self.temp_dir.name)
        self.patches = [
            patch.object(db, "DB_DIR", root),
            patch.object(db, "PREFERENCES_FILE", root / "preferences.json"),
            patch.object(db, "TRIPS_FILE", root / "trips.json"),
            patch.object(db, "datetime", FrozenDateTime),
        ]
        for patcher in self.patches:
            patcher.start()
        self.addCleanup(self._cleanup)
        db.ensure_db_files()

    def _cleanup(self):
        for patcher in reversed(self.patches):
            patcher.stop()
        self.temp_dir.cleanup()

    def test_same_clock_expenses_have_distinct_ids(self):
        trip_id = db.add_trip({"name": "Synthetic expense trip"})

        self.assertTrue(db.add_expense(trip_id, {"label": "First", "amount": 10}))
        self.assertTrue(db.add_expense(trip_id, {"label": "Second", "amount": 20}))
        expenses = db.get_trip_expenses(trip_id)

        self.assertEqual(len(expenses), 2)
        self.assertEqual(len({expense["id"] for expense in expenses}), 2)
        self.assertEqual({expense["label"] for expense in expenses}, {"First", "Second"})

    def test_same_clock_itinerary_items_have_distinct_ids(self):
        trip_id = db.add_trip({"name": "Synthetic itinerary trip"})

        self.assertTrue(db.add_itinerary_item(trip_id, {"label": "First", "date": "2026-11-10"}))
        self.assertTrue(db.add_itinerary_item(trip_id, {"label": "Second", "date": "2026-11-10"}))
        items = db.get_itinerary(trip_id)

        self.assertEqual(len(items), 2)
        self.assertEqual(len({item["id"] for item in items}), 2)
        self.assertEqual({item["label"] for item in items}, {"First", "Second"})

    def test_same_second_trips_have_distinct_ids_and_update_targets_second(self):
        first_id = db.add_trip({"name": "First synthetic trip"})
        second_id = db.add_trip({"name": "Second synthetic trip"})

        self.assertNotEqual(first_id, second_id)
        self.assertEqual(db.get_trip_by_id(first_id)["name"], "First synthetic trip")
        self.assertEqual(db.get_trip_by_id(second_id)["name"], "Second synthetic trip")
        self.assertTrue(db.update_trip(second_id, {"name": "Updated second synthetic trip"}))
        self.assertEqual(db.get_trip_by_id(first_id)["name"], "First synthetic trip")
        self.assertEqual(
            db.get_trip_by_id(second_id)["name"], "Updated second synthetic trip"
        )
        trips = db.get_trips("current")["current_trips"]
        self.assertEqual(len(trips), 2)


if __name__ == "__main__":
    unittest.main()
