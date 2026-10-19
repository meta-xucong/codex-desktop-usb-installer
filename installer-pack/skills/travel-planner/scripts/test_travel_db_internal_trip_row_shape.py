"""Travel trip rows must be JSON objects before ID lookup."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
SCRIPT = ROOT / "installer-pack" / "skills" / "travel-planner" / "scripts" / "plan_generator.py"
TEMP_ROOT = ROOT / "tmp"


def trip(trip_id: str = "synthetic-trip") -> dict:
    return {
        "id": trip_id,
        "destination": {"city": "Synthetic City", "country": "Synthetic Country"},
        "departure_date": "2030-06-01",
        "return_date": "2030-06-04",
        "duration_days": 3,
        "budget": {"total": 3000},
        "climate": "moderate",
        "activities": [],
    }


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class InternalTripRowShapeTests(unittest.TestCase):
    def _run_cli(self, root: Path, output: Path) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env.update({
            "CODEX_DATA_DIR": str(root / "data"),
            "TEMP": str(root),
            "TMP": str(root),
            "PYTHONDONTWRITEBYTECODE": "1",
        })
        return subprocess.run(
            [sys.executable, "-B", str(SCRIPT), "--trip-id", "synthetic-trip", "--output", str(output)],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
            check=False,
            timeout=30,
        )

    def _write_database(self, root: Path, trip_rows: list[object]) -> tuple[Path, Path]:
        db_dir = root / "data" / "travel_planner"
        db_dir.mkdir(parents=True)
        preferences = db_dir / "preferences.json"
        trips = db_dir / "trips.json"
        preferences.write_text(json.dumps({"initialized": True}) + "\n", encoding="utf-8")
        trips.write_text(json.dumps({
            "current_trips": trip_rows,
            "past_trips": [],
            "trip_ideas": [],
        }) + "\n", encoding="utf-8")
        return preferences, trips

    def test_cli_rejects_null_trip_row_without_traceback_output_or_mutation(self):
        with tempfile.TemporaryDirectory(prefix="travel-row-shape-null-", dir=TEMP_ROOT) as temporary:
            root = Path(temporary)
            preferences, trips = self._write_database(root, [None])
            before = {preferences.name: sha256(preferences), trips.name: sha256(trips)}
            output = root / "plan.json"

            result = self._run_cli(root, output)

            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertIn("trips.json", result.stderr)
            self.assertIn("current_trips", result.stderr)
            self.assertIn("JSON object", result.stderr)
            self.assertNotIn("Traceback", result.stdout + result.stderr)
            self.assertEqual(result.stdout, "")
            self.assertFalse(output.exists())
            self.assertEqual({preferences.name: sha256(preferences), trips.name: sha256(trips)}, before)

    def test_mapping_trip_row_still_generates_a_plan(self):
        with tempfile.TemporaryDirectory(prefix="travel-row-shape-mapping-", dir=TEMP_ROOT) as temporary:
            root = Path(temporary)
            preferences, trips = self._write_database(root, [trip()])
            before = {preferences.name: sha256(preferences), trips.name: sha256(trips)}
            output = root / "plan.json"

            result = self._run_cli(root, output)

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("Travel plan generated", result.stdout)
            self.assertEqual(json.loads(output.read_text(encoding="utf-8"))["trip_id"], "synthetic-trip")
            self.assertEqual({preferences.name: sha256(preferences), trips.name: sha256(trips)}, before)


if __name__ == "__main__":
    unittest.main()
