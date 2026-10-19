"""Corrupt meal-planner JSON must fail closed without overwriting stored bytes."""

import hashlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import meal_db as db


ROOT = Path(__file__).resolve().parents[4]


class CorruptMealDatabaseTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        root = Path(self.temp_dir.name)
        self.patches = [
            patch.object(db, "DB_DIR", root),
            patch.object(db, "PROFILE_FILE", root / "profile.json"),
            patch.object(db, "PLANS_FILE", root / "plans.json"),
            patch.object(db, "LOGS_FILE", root / "logs.json"),
            patch.object(db, "RECIPES_FILE", root / "recipes.json"),
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
            db.PROFILE_FILE, lambda: db.save_profile({"goal": "maintain"})
        )

    def test_plan_read_and_add_reject_corrupt_json_without_rewrite(self):
        self.assert_corrupt_file_preserved(db.PLANS_FILE, db.get_plans)
        self.assert_corrupt_file_preserved(
            db.PLANS_FILE, lambda: db.add_plan({"name": "synthetic"})
        )

    def test_log_read_and_write_reject_corrupt_json_without_rewrite(self):
        self.assert_corrupt_file_preserved(db.LOGS_FILE, db.get_logs)
        self.assert_corrupt_file_preserved(
            db.LOGS_FILE,
            lambda: db.log_meal({"date": "2026-10-19", "foods": [{"name": "oats"}]}),
        )

    def test_recipe_store_read_rejects_corrupt_json_without_rewrite(self):
        self.assert_corrupt_file_preserved(
            db.RECIPES_FILE, lambda: db.load_json(db.RECIPES_FILE)
        )


if __name__ == "__main__":
    unittest.main()
