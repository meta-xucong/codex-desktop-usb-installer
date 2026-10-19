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
import meal_db


SCRIPTS_DIR = Path(__file__).resolve().parent
GENERATOR = SCRIPTS_DIR / "recipe_generator.py"
sys.path.insert(0, str(SCRIPTS_DIR))
from recipe_generator import _reject_output_overwrite, persist_plan_with_output  # noqa: E402


class RecipeGeneratorOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        temp_root = Path.cwd() / "tmp"
        temp_root.mkdir(exist_ok=True)
        self._temporary = tempfile.TemporaryDirectory(
            prefix="meal-plan-output-alias-", dir=temp_root
        )
        self.root = Path(self._temporary.name)
        self.data_dir = self.root / "data"
        self.db_dir = self.data_dir / "meal_planner"
        self.db_dir.mkdir(parents=True)
        self.profile = self.db_dir / "profile.json"
        self.plans = self.db_dir / "plans.json"
        self.logs = self.db_dir / "logs.json"
        self.recipes = self.db_dir / "recipes.json"
        self.initial = {
            self.profile: {
                "initialized": True,
                "age": 30,
                "gender": "female",
                "height_cm": 165,
                "weight_kg": 60,
                "activity_level": "moderate",
                "goal": "maintain",
                "cuisine_preference": "chinese",
                "diet_type": "regular",
                "allergies": ["花生"],
                "disliked_foods": ["香菜"],
                "cooking_condition": "full_kitchen",
                "meals_per_day": 3,
                "meal_prep": False,
            },
            self.plans: {
                "active_plans": [],
                "completed_plans": [],
                "plan_templates": [],
            },
            self.logs: {"food_logs": []},
            self.recipes: {"custom_recipes": [], "favorite_recipes": []},
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
            "--days", "1", "--goal", "maintain",
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
        for database in (self.profile, self.plans, self.logs, self.recipes):
            with self.subTest(database=database.name):
                before = self._snapshots()
                result = self._run(database)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("--output must differ from meal data files", result.stderr)
                for path, (content, digest) in before.items():
                    self.assertEqual(path.read_bytes(), content)
                    self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), digest)

    def test_cli_rejects_normalized_profile_alias(self) -> None:
        before = self._snapshots()
        alias = self.db_dir / "nested" / ".." / "profile.json"
        result = self._run(alias)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("--output must differ from meal data files", result.stderr)
        for path, (content, digest) in before.items():
            self.assertEqual(path.read_bytes(), content)
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), digest)

    def test_cli_writes_independent_output_and_only_updates_plan_database(self) -> None:
        before_profile = self.profile.read_bytes()
        before_logs = self.logs.read_bytes()
        before_recipes = self.recipes.read_bytes()
        output = self.root / "generated-plan.json"
        result = self._run(output)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(output.read_text(encoding="utf-8"))["status"], "active")
        plans = json.loads(self.plans.read_text(encoding="utf-8"))
        self.assertEqual(len(plans["active_plans"]), 1)
        self.assertEqual(self.profile.read_bytes(), before_profile)
        self.assertEqual(self.logs.read_bytes(), before_logs)
        self.assertEqual(self.recipes.read_bytes(), before_recipes)

    def test_cli_output_parent_failure_does_not_persist_or_duplicate_plan(self) -> None:
        before = self._snapshots()
        output = self.root / "missing-parent" / "generated-plan.json"

        result = self._run(output)

        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertIn("Error:", result.stderr)
        self.assertFalse(output.exists())
        self.assertEqual(json.loads(self.plans.read_text(encoding="utf-8"))["active_plans"], [])
        for path, (content, digest) in before.items():
            self.assertEqual(path.read_bytes(), content)
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), digest)

    def test_failed_final_output_replace_compensates_plan_database(self) -> None:
        output = self.root / "generated-plan.json"
        before = json.loads(self.plans.read_text(encoding="utf-8"))
        with patch.object(meal_db, "PLANS_FILE", self.plans):
            with patch.object(Path, "replace", side_effect=PermissionError("synthetic denial")):
                with self.assertRaisesRegex(OSError, "已回滚活动计划"):
                    persist_plan_with_output({"name": "synthetic plan", "duration_days": 1}, output)
        after = json.loads(self.plans.read_text(encoding="utf-8"))
        self.assertEqual(after, before)
        self.assertFalse(output.exists())
        self.assertEqual(list(self.root.glob(".generated-plan.json.*.tmp")), [])

    def test_failed_compensation_reports_persisted_plan_for_manual_recovery(self) -> None:
        output = self.root / "generated-plan.json"
        with patch.object(meal_db, "PLANS_FILE", self.plans):
            with patch.object(Path, "replace", side_effect=PermissionError("synthetic denial")):
                with patch("recipe_generator.remove_plan", side_effect=OSError("synthetic rollback denial")):
                    with self.assertRaisesRegex(RuntimeError, "输出报告发布失败，且计划回滚失败") as raised:
                        persist_plan_with_output({"name": "recovery plan", "duration_days": 1}, output)
        self.assertIn("synthetic rollback denial", str(raised.exception))
        plans = json.loads(self.plans.read_text(encoding="utf-8"))
        self.assertEqual(len(plans["active_plans"]), 1)
        self.assertEqual(plans["active_plans"][0]["name"], "recovery plan")
        self.assertFalse(output.exists())
        self.assertEqual(list(self.root.glob(".generated-plan.json.*.tmp")), [])

    def test_guard_rejects_filesystem_samefile_alias(self) -> None:
        output = self.root / "different-path.json"
        output.write_bytes(b"leave this file unchanged")
        output_before = output.read_bytes()
        profile_before = self.profile.read_bytes()
        with patch.object(Path, "samefile", return_value=True):
            with self.assertRaisesRegex(ValueError, "--output must differ from meal data files"):
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


