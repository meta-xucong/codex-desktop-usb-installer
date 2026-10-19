import importlib.util
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).with_name("program_generator.py")
SPEC = importlib.util.spec_from_file_location("fitness_program_generator_under_test", SCRIPT_PATH)
program_generator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(program_generator)


class ProgramGeneratorMuscleCoverageTests(unittest.TestCase):
    def setUp(self):
        self.profile = {
            "experience_level": "intermediate",
            "equipment": ["barbell", "dumbbells", "gym", "none", "bands"],
            "goals": ["hypertrophy"],
        }

    @staticmethod
    def program(split, days):
        return program_generator.generate_program(
            {
                "experience_level": "intermediate",
                "equipment": ["barbell", "dumbbells", "gym", "none", "bands"],
                "goals": ["hypertrophy"],
            },
            {
                "name": "Synthetic coverage test",
                "goal": "hypertrophy",
                "duration_weeks": 1,
                "split_type": split,
                "days_per_week": days,
            },
            program_id="synthetic-test-program",
        )

    @staticmethod
    def library_equipment_by_name():
        result = {}

        def collect(value):
            if isinstance(value, dict):
                if isinstance(value.get("name"), str):
                    result[value["name"]] = set(value.get("equipment", []))
                else:
                    for child in value.values():
                        collect(child)
            elif isinstance(value, list):
                for child in value:
                    collect(child)

        collect(program_generator.EXERCISE_LIBRARY)
        return result

    def assert_equipment_supported(self, workout):
        available = set(self.profile["equipment"])
        library = self.library_equipment_by_name()
        for exercise in workout["exercises"]:
            self.assertIn(exercise["name"], library)
            self.assertTrue(library[exercise["name"]] & available, exercise["name"])

    def test_bro_split_arm_day_includes_real_biceps_and_triceps_exercises(self):
        program = self.program("bro_split", 5)
        workout = program["weeks"][0]["workouts"][4]
        names = {exercise["name"] for exercise in workout["exercises"]}
        biceps_names = {
            exercise["name"]
            for exercise in program_generator.EXERCISE_LIBRARY["arms"]["biceps"]
        }
        triceps_names = {
            exercise["name"]
            for exercise in program_generator.EXERCISE_LIBRARY["arms"]["triceps"]
        }

        self.assertEqual(workout["status"], "available")
        self.assertTrue(names & biceps_names)
        self.assertTrue(names & triceps_names)
        category_by_name = {
            exercise["name"]: exercise["category"] for exercise in workout["exercises"]
        }
        self.assertEqual(category_by_name["杠铃弯举"], "isolation")
        self.assertEqual(category_by_name["绳索下压"], "isolation")
        self.assertEqual(
            program_generator._get_exercise_category("双杠臂屈伸"),
            "compound",
        )
        self.assertEqual(
            program_generator._get_exercise_category("窄距卧推"),
            "compound",
        )
        self.assert_equipment_supported(workout)

    def test_upper_lower_push_has_triceps_and_pull_has_biceps(self):
        program = self.program("upper_lower", 4)
        workouts = {workout["day"]: workout for workout in program["weeks"][0]["workouts"]}
        triceps_names = {
            exercise["name"]
            for exercise in program_generator.EXERCISE_LIBRARY["arms"]["triceps"]
        }
        biceps_names = {
            exercise["name"]
            for exercise in program_generator.EXERCISE_LIBRARY["arms"]["biceps"]
        }

        push = workouts[1]
        pull = workouts[3]
        self.assertTrue({exercise["name"] for exercise in push["exercises"]} & triceps_names)
        self.assertTrue({exercise["name"] for exercise in pull["exercises"]} & biceps_names)
        push_by_name = {exercise["name"]: exercise["category"] for exercise in push["exercises"]}
        pull_by_name = {exercise["name"]: exercise["category"] for exercise in pull["exercises"]}
        self.assertEqual(push_by_name["绳索下压"], "isolation")
        self.assertEqual(pull_by_name["杠铃弯举"], "isolation")
        self.assertEqual(push_by_name["杠铃卧推"], "compound")
        self.assertEqual(pull_by_name["引体向上"], "compound")
        self.assertEqual(
            program_generator._get_exercise_category("哑铃飞鸟"),
            "isolation",
        )
        self.assertEqual(
            program_generator._get_exercise_category("直臂下压"),
            "isolation",
        )
        self.assert_equipment_supported(push)
        self.assert_equipment_supported(pull)

    def test_avoiding_every_arm_exercise_marks_arm_day_unavailable_only(self):
        avoided = [
            exercise["name"]
            for group in ("biceps", "triceps")
            for exercise in program_generator.EXERCISE_LIBRARY["arms"][group]
        ]
        profile = {**self.profile, "avoid_exercises": avoided}
        program = program_generator.generate_program(
            profile,
            {
                "goal": "hypertrophy",
                "duration_weeks": 1,
                "split_type": "bro_split",
                "days_per_week": 5,
            },
            program_id="synthetic-avoidance-test",
        )
        workouts = program["weeks"][0]["workouts"]
        arm_day = workouts[4]

        self.assertEqual(arm_day["status"], "unavailable")
        self.assertTrue(arm_day["availability_reason"])
        self.assertEqual(arm_day["exercises"], [])
        self.assertEqual(program["status"], "partial")
        self.assertEqual(len(program["unavailable_workouts"]), 1)
        self.assertTrue(all(workout["status"] == "available" for workout in workouts[:4]))


if __name__ == "__main__":
    unittest.main()
