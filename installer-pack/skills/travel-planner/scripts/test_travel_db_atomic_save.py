"""Atomic JSON save failure tests for the travel planner database."""

import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import travel_db as db


ROOT = Path(__file__).resolve().parents[4]


class TravelDatabaseAtomicSaveTests(unittest.TestCase):
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
        self.original = {"current_trips": [{"id": "preserve-trip", "name": "Original"}],
                         "past_trips": [], "trip_ideas": []}
        db.TRIPS_FILE.write_text(json.dumps(self.original), encoding="utf-8")
        self.original_bytes = db.TRIPS_FILE.read_bytes()
        self.original_hash = hashlib.sha256(self.original_bytes).hexdigest()

    def _cleanup(self):
        for patcher in reversed(self.patches):
            patcher.stop()
        self.temp_dir.cleanup()

    def _temporary_files(self):
        return list(db.DB_DIR.glob(".trips.json.*.tmp"))

    def test_unserializable_trip_does_not_damage_existing_database(self):
        with self.assertRaises(TypeError):
            db.add_trip({"name": "Synthetic bad trip", "unserializable": object()})

        self.assertEqual(db.TRIPS_FILE.read_bytes(), self.original_bytes)
        self.assertEqual(hashlib.sha256(db.TRIPS_FILE.read_bytes()).hexdigest(), self.original_hash)
        self.assertEqual(db.get_trip_by_id("preserve-trip")["name"], "Original")
        self.assertEqual(self._temporary_files(), [])

    def test_replace_failure_preserves_existing_database_and_cleans_temporary_file(self):
        replacement = {"current_trips": [{"id": "replacement"}], "past_trips": [], "trip_ideas": []}

        with patch.object(db.os, "replace", side_effect=OSError("synthetic replace failure")):
            with self.assertRaisesRegex(OSError, "synthetic replace failure"):
                db.save_json(db.TRIPS_FILE, replacement)

        self.assertEqual(db.TRIPS_FILE.read_bytes(), self.original_bytes)
        self.assertEqual(db.get_trip_by_id("preserve-trip")["name"], "Original")
        self.assertEqual(self._temporary_files(), [])


if __name__ == "__main__":
    unittest.main()
