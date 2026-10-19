"""Workout set input validation and persistence regressions."""

import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
SOURCE = Path(__file__).with_name("fitness_db.py")


class FitnessDbWorkoutValidationTests(unittest.TestCase):
    def setUp(self):
        self.temp_root = tempfile.TemporaryDirectory(
            prefix="fitness-workout-validation-", dir=ROOT / "tmp"
        )
        self.previous_data_dir = os.environ.get("CODEX_DATA_DIR")
        os.environ["CODEX_DATA_DIR"] = self.temp_root.name
        spec = importlib.util.spec_from_file_location(
            f"fitness_db_validation_{id(self)}", SOURCE
        )
        self.db = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.db)
        self.db.ensure_db_files()

    def tearDown(self):
        if self.previous_data_dir is None:
            os.environ.pop("CODEX_DATA_DIR", None)
        else:
            os.environ["CODEX_DATA_DIR"] = self.previous_data_dir
        self.temp_root.cleanup()

    def workout(self, weight=60, reps=10):
        return {
            "program_id": "synthetic",
            "duration_minutes": 45,
            "exercises": [{"name": "bench press", "sets": [{"weight": weight, "reps": reps}]}],
        }

    def assert_rejected_without_mutation(self, workout):
        before = self.db.WORKOUTS_FILE.read_bytes()
        original = json.dumps(workout, allow_nan=True, sort_keys=True)
        with self.assertRaises(ValueError):
            self.db.log_workout(workout)
        self.assertEqual(self.db.WORKOUTS_FILE.read_bytes(), before)
        self.assertEqual(json.dumps(workout, allow_nan=True, sort_keys=True), original)

    def test_negative_repetitions_are_rejected_before_persistence(self):
        workout = self.workout(reps=-10)
        self.assert_rejected_without_mutation(workout)

    def test_invalid_weight_and_repetition_values_are_rejected(self):
        for weight, reps in [
            (-1, 5), (True, 5), (float("nan"), 5), (float("inf"), 5),
            ("60", 5), (60, True), (60, -1), (60, 2.5), (60, float("inf")),
        ]:
            with self.subTest(weight=weight, reps=reps):
                self.assert_rejected_without_mutation(self.workout(weight, reps))

    def test_finite_nonnegative_set_still_persists_and_calculates_metrics(self):
        workout = self.workout()
        log_id = self.db.log_workout(workout)
        self.db.log_workout(self.workout(weight=55, reps=5.0))
        self.assertNotIn("log_id", workout)
        self.assertEqual(self.db.get_workout_by_id(log_id)["exercises"][0]["sets"][0]["reps"], 10)
        self.assertEqual(self.db.get_progress_summary()["total_volume"], 875)
        self.assertAlmostEqual(self.db.get_personal_records()["bench press"]["estimated_1rm"], 80)

    def test_per_set_derived_overflow_is_rejected_without_database_change(self):
        self.assert_rejected_without_mutation(self.workout(weight=1e308, reps=2))

    def test_history_total_overflow_is_rejected_without_database_change(self):
        self.db.log_workout(self.workout(weight=1e308, reps=1))
        before = self.db.WORKOUTS_FILE.read_bytes()
        second = self.workout(weight=1e308, reps=1)
        original = json.loads(json.dumps(second))
        with self.assertRaises(ValueError):
            self.db.log_workout(second)
        self.assertEqual(self.db.WORKOUTS_FILE.read_bytes(), before)
        self.assertEqual(second, original)

    def test_queries_fail_closed_on_invalid_legacy_rows(self):
        self.db.WORKOUTS_FILE.write_text(json.dumps({"workouts": [self.workout(reps=-1)]}), encoding="utf-8")
        for query in (
            self.db.get_workouts,
            lambda: self.db.get_workout_by_id("legacy-bad"),
            self.db.get_progress_summary,
            self.db.get_personal_records,
            lambda: self.db.get_exercise_history("bench press"),
        ):
            with self.subTest(query=query):
                with self.assertRaises(ValueError):
                    query()

    def test_direct_one_rep_max_rejects_invalid_set_values(self):
        for weight, reps in [(-1, 5), (True, 5), (60, -1), (60, True), (1e308, 10**308)]:
            with self.subTest(weight=weight, reps=reps):
                with self.assertRaises(ValueError):
                    self.db.calculate_1rm(weight, reps)


if __name__ == "__main__":
    unittest.main()
