"""Corrupt persisted JSON must fail closed without rewriting the source file."""

import hashlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import reflection_db as db


ROOT = Path(__file__).resolve().parents[4]


class CorruptReflectionDatabaseTests(unittest.TestCase):
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

    def assert_corrupt_file_preserved(self, path, operation):
        original = b'{"preservation_sentinel": '
        path.write_bytes(original)
        original_hash = hashlib.sha256(original).hexdigest()

        with self.assertRaisesRegex(ValueError, "contains invalid JSON"):
            operation()

        self.assertEqual(path.read_bytes(), original)
        self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), original_hash)

    def test_profile_reads_and_saves_fail_closed_without_rewriting_corrupt_json(self):
        self.assert_corrupt_file_preserved(db.PROFILE_FILE, db.get_profile)
        self.assert_corrupt_file_preserved(
            db.PROFILE_FILE, lambda: db.save_profile({"timezone": "UTC"})
        )

    def test_reflection_reads_and_logs_fail_closed_without_rewriting_corrupt_json(self):
        self.assert_corrupt_file_preserved(db.REFLECTIONS_FILE, db.get_reflections)
        self.assert_corrupt_file_preserved(
            db.REFLECTIONS_FILE,
            lambda: db.log_reflection({"date": "2026-10-19", "mood": "calm"}),
        )


if __name__ == "__main__":
    unittest.main()
