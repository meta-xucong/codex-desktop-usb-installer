"""Offline regression tests for profile-aware exercise selection."""

import json
import os
import sys
import subprocess
import tempfile
import unittest
from pathlib import Path


SCRIPTS_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPTS_DIR))

from program_generator import (  # noqa: E402
    EXERCISE_AVOID_ALIASES,
    MAX_PROGRAM_DURATION_WEEKS,
    _expand_avoided_exercises,
    generate_workout_day,
    generate_program,
    format_workout_for_display,
    select_exercises,
)


class AvoidExerciseTests(unittest.TestCase):
    def test_overhead_press_profile_slug_excludes_all_library_press_variants(self):
        avoided = _expand_avoided_exercises(["overhead_press"])
        press_variants = EXERCISE_AVOID_ALIASES["overhead_press"]
        self.assertEqual(avoided.intersection(press_variants), press_variants)
        candidates = select_exercises(
            ["shoulders"],
            ["barbell", "dumbbells", "gym"],
            "intermediate",
            avoid_exercises=[],
        )
        self.assertEqual(candidates[0]["name"], "杠铃肩推")

        day = generate_workout_day(
            {"day": 1, "focus": "肩部", "muscle_groups": ["shoulders"]},
            {
                "experience_level": "intermediate",
                "equipment": ["barbell", "dumbbells", "gym"],
                "avoid_exercises": ["overhead_press"],
            },
            goal="hypertrophy",
            week=1,
        )

        names = [exercise["name"] for exercise in day["exercises"]]
        self.assertTrue(names)
        self.assertTrue(press_variants.isdisjoint(names))

    def test_excluding_first_choice_selects_next_equipped_safe_exercise(self):
        candidates = select_exercises(
            ["chest"],
            ["dumbbells", "gym"],
            "intermediate",
            avoid_exercises=[],
        )
        self.assertEqual(candidates[0]["name"], "杠铃卧推")

        day = generate_workout_day(
            {"day": 1, "focus": "胸部", "muscle_groups": ["chest"]},
            {
                "experience_level": "intermediate",
                "equipment": ["dumbbells", "gym"],
                "avoid_exercises": ["杠铃卧推"],
            },
            goal="hypertrophy",
            week=1,
        )

        names = [exercise["name"] for exercise in day["exercises"]]
        self.assertNotIn("杠铃卧推", names)
        self.assertIn("哑铃卧推", names)
        self.assertTrue(names)

    def test_no_safe_available_exercise_returns_empty_exercise_list(self):
        candidates = select_exercises(
            ["chest"], ["none"], "intermediate", avoid_exercises=[]
        )
        self.assertEqual([candidate["name"] for candidate in candidates], ["俯卧撑"])

        day = generate_workout_day(
            {"day": 1, "focus": "胸部", "muscle_groups": ["chest"]},
            {
                "experience_level": "intermediate",
                "equipment": ["none"],
                "avoid_exercises": ["俯卧撑"],
            },
            goal="hypertrophy",
            week=1,
        )

        self.assertEqual(day["exercises"], [])
        self.assertEqual(day["status"], "unavailable")
        self.assertTrue(day["availability_reason"])
        self.assertEqual(day["estimated_duration"], 0)
        display = format_workout_for_display(day)
        self.assertIn("状态：不可用", display)
        self.assertIn(day["availability_reason"], display)
        self.assertIn("预计时间: 0分钟", display)

    def test_available_workout_formats_day_number_and_exercises(self):
        day = generate_workout_day(
            {"day": 3, "focus": "胸部", "muscle_groups": ["chest"]},
            {"experience_level": "intermediate", "equipment": ["barbell"]},
            goal="hypertrophy",
            week=1,
        )

        display = format_workout_for_display(day)
        self.assertEqual(day["status"], "available")
        self.assertEqual(day["day_number"], 3)
        self.assertIn("训练日 3 - 胸部", display)
        self.assertIn("杠铃卧推", display)

    def test_all_unavailable_workouts_make_program_unavailable(self):
        program = generate_program(
            {"experience_level": "intermediate", "equipment": ["none"],
             "avoid_exercises": [
                 "俯卧撑", "双杠臂屈伸", "箭步蹲", "提踵", "平板支撑", "死虫式", "卷腹", "俄罗斯转体"
             ]},
            {"goal": "hypertrophy", "duration_weeks": 1,
             "split_type": "upper_lower", "days_per_week": 2},
        )

        self.assertEqual(program["status"], "unavailable")
        self.assertEqual(len(program["unavailable_workouts"]), 2)
        self.assertTrue(all(
            workout["status"] == "unavailable"
            for workout in program["weeks"][0]["workouts"]
        ))

    def test_mixed_workout_availability_makes_program_partial(self):
        program = generate_program(
            {"experience_level": "intermediate", "equipment": ["none"],
             "avoid_exercises": ["俯卧撑", "双杠臂屈伸"]},
            {"goal": "hypertrophy", "duration_weeks": 1,
             "split_type": "upper_lower", "days_per_week": 2},
        )

        workouts = program["weeks"][0]["workouts"]
        self.assertEqual(program["status"], "partial")
        self.assertEqual([workout["status"] for workout in workouts],
                         ["unavailable", "available"])
        self.assertEqual(len(program["unavailable_workouts"]), 1)

    def test_full_program_surfaces_unavailable_workouts(self):
        program = generate_program(
            {
                "experience_level": "intermediate",
                "equipment": ["none"],
                "avoid_exercises": ["俯卧撑", "双杠臂屈伸"],
            },
            {
                "goal": "hypertrophy",
                "duration_weeks": 1,
                "split_type": "upper_lower",
                "days_per_week": 1,
            },
        )

        self.assertEqual(program["status"], "unavailable")
        self.assertEqual(
            program["unavailable_workouts"],
            [{"week": 1, "day": 1, "focus": "上肢推"}],
        )
        self.assertEqual(program["weeks"][0]["workouts"][0]["status"], "unavailable")

    def test_all_available_workouts_make_program_available(self):
        program = generate_program(
            {"experience_level": "intermediate", "equipment": ["gym"]},
            {"goal": "hypertrophy", "duration_weeks": 1,
             "split_type": "upper_lower", "days_per_week": 2},
        )

        self.assertEqual(program["status"], "available")
        self.assertEqual(program["unavailable_workouts"], [])

    def test_program_rejects_invalid_duration_and_day_counts(self):
        profile = {"experience_level": "intermediate", "equipment": ["gym"]}
        base = {"split_type": "full_body", "duration_weeks": 1, "days_per_week": 2}

        for invalid_duration in (0, -1, True, 1.0, "1"):
            with self.subTest(duration_weeks=invalid_duration):
                with self.assertRaisesRegex(ValueError, "duration_weeks must be an integer from 1 to 52"):
                    generate_program(profile, {
                        **base,
                        "duration_weeks": invalid_duration,
                    })

        for invalid_days in (0, -1, True, 1.0, "1"):
            with self.subTest(days_per_week=invalid_days):
                with self.assertRaisesRegex(ValueError, "days_per_week must be a positive integer"):
                    generate_program(profile, {
                        **base,
                        "days_per_week": invalid_days,
                    })

        with self.assertRaisesRegex(ValueError, "days_per_week cannot exceed 3"):
            generate_program(profile, {**base, "days_per_week": 4})

    def test_program_duration_is_limited_to_one_year(self):
        profile = {"experience_level": "intermediate", "equipment": ["gym"]}
        for invalid_duration in (MAX_PROGRAM_DURATION_WEEKS + 1, 1_000_000_000):
            with self.subTest(duration_weeks=invalid_duration):
                with self.assertRaisesRegex(ValueError, "duration_weeks must be an integer from 1 to 52"):
                    generate_program(profile, {
                        "split_type": "full_body",
                        "duration_weeks": invalid_duration,
                        "days_per_week": 1,
                    })

        max_duration_program = generate_program(
            profile,
            {
                "split_type": "full_body",
                "duration_weeks": MAX_PROGRAM_DURATION_WEEKS,
                "days_per_week": 1,
            },
        )
        self.assertEqual(max_duration_program["duration_weeks"], 52)
        self.assertEqual(len(max_duration_program["weeks"]), 52)

    def test_unknown_avoid_slug_fails_closed(self):
        with self.assertRaisesRegex(ValueError, "Unknown exercise in avoid_exercises"):
            generate_workout_day(
                {"day": 1, "focus": "胸部", "muscle_groups": ["chest"]},
                {
                    "experience_level": "intermediate",
                    "equipment": ["barbell", "dumbbells", "gym"],
                    "avoid_exercises": ["unknown_press_variant"],
                },
                goal="hypertrophy",
                week=1,
            )

    def test_cli_does_not_persist_fully_unavailable_program(self):
        with tempfile.TemporaryDirectory(
            prefix="fitness-coach-cli-test-", dir=str(SCRIPTS_DIR)
        ) as temp_root:
            data_dir = Path(temp_root) / "fitness_coach"
            data_dir.mkdir()
            profile = {
                "initialized": True,
                "experience_level": "intermediate",
                "equipment": ["none"],
                "avoid_exercises": ["俯卧撑", "双杠臂屈伸"],
            }
            (data_dir / "profile.json").write_text(
                json.dumps(profile), encoding="utf-8"
            )
            programs_file = data_dir / "programs.json"
            programs_file.write_text(
                json.dumps({
                    "active_programs": [],
                    "completed_programs": [],
                    "program_templates": [],
                }),
                encoding="utf-8",
            )
            (data_dir / "workouts.json").write_text(
                json.dumps({"workouts": []}), encoding="utf-8"
            )
            original_programs = programs_file.read_bytes()

            env = os.environ.copy()
            env["CODEX_DATA_DIR"] = temp_root
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS_DIR / "program_generator.py"),
                    "--goal", "hypertrophy",
                    "--weeks", "1",
                    "--split", "upper_lower",
                    "--days", "1",
                ],
                cwd=str(SCRIPTS_DIR),
                env=env,
                capture_output=True,
                text=True,
                timeout=30,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("no available safe exercises", result.stderr)
            self.assertIn("nothing was saved", result.stderr)
            self.assertEqual(programs_file.read_bytes(), original_programs)
            self.assertEqual(
                json.loads(programs_file.read_text(encoding="utf-8"))["active_programs"],
                [],
            )

    def test_cli_rejects_invalid_days_without_persisting(self):
        with tempfile.TemporaryDirectory(
            prefix="fitness-coach-cli-test-", dir=str(SCRIPTS_DIR)
        ) as temp_root:
            data_dir = Path(temp_root) / "fitness_coach"
            data_dir.mkdir()
            (data_dir / "profile.json").write_text(
                json.dumps({
                    "initialized": True,
                    "experience_level": "intermediate",
                    "equipment": ["gym"],
                    "avoid_exercises": [],
                }),
                encoding="utf-8",
            )
            programs_file = data_dir / "programs.json"
            programs_file.write_text(
                json.dumps({
                    "active_programs": [],
                    "completed_programs": [],
                    "program_templates": [],
                }),
                encoding="utf-8",
            )
            (data_dir / "workouts.json").write_text(
                json.dumps({"workouts": []}), encoding="utf-8"
            )
            original_programs = programs_file.read_bytes()

            env = os.environ.copy()
            env["CODEX_DATA_DIR"] = temp_root
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS_DIR / "program_generator.py"),
                    "--goal", "hypertrophy",
                    "--weeks", "1",
                    "--split", "upper_lower",
                    "--days", "0",
                ],
                cwd=str(SCRIPTS_DIR),
                env=env,
                capture_output=True,
                text=True,
                timeout=30,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("days_per_week must be a positive integer", result.stderr)
            self.assertEqual(programs_file.read_bytes(), original_programs)


if __name__ == "__main__":
    unittest.main()
