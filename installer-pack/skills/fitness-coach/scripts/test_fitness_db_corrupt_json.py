"""Corrupt fitness DB JSON must fail closed without rewriting persisted bytes."""

import hashlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import fitness_db as db


ROOT = Path(__file__).resolve().parents[4]


class CorruptFitnessDatabaseTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        root = Path(self.temp_dir.name)
        self.patches = [
            patch.object(db, "DB_DIR", root),
            patch.object(db, "PROFILE_FILE", root / "profile.json"),
            patch.object(db, "PROGRAMS_FILE", root / "programs.json"),
            patch.object(db, "WORKOUTS_FILE", root / "workouts.json"),
        ]
        for patcher in self.patches:
            patcher.start()
        self.addCleanup(self._cleanup)
        db.ensure_db_files()

    def _cleanup(self):
        for patcher in reversed(self.patches):
            patcher.stop()
        self.temp_dir.cleanup()

    def assert_corrupt_file_preserved(self, path, operation):
        original = b'{"preservation_sentinel": '
        path.write_bytes(original)
        original_hash = hashlib.sha256(original).hexdigest()

        with self.assertRaisesRegex(ValueError, "contains invalid JSON"):
            operation()

        self.assertEqual(path.read_bytes(), original)
        self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), original_hash)

    def test_profile_read_and_save_reject_corrupt_json_without_rewrite(self):
        self.assert_corrupt_file_preserved(db.PROFILE_FILE, db.get_profile)
        self.assert_corrupt_file_preserved(
            db.PROFILE_FILE, lambda: db.save_profile({"experience_level": "beginner"})
        )

    def test_program_read_and_add_reject_corrupt_json_without_rewrite(self):
        self.assert_corrupt_file_preserved(db.PROGRAMS_FILE, db.get_programs)
        self.assert_corrupt_file_preserved(
            db.PROGRAMS_FILE, lambda: db.add_program({"name": "synthetic"})
        )

    def test_workout_read_and_log_reject_corrupt_json_without_rewrite(self):
        self.assert_corrupt_file_preserved(db.WORKOUTS_FILE, db.get_workouts)
        self.assert_corrupt_file_preserved(
            db.WORKOUTS_FILE,
            lambda: db.log_workout({"program_id": "synthetic", "exercises": []}),
        )

    def test_missing_file_keeps_empty_object_fallback(self):
        missing = Path(self.temp_dir.name) / "missing.json"
        self.assertEqual(db.load_json(missing), {})
        self.assertFalse(missing.exists())


if __name__ == "__main__":
    unittest.main()
