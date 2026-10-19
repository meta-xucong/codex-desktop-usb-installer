"""Cross-Skill atomic JSON persistence regressions."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
SKILLS = ROOT / "installer-pack" / "skills"
DATABASES = (
    ("meal-planner", "meal_db.py", "PLANS_FILE", ("PROFILE_FILE", "LOGS_FILE", "RECIPES_FILE")),
    ("fitness-coach", "fitness_db.py", "PROGRAMS_FILE", ("PROFILE_FILE", "WORKOUTS_FILE")),
    ("daily-reflection", "reflection_db.py", "GOALS_FILE",
     ("PROFILE_FILE", "REFLECTIONS_FILE", "HABITS_FILE", "READING_FILE", "WELLNESS_FILE")),
)


def load_module(skill: str, filename: str):
    source = SKILLS / skill / "scripts" / filename
    spec = importlib.util.spec_from_file_location(f"atomic_save_{skill.replace('-', '_')}", source)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Cannot load {source}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SkillDatabaseAtomicSaveTests(unittest.TestCase):
    def _setup_database(self, skill, filename, target_name, other_names):
        temporary = tempfile.TemporaryDirectory(prefix=f"{skill}-atomic-save-", dir=ROOT / "tmp")
        root = Path(temporary.name)
        module = load_module(skill, filename)
        stack = ExitStack()
        stack.enter_context(patch.object(module, "DB_DIR", root))
        target_path = root / f"{target_name.removesuffix('_FILE').lower()}.json"
        stack.enter_context(patch.object(module, target_name, target_path))
        for name in other_names:
            stack.enter_context(patch.object(module, name, root / f"{name.removesuffix('_FILE').lower()}.json"))
        module.ensure_db_files()
        original = {"records": [{"id": "preserve", "name": "Original"}]}
        target_path.write_text(json.dumps(original), encoding="utf-8")
        return temporary, stack, module, target_path, target_path.read_bytes()

    def test_serialization_failure_preserves_old_json_and_cleans_temp(self):
        for skill, filename, target_name, other_names in DATABASES:
            with self.subTest(skill=skill):
                temporary, stack, module, target, original = self._setup_database(
                    skill, filename, target_name, other_names
                )
                try:
                    original_hash = hashlib.sha256(original).hexdigest()
                    with self.assertRaises(TypeError):
                        module.save_json(target, {"unserializable": object()})
                    self.assertEqual(target.read_bytes(), original)
                    self.assertEqual(hashlib.sha256(target.read_bytes()).hexdigest(), original_hash)
                    self.assertEqual(json.loads(target.read_text(encoding="utf-8"))["records"][0]["id"], "preserve")
                    self.assertEqual(list(target.parent.glob(f".{target.name}.*.tmp")), [])
                finally:
                    stack.close()
                    temporary.cleanup()

    def test_replace_failure_preserves_old_json_and_cleans_temp(self):
        for skill, filename, target_name, other_names in DATABASES:
            with self.subTest(skill=skill):
                temporary, stack, module, target, original = self._setup_database(
                    skill, filename, target_name, other_names
                )
                try:
                    replacement = {"records": [{"id": "replacement"}]}
                    with patch.object(module.os, "replace", side_effect=OSError("synthetic replace failure")):
                        with self.assertRaisesRegex(OSError, "synthetic replace failure"):
                            module.save_json(target, replacement)
                    self.assertEqual(target.read_bytes(), original)
                    self.assertEqual(json.loads(target.read_text(encoding="utf-8"))["records"][0]["id"], "preserve")
                    self.assertEqual(list(target.parent.glob(f".{target.name}.*.tmp")), [])
                finally:
                    stack.close()
                    temporary.cleanup()

    def test_valid_save_replaces_database_with_complete_json(self):
        for skill, filename, target_name, other_names in DATABASES:
            with self.subTest(skill=skill):
                temporary, stack, module, target, _original = self._setup_database(
                    skill, filename, target_name, other_names
                )
                try:
                    payload = {"records": [{"id": "replacement", "name": "Complete"}]}
                    module.save_json(target, payload)
                    self.assertEqual(json.loads(target.read_text(encoding="utf-8")), payload)
                    self.assertEqual(list(target.parent.glob(f".{target.name}.*.tmp")), [])
                finally:
                    stack.close()
                    temporary.cleanup()


if __name__ == "__main__":
    unittest.main(verbosity=2)
