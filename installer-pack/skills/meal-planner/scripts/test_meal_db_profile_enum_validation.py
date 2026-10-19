#!/usr/bin/env python3
"""Reject unsupported profile enums without changing valid nutrition math."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import meal_db  # noqa: E402


PROFILE = {
    "age": 30,
    "gender": "female",
    "height_cm": 165,
    "weight_kg": 60,
}


class MealDatabaseProfileEnumValidationTests(unittest.TestCase):
    def test_every_documented_activity_level_keeps_its_existing_targets(self) -> None:
        expected_tdee = {
            "sedentary": 1584,
            "light": 1815,
            "moderate": 2046,
            "high": 2277,
            "very_high": 2508,
        }
        for activity, tdee in expected_tdee.items():
            with self.subTest(activity=activity):
                targets = meal_db.calculate_nutrition_targets({
                    **PROFILE, "activity_level": activity, "goal": "maintain"
                })
                self.assertEqual(targets["tdee"], tdee)
                self.assertEqual(targets["calories"], tdee)

    def test_every_documented_goal_keeps_its_existing_targets(self) -> None:
        expected = {
            "muscle_gain": (2446, 183, 275, 68),
            "fat_loss": (1646, 144, 144, 55),
            "maintain": (2046, 128, 256, 57),
            "health": (2046, 128, 256, 57),
        }
        for goal, targets_expected in expected.items():
            with self.subTest(goal=goal):
                targets = meal_db.calculate_nutrition_targets({
                    **PROFILE, "activity_level": "moderate", "goal": goal
                })
                self.assertEqual(
                    (targets["calories"], targets["protein_g"], targets["carbs_g"], targets["fat_g"]),
                    targets_expected,
                )

    def test_unknown_null_blank_and_nonstring_values_fail_with_field_names(self) -> None:
        invalid_values = ("sedentry", "fatloss", None, "", "   ", {}, [], 7, True)
        for value in invalid_values:
            with self.subTest(field="activity_level", value=value):
                with self.assertRaisesRegex(ValueError, "activity_level"):
                    meal_db.calculate_tdee({**PROFILE, "activity_level": value})
            with self.subTest(field="goal", value=value):
                with self.assertRaisesRegex(ValueError, "goal"):
                    meal_db.calculate_nutrition_targets({
                        **PROFILE, "activity_level": "moderate", "goal": value
                    })

    def test_missing_enum_fields_keep_compatibility_defaults(self) -> None:
        missing_fields = {key: value for key, value in PROFILE.items()}
        defaults = meal_db.calculate_nutrition_targets(missing_fields)
        explicit_defaults = meal_db.calculate_nutrition_targets({
            **PROFILE, "activity_level": "moderate", "goal": "maintain"
        })
        self.assertEqual(defaults, explicit_defaults)
        self.assertEqual(defaults["tdee"], 2046)
        self.assertEqual(defaults["calories"], 2046)

    def test_frozen_silent_fallback_vectors_are_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "activity_level"):
            meal_db.calculate_nutrition_targets({
                **PROFILE, "activity_level": "sedentry", "goal": "maintain"
            })
        with self.assertRaisesRegex(ValueError, "goal"):
            meal_db.calculate_nutrition_targets({
                **PROFILE, "activity_level": "moderate", "goal": "fatloss"
            })


if __name__ == "__main__":
    unittest.main()
