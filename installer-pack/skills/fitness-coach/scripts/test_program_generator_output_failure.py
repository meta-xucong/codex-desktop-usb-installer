"""Offline regression tests for output failures during program generation."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPTS_DIR = Path(__file__).resolve().parent
GENERATOR = SCRIPTS_DIR / "program_generator.py"


class ProgramGeneratorOutputFailureTests(unittest.TestCase):
    def setUp(self) -> None:
        temp_root = Path.cwd() / "tmp"
        temp_root.mkdir(exist_ok=True)
        self._temporary = tempfile.TemporaryDirectory(
            prefix="fitness-program-output-failure-", dir=temp_root
        )
        self.root = Path(self._temporary.name)
        self.data_dir = self.root / "data"
        self.db_dir = self.data_dir / "fitness_coach"
        self.db_dir.mkdir(parents=True)
        self.profile = self.db_dir / "profile.json"
        self.programs = self.db_dir / "programs.json"
        self.workouts = self.db_dir / "workouts.json"
        self.initial = {
            self.profile: {
                "initialized": True,
                "experience_level": "intermediate",
                "equipment": ["gym"],
                "goals": ["hypertrophy"],
                "limitations": [],
                "avoid_exercises": [],
            },
            self.programs: {
                "active_programs": [],
                "completed_programs": [],
                "program_templates": [],
            },
            self.workouts: {"workouts": []},
        }
        for path, value in self.initial.items():
            path.write_text(
                json.dumps(value, ensure_ascii=False) + "\n", encoding="utf-8"
            )

    def tearDown(self) -> None:
        self._temporary.cleanup()

    def _run(self, output: Path) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["CODEX_DATA_DIR"] = str(self.data_dir)
        env["TEMP"] = str(self.root)
        env["TMP"] = str(self.root)
        return subprocess.run(
            [
                sys.executable, "-B", str(GENERATOR),
                "--goal", "hypertrophy", "--weeks", "1",
                "--split", "upper_lower", "--days", "1",
                "--output", str(output),
            ],
            cwd=str(SCRIPTS_DIR), env=env, capture_output=True,
            text=True, timeout=30, check=False,
        )

    def _snapshots(self) -> dict[Path, tuple[bytes, str]]:
        return {
            path: (content := path.read_bytes(), hashlib.sha256(content).hexdigest())
            for path in self.initial
        }

    def _assert_snapshots_unchanged(
        self, snapshots: dict[Path, tuple[bytes, str]]
    ) -> None:
        for path, (content, digest) in snapshots.items():
            current = path.read_bytes()
            self.assertEqual(current, content, path.name)
            self.assertEqual(hashlib.sha256(current).hexdigest(), digest, path.name)

    def _assert_no_output_temps(self, output: Path) -> None:
        self.assertEqual(
            list(output.parent.glob(f".{output.name}.*.tmp")), [],
            "output staging files must be removed after failure or publication",
        )

    def test_missing_parent_preserves_all_database_bytes_and_staging_state(self) -> None:
        output = self.root / "missing-parent" / "program.json"
        before = self._snapshots()

        result = self._run(output)

        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertEqual(
            result.stderr.strip(),
            "Error: could not stage program output; no program was saved.",
        )
        self._assert_snapshots_unchanged(before)
        self.assertFalse(output.exists())
        self.assertFalse(output.parent.exists())
        self._assert_no_output_temps(output)

    def test_publish_failure_rolls_back_program_database_and_temp_file(self) -> None:
        output = self.root / "output-is-a-directory"
        output.mkdir()
        before = self._snapshots()

        result = self._run(output)

        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertEqual(
            result.stderr.strip(),
            "Error: could not publish program output; the program was not saved.",
        )
        self._assert_snapshots_unchanged(before)
        self.assertTrue(output.is_dir())
        self.assertEqual(list(output.iterdir()), [])
        self._assert_no_output_temps(output)

    def test_successful_output_matches_exactly_one_active_database_program(self) -> None:
        output = self.root / "program.json"

        result = self._run(output)

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        saved_output = json.loads(output.read_text(encoding="utf-8"))
        program_data = json.loads(self.programs.read_text(encoding="utf-8"))
        active_programs = program_data["active_programs"]
        self.assertEqual(len(active_programs), 1)
        self.assertEqual(saved_output, active_programs[0])
        self.assertEqual(saved_output["status"], "active")
        self.assertEqual(saved_output["id"], active_programs[0]["id"])
        self.assertEqual(program_data["completed_programs"], [])
        self._assert_no_output_temps(output)


if __name__ == "__main__":
    unittest.main()
