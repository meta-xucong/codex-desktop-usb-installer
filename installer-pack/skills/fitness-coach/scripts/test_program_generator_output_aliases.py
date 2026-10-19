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


SCRIPTS_DIR = Path(__file__).resolve().parent
GENERATOR = SCRIPTS_DIR / "program_generator.py"
sys.path.insert(0, str(SCRIPTS_DIR))
from program_generator import _reject_output_overwrite  # noqa: E402


class ProgramGeneratorOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        temp_root = Path.cwd() / "tmp"
        temp_root.mkdir(exist_ok=True)
        self._temporary = tempfile.TemporaryDirectory(
            prefix="fitness-program-output-alias-", dir=temp_root
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
            path.write_text(json.dumps(value, ensure_ascii=False) + "\n", encoding="utf-8")

    def tearDown(self) -> None:
        self._temporary.cleanup()

    def _run(self, output: Path | None = None) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["CODEX_DATA_DIR"] = str(self.data_dir)
        env["TEMP"] = str(self.root)
        env["TMP"] = str(self.root)
        command = [
            sys.executable, "-B", str(GENERATOR),
            "--goal", "hypertrophy", "--weeks", "1",
            "--split", "upper_lower", "--days", "1",
        ]
        if output is not None:
            command.extend(["--output", str(output)])
        return subprocess.run(
            command, cwd=str(SCRIPTS_DIR), env=env, capture_output=True,
            text=True, timeout=30, check=False,
        )

    def _snapshots(self) -> dict[Path, tuple[bytes, str]]:
        return {
            path: (data := path.read_bytes(), hashlib.sha256(data).hexdigest())
            for path in self.initial
        }

    def test_cli_rejects_every_database_as_output_before_persisting(self) -> None:
        for database in (self.profile, self.programs, self.workouts):
            with self.subTest(database=database.name):
                before = self._snapshots()
                result = self._run(database)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("--output must differ from fitness data files", result.stderr)
                for path, (content, digest) in before.items():
                    self.assertEqual(path.read_bytes(), content)
                    self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), digest)

    def test_cli_rejects_normalized_profile_alias(self) -> None:
        before = self._snapshots()
        alias = self.db_dir / "nested" / ".." / "profile.json"
        result = self._run(alias)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("--output must differ from fitness data files", result.stderr)
        for path, (content, digest) in before.items():
            self.assertEqual(path.read_bytes(), content)
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), digest)

    def test_cli_writes_independent_output_and_only_updates_program_database(self) -> None:
        before_profile = self.profile.read_bytes()
        before_workouts = self.workouts.read_bytes()
        output = self.root / "generated-program.json"
        result = self._run(output)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(output.read_text(encoding="utf-8"))["status"], "active")
        programs = json.loads(self.programs.read_text(encoding="utf-8"))
        self.assertEqual(len(programs["active_programs"]), 1)
        self.assertEqual(self.profile.read_bytes(), before_profile)
        self.assertEqual(self.workouts.read_bytes(), before_workouts)

    def test_guard_rejects_filesystem_samefile_alias(self) -> None:
        output = self.root / "different-path.json"
        output.write_bytes(b"leave this file unchanged")
        output_before = output.read_bytes()
        profile_before = self.profile.read_bytes()
        with patch.object(Path, "samefile", return_value=True):
            with self.assertRaisesRegex(ValueError, "--output must differ from fitness data files"):
                _reject_output_overwrite(output, [self.profile])
        self.assertEqual(output.read_bytes(), output_before)
        self.assertEqual(self.profile.read_bytes(), profile_before)

    def test_guard_fails_closed_when_filesystem_identity_cannot_be_checked(self) -> None:
        output = self.root / "identity-unknown.json"
        output.write_bytes(b"leave this file unchanged")
        output_before = output.read_bytes()
        profile_before = self.profile.read_bytes()
        with patch.object(Path, "samefile", side_effect=PermissionError("synthetic identity error")):
            with self.assertRaisesRegex(ValueError, "Cannot verify --output is distinct"):
                _reject_output_overwrite(output, [self.profile])
        self.assertEqual(output.read_bytes(), output_before)
        self.assertEqual(self.profile.read_bytes(), profile_before)


if __name__ == "__main__":
    unittest.main()
