"""Habit completion fields must retain strict boolean semantics."""

import copy
import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import reflection_db as db


ROOT = Path(__file__).resolve().parents[4]
INVALID_VALUES = ("false", "0", 0, 1, None)


class ReflectionHabitCompletedBooleanTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        root = Path(self.temp_dir.name)
        self.patches = [
            patch.object(db, "DB_DIR", root),
            patch.object(db, "PROFILE_FILE", root / "profile.json"),
            patch.object(db, "REFLECTIONS_FILE", root / "reflections.json"),
            patch.object(db, "HABITS_FILE", root / "habits.json"),
            patch.object(db, "GOALS_FILE", root / "goals.json"),
            patch.object(db, "READING_FILE", root / "reading.json"),
            patch.object(db, "WELLNESS_FILE", root / "wellness.json"),
        ]
        for patcher in self.patches:
            patcher.start()
        self.addCleanup(self._cleanup)
        db.ensure_db_files()

    def _cleanup(self):
        for patcher in reversed(self.patches):
            patcher.stop()
        self.temp_dir.cleanup()

    def add_habit(self, name):
        return db.add_habit({"name": name})

    def snapshot_stores(self):
        return {
            path: hashlib.sha256(path.read_bytes()).hexdigest()
            for path in (db.HABITS_FILE, db.REFLECTIONS_FILE)
        }

    def test_true_and_false_are_counted_as_boolean_completion_states(self):
        habit_id = self.add_habit("boolean values")

        db.log_habit(habit_id, "2026-10-18", True)
        db.log_habit(habit_id, "2026-10-19", False)

        stats = db.get_habit_stats(
            habit_id, start_date="2026-10-18", end_date="2026-10-19"
        )
        self.assertEqual(stats["total_completions"], 1)
        self.assertEqual(stats["completion_rate"], 50.0)
        stored = db.get_habit_logs(
            habit_id, start_date="2026-10-18", end_date="2026-10-19"
        )
        self.assertEqual([row["completed"] for row in stored], [True, False])

    def test_direct_habit_log_rejects_non_boolean_without_store_changes(self):
        habit_id = self.add_habit("direct invalid values")
        before = self.snapshot_stores()

        for value in INVALID_VALUES:
            with self.subTest(value=value):
                with self.assertRaisesRegex(ValueError, "completed must be a bool"):
                    db.log_habit(habit_id, "2026-10-19", value)
                self.assertEqual(self.snapshot_stores(), before)

    def test_reflection_habit_log_rejects_non_boolean_without_store_or_caller_changes(self):
        habit_id = self.add_habit("reflection invalid values")
        before = self.snapshot_stores()

        for value in INVALID_VALUES:
            caller = {
                "date": "2026-10-19",
                "mood": "good",
                "habits": [{"habit_id": habit_id, "completed": value}],
            }
            original_caller = copy.deepcopy(caller)
            with self.subTest(value=value):
                with self.assertRaisesRegex(ValueError, "completed must be a bool"):
                    db.log_reflection(caller)
                self.assertEqual(self.snapshot_stores(), before)
                self.assertEqual(caller, original_caller)

    def test_missing_reflection_completed_keeps_false_default(self):
        habit_id = self.add_habit("default false")

        db.log_reflection({
            "date": "2026-10-19",
            "habits": [{"habit_id": habit_id}],
        })

        stats = db.get_habit_stats(
            habit_id, start_date="2026-10-19", end_date="2026-10-19"
        )
        self.assertEqual(stats["total_completions"], 0)
        self.assertEqual(stats["completion_rate"], 0.0)
        self.assertIs(db.get_habit_logs(
            habit_id, start_date="2026-10-19", end_date="2026-10-19"
        )[0]["completed"], False)


if __name__ == "__main__":
    unittest.main()
