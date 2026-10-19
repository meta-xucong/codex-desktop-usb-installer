"""Reflection and habit writes validate together and compensate failed commits."""

import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import reflection_db as db


ROOT = Path(__file__).resolve().parents[4]


class ReflectionCrossStoreAtomicityTests(unittest.TestCase):
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

    def reflection(self):
        return {
            "date": "2026-10-19",
            "mood": "good",
            "habits": [{"habit_id": "habit-1", "completed": True}],
        }

    def test_corrupt_habit_store_fails_before_reflection_or_input_changes(self):
        original_reflections = db.REFLECTIONS_FILE.read_bytes()
        corrupt = b'{"habits": '
        db.HABITS_FILE.write_bytes(corrupt)
        caller = self.reflection()

        with self.assertRaisesRegex(ValueError, "contains invalid JSON"):
            db.log_reflection(caller)

        self.assertEqual(db.REFLECTIONS_FILE.read_bytes(), original_reflections)
        self.assertEqual(db.HABITS_FILE.read_bytes(), corrupt)
        self.assertNotIn("reflection_id", caller)
        self.assertNotIn("logged_at", caller)

    def test_corrupt_habits_as_only_existing_store_does_not_create_other_files(self):
        corrupt = b'{"habits": '
        for path in Path(db.DB_DIR).iterdir():
            if path != db.HABITS_FILE:
                path.unlink()
        db.HABITS_FILE.write_bytes(corrupt)
        caller = self.reflection()

        with self.assertRaisesRegex(ValueError, "contains invalid JSON"):
            db.log_reflection(caller)

        self.assertEqual({path.name for path in Path(db.DB_DIR).iterdir()}, {"habits.json"})
        self.assertEqual(db.HABITS_FILE.read_bytes(), corrupt)
        self.assertNotIn("reflection_id", caller)

    def test_staging_fsync_failure_removes_temporary_files_and_keeps_sources(self):
        original_habits = db.HABITS_FILE.read_bytes()
        original_reflections = db.REFLECTIONS_FILE.read_bytes()
        caller = self.reflection()

        with patch.object(db.os, "fsync", side_effect=OSError("injected fsync failure")):
            with self.assertRaisesRegex(OSError, "injected fsync failure"):
                db.log_reflection(caller)

        self.assertEqual(db.HABITS_FILE.read_bytes(), original_habits)
        self.assertEqual(db.REFLECTIONS_FILE.read_bytes(), original_reflections)
        self.assertFalse(list(Path(db.DB_DIR).glob(".*.tmp")))
        self.assertNotIn("reflection_id", caller)

    def test_second_replace_failure_restores_first_file_and_leaves_input_unchanged(self):
        original_habits = db.HABITS_FILE.read_bytes()
        original_reflections = db.REFLECTIONS_FILE.read_bytes()
        caller = self.reflection()
        original_replace = db.os.replace

        def fail_reflection_commit(source, destination):
            if Path(destination) == db.REFLECTIONS_FILE:
                raise OSError("injected second-file replace failure")
            return original_replace(source, destination)

        with patch.object(db.os, "replace", side_effect=fail_reflection_commit):
            with self.assertRaisesRegex(OSError, "injected second-file"):
                db.log_reflection(caller)

        self.assertEqual(db.HABITS_FILE.read_bytes(), original_habits)
        self.assertEqual(db.REFLECTIONS_FILE.read_bytes(), original_reflections)
        self.assertNotIn("reflection_id", caller)
        self.assertEqual(
            hashlib.sha256(db.HABITS_FILE.read_bytes()).hexdigest(),
            hashlib.sha256(original_habits).hexdigest(),
        )

    def test_second_replace_failure_removes_new_first_file_when_no_prior_store_existed(self):
        db.HABITS_FILE.unlink()
        original_reflections = db.REFLECTIONS_FILE.read_bytes()
        caller = self.reflection()
        original_replace = db.os.replace

        def fail_reflection_commit(source, destination):
            if Path(destination) == db.REFLECTIONS_FILE:
                raise OSError("injected second-file replace failure")
            return original_replace(source, destination)

        with patch.object(db.os, "replace", side_effect=fail_reflection_commit):
            with self.assertRaisesRegex(OSError, "injected second-file"):
                db.log_reflection(caller)

        self.assertFalse(db.HABITS_FILE.exists())
        self.assertEqual(db.REFLECTIONS_FILE.read_bytes(), original_reflections)
        self.assertNotIn("reflection_id", caller)

    def test_success_commits_both_records_and_preserves_existing_log_id(self):
        db.HABITS_FILE.write_text(json.dumps({
            "habits": [],
            "habit_logs": [{
                "log_id": "stable-log-id",
                "habit_id": "habit-1",
                "date": "2026-10-19",
                "completed": False,
            }],
        }), encoding="utf-8")
        caller = self.reflection()

        reflection_id = db.log_reflection(caller)

        stored_reflections = json.loads(db.REFLECTIONS_FILE.read_text(encoding="utf-8"))
        stored_habits = json.loads(db.HABITS_FILE.read_text(encoding="utf-8"))
        self.assertEqual(stored_reflections["reflections"][0]["reflection_id"], reflection_id)
        self.assertEqual(stored_habits["habit_logs"][0]["log_id"], "stable-log-id")
        self.assertTrue(stored_habits["habit_logs"][0]["completed"])
        self.assertEqual(caller["reflection_id"], reflection_id)


if __name__ == "__main__":
    unittest.main()
