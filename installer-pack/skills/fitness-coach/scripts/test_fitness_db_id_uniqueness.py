"""Fixed-clock regression tests for fitness database record identifiers."""

import importlib.util
import os
import tempfile
import unittest
from datetime import datetime as RealDateTime
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
SOURCE = Path(__file__).with_name("fitness_db.py")
FIXED_NOW = RealDateTime(2026, 10, 19, 12, 0, 0)


class FrozenDateTime:
    @staticmethod
    def now():
        return FIXED_NOW


class FitnessDbIdUniquenessTests(unittest.TestCase):
    def setUp(self):
        self.temp_root = tempfile.TemporaryDirectory(
            prefix="fitness-db-id-", dir=ROOT / "tmp"
        )
        self.previous_data_dir = os.environ.get("CODEX_DATA_DIR")
        os.environ["CODEX_DATA_DIR"] = self.temp_root.name
        spec = importlib.util.spec_from_file_location(
            f"fitness_db_test_{id(self)}", SOURCE
        )
        self.db = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.db)
        self.db.datetime = FrozenDateTime

    def tearDown(self):
        if self.previous_data_dir is None:
            os.environ.pop("CODEX_DATA_DIR", None)
        else:
            os.environ["CODEX_DATA_DIR"] = self.previous_data_dir
        self.temp_root.cleanup()

    def test_same_second_program_ids_lookup_and_update_are_independent(self):
        first_id = self.db.add_program({"name": "First", "workout_days": []})
        second_id = self.db.add_program({"name": "Second", "workout_days": []})

        self.assertNotEqual(first_id, second_id)
        self.assertEqual(self.db.get_program_by_id(first_id)["name"], "First")
        self.assertEqual(self.db.get_program_by_id(second_id)["name"], "Second")
        self.assertTrue(self.db.update_program(second_id, {"name": "Updated second"}))
        self.assertEqual(self.db.get_program_by_id(first_id)["name"], "First")
        self.assertEqual(
            self.db.get_program_by_id(second_id)["name"], "Updated second"
        )

    def test_same_second_workout_log_ids_and_lookups_are_independent(self):
        first_id = self.db.log_workout(
            {"program_id": "program-a", "exercises": [{"name": "Squat"}]}
        )
        second_id = self.db.log_workout(
            {"program_id": "program-b", "exercises": [{"name": "Press"}]}
        )

        self.assertNotEqual(first_id, second_id)
        self.assertEqual(
            self.db.get_workout_by_id(first_id)["exercises"][0]["name"], "Squat"
        )
        self.assertEqual(
            self.db.get_workout_by_id(second_id)["exercises"][0]["name"], "Press"
        )
        self.assertEqual(len(self.db.get_workouts()), 2)


if __name__ == "__main__":
    unittest.main()
