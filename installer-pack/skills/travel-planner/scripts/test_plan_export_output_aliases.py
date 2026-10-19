from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import plan_generator


class TravelPlanExportOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        self._temporary = tempfile.TemporaryDirectory(prefix="travel-export-alias-")
        self.root = Path(self._temporary.name)
        self.data_dir = self.root / "isolated-data"
        self.db_dir = self.data_dir / "travel_planner"
        self.db_dir.mkdir(parents=True)
        self.trips_path = self.db_dir / "trips.json"
        self.preferences_path = self.db_dir / "preferences.json"
        self.trips_bytes = json.dumps({
            "current_trips": [self._trip("synthetic-1", "First City"), self._trip("synthetic-2", "Second City")],
            "past_trips": [], "trip_ideas": [],
        }).encode("utf-8")
        self.preferences_bytes = json.dumps({
            "initialized": True, "created_at": "2030-01-01T00:00:00", "travel_style": "",
            "budget_level": "mid-range", "accommodation_preference": [], "interests": [],
            "dietary_restrictions": [], "accessibility_needs": [], "preferred_activities": [],
            "pace_preference": "moderate", "travel_companions": "", "language_skills": [],
            "previous_destinations": [], "bucket_list": [],
        }).encode("utf-8")
        self.trips_path.write_bytes(self.trips_bytes)
        self.preferences_path.write_bytes(self.preferences_bytes)

    def tearDown(self) -> None:
        self._temporary.cleanup()

    def _trip(self, trip_id: str, city: str) -> dict:
        return {
            "id": trip_id,
            "destination": {"city": city, "country": "Synthetic Country"},
            "departure_date": "2030-06-01", "return_date": "2030-06-04",
            "duration_days": 3, "budget": {"total": 3000}, "climate": "moderate", "activities": [],
        }

    def _run(self, output: Path) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["CODEX_DATA_DIR"] = str(self.data_dir)
        env["TEMP"] = str(self.root)
        env["TMP"] = str(self.root)
        return subprocess.run(
            [sys.executable, "-B", str(Path(plan_generator.__file__).resolve()),
             "--trip-id", "synthetic-1", "--output", str(output)],
            capture_output=True, text=True, check=False, env=env,
        )

    def _assert_rejected_without_mutation(self, output: Path, expected_name: str) -> None:
        trips_hash = hashlib.sha256(self.trips_path.read_bytes()).hexdigest()
        preferences_hash = hashlib.sha256(self.preferences_path.read_bytes()).hexdigest()

        result = self._run(output)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(f"output must differ from travel database file: {expected_name}", result.stderr)
        self.assertEqual(hashlib.sha256(self.trips_path.read_bytes()).hexdigest(), trips_hash)
        self.assertEqual(hashlib.sha256(self.preferences_path.read_bytes()).hexdigest(), preferences_hash)

    def test_cli_rejects_trips_database_as_output(self) -> None:
        self._assert_rejected_without_mutation(self.trips_path, "trips.json")

    def test_cli_rejects_preferences_database_as_output(self) -> None:
        alias = self.db_dir / "nested" / ".." / "preferences.json"
        self._assert_rejected_without_mutation(alias, "preferences.json")

    def test_cli_writes_distinct_plan_and_preserves_both_databases(self) -> None:
        output = self.root / "plan.json"
        result = self._run(output)

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("Travel plan generated", result.stdout)
        self.assertEqual(json.loads(output.read_text(encoding="utf-8"))["trip_id"], "synthetic-1")
        self.assertEqual(self.trips_path.read_bytes(), self.trips_bytes)
        self.assertEqual(self.preferences_path.read_bytes(), self.preferences_bytes)

    def test_guard_rejects_filesystem_samefile_alias(self) -> None:
        alias = self.root / "alias.json"
        alias.write_text("samefile simulation", encoding="utf-8")

        with patch.object(Path, "samefile", return_value=True):
            with self.assertRaisesRegex(ValueError, "output must differ from travel database"):
                plan_generator._reject_database_output_alias(alias, [self.trips_path])

        self.assertEqual(self.trips_path.read_bytes(), self.trips_bytes)
        self.assertEqual(alias.read_text(encoding="utf-8"), "samefile simulation")


if __name__ == "__main__":
    unittest.main()
