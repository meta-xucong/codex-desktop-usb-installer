"""Regression tests for fail-closed handling of corrupted travel JSON."""
from __future__ import annotations
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPTS_DIR = Path(__file__).resolve().parent
SOURCE = SCRIPTS_DIR / "travel_db.py"
REPO_ROOT = SCRIPTS_DIR.parents[3]
TEMP_ROOT = REPO_ROOT / "tmp"
TEMP_ROOT.mkdir(exist_ok=True)
TEST_ROOT = Path(tempfile.mkdtemp(prefix="travel-db-corrupt-json-test-", dir=TEMP_ROOT)).resolve()
if TEST_ROOT.parent != TEMP_ROOT.resolve() or not TEST_ROOT.name.startswith("travel-db-corrupt-json-test-"):
    raise RuntimeError(f"Unexpected test directory: {TEST_ROOT}")
os.environ["CODEX_DATA_DIR"] = str(TEST_ROOT / "data")
spec = importlib.util.spec_from_file_location("travel_db_corrupt_json_test", SOURCE)
assert spec and spec.loader
travel_db = importlib.util.module_from_spec(spec)
spec.loader.exec_module(travel_db)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class CorruptTravelJsonTests(unittest.TestCase):
    @classmethod
    def tearDownClass(cls) -> None:
        resolved = TEST_ROOT.resolve()
        if resolved.parent != TEMP_ROOT.resolve() or not resolved.name.startswith("travel-db-corrupt-json-test-"):
            raise RuntimeError(f"Refusing to clean unexpected test path: {resolved}")
        shutil.rmtree(resolved)

    def setUp(self) -> None:
        travel_db.DB_DIR.mkdir(parents=True, exist_ok=True)
        travel_db.PREFERENCES_FILE.write_bytes(b'{"SYNTHETIC_SENTINEL":"preserve-preferences",')
        travel_db.TRIPS_FILE.write_text(json.dumps({"current_trips": [], "past_trips": [], "trip_ideas": []}), encoding="utf-8")

    def test_preference_reads_and_mutations_refuse_corrupt_json_without_overwrite(self) -> None:
        operations = (
            travel_db.get_preferences,
            lambda: travel_db.save_preferences({"travel_style": "synthetic"}),
            lambda: travel_db.update_preference("travel_style", "synthetic"),
            lambda: travel_db.add_to_bucket_list("SYNTHETIC_DESTINATION"),
        )
        for operation in operations:
            before_prefs = digest(travel_db.PREFERENCES_FILE)
            before_trips = digest(travel_db.TRIPS_FILE)
            with self.subTest(operation=getattr(operation, "__name__", "mutator")):
                with self.assertRaisesRegex(ValueError, "preferences.json.*invalid JSON"):
                    operation()
                self.assertEqual(digest(travel_db.PREFERENCES_FILE), before_prefs)
                self.assertEqual(digest(travel_db.TRIPS_FILE), before_trips)

    def test_add_trip_refuses_corrupt_trips_without_overwrite(self) -> None:
        travel_db.PREFERENCES_FILE.write_text(json.dumps({"initialized": True}), encoding="utf-8")
        travel_db.TRIPS_FILE.write_bytes(b'{"current_trips":[{"SYNTHETIC_SENTINEL":"preserve-trip",')
        before_prefs = digest(travel_db.PREFERENCES_FILE)
        before_trips = digest(travel_db.TRIPS_FILE)
        with self.assertRaisesRegex(ValueError, "trips.json.*invalid JSON"):
            travel_db.add_trip({"destination": "synthetic"})
        self.assertEqual(digest(travel_db.PREFERENCES_FILE), before_prefs)
        self.assertEqual(digest(travel_db.TRIPS_FILE), before_trips)

    def test_reset_no_preserves_isolated_database_files(self) -> None:
        travel_db.PREFERENCES_FILE.write_text(json.dumps({"initialized": True, "SYNTHETIC_SENTINEL": "keep"}), encoding="utf-8")
        travel_db.TRIPS_FILE.write_text(json.dumps({"current_trips": [{"SYNTHETIC_SENTINEL": "keep"}], "past_trips": [], "trip_ideas": []}), encoding="utf-8")
        before = {path.name: digest(path) for path in (travel_db.PREFERENCES_FILE, travel_db.TRIPS_FILE)}
        env = os.environ.copy()
        env["PYTHONDONTWRITEBYTECODE"] = "1"
        result = subprocess.run([sys.executable, "-B", str(SOURCE), "reset"], input="no\n", env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Reset cancelled.", result.stdout)
        after = {path.name: digest(path) for path in (travel_db.PREFERENCES_FILE, travel_db.TRIPS_FILE)}
        self.assertEqual(after, before)


if __name__ == "__main__":
    unittest.main()
