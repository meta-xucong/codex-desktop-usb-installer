"""Vegan meal plans reject recipes containing animal-derived ingredients."""

from __future__ import annotations

import unittest
import json

import recipe_generator


TARGETS = {"calories": 2000, "protein_g": 100}


class RecipeGeneratorVeganFilterTests(unittest.TestCase):
    def test_vegan_filter_rejects_animal_foods_and_keeps_whole_plant_recipe(self):
        recipes = [
            {"name": "茶叶蛋套餐", "foods": [{"name": "茶叶蛋"}]},
            {"name": "牛奶燕麦", "foods": [{"name": "牛奶"}]},
            {"name": "鸡胸饭", "foods": [{"name": "鸡胸肉"}]},
            {"name": "清蒸鱼", "foods": [{"name": "鲈鱼"}]},
            {"name": "虾仁饭", "foods": [{"name": "虾仁"}]},
            {"name": "蜂蜜水果", "foods": [{"name": "蜂蜜"}, {"name": "苹果"}]},
            {
                "name": "豆腐燕麦奶碗",
                "foods": [{"name": "豆腐"}, {"name": "燕麦奶"}, {"name": "香蕉"}],
            },
        ]

        safe = recipe_generator.filter_recipes_by_diet(recipes, "vegan")

        self.assertEqual([recipe["name"] for recipe in safe], ["豆腐燕麦奶碗"])
        self.assertEqual(safe[0]["foods"], recipes[-1]["foods"])

    def test_frozen_chinese_profile_generates_a_complete_safe_vegan_day(self):
        profile = {
            "age": 30,
            "gender": "female",
            "height_cm": 165,
            "weight_kg": 60,
            "activity_level": "moderate",
            "goal": "maintain",
            "cuisine_preference": "chinese",
            "diet_type": "vegan",
            "meals_per_day": 3,
            "allergies": [],
            "disliked_foods": [],
        }

        plan = recipe_generator.generate_daily_meals(profile, TARGETS, day_number=1)

        self.assertEqual(len(plan["meals"]), 3)
        self.assertEqual([meal["type"] for meal in plan["meals"]], ["breakfast", "lunch", "dinner"])
        forbidden = ("鸡蛋", "茶叶蛋", "水煮蛋", "牛奶", "鸡胸肉", "牛肉", "鲈鱼", "鱼", "虾", "奶酪", "酸奶", "蜂蜜")
        for meal in plan["meals"]:
            for food in meal["foods"]:
                self.assertFalse(any(term in food["name"] for term in forbidden), food["name"])
                self.assertEqual(meal["totals"], recipe_generator.calculate_meal_totals(meal["foods"]))

        source_records = set()
        for recipe_group in recipe_generator.RECIPE_DATABASE.values():
            groups = recipe_group.values() if isinstance(recipe_group, dict) else (recipe_group,)
            for recipes in groups:
                for recipe in recipes:
                    for food in recipe.get("foods", []):
                        source_records.add(json.dumps(food, sort_keys=True, ensure_ascii=False))
        for meal in plan["meals"]:
            for food in meal["foods"]:
                self.assertIn(json.dumps(food, sort_keys=True, ensure_ascii=False), source_records)

    def test_vegan_four_meal_day_includes_safe_snack(self):
        profile = {
            "age": 30,
            "gender": "female",
            "height_cm": 165,
            "weight_kg": 60,
            "activity_level": "moderate",
            "goal": "maintain",
            "cuisine_preference": "chinese",
            "diet_type": "vegan",
            "meals_per_day": 4,
            "allergies": [],
            "disliked_foods": [],
        }

        plan = recipe_generator.generate_daily_meals(profile, TARGETS, day_number=1)

        self.assertEqual([meal["type"] for meal in plan["meals"]], ["breakfast", "snack", "lunch", "dinner"])
        self.assertEqual(plan["meals"][1]["name"], "苹果香蕉水果杯")

    def test_explicit_no_safe_recipe_raises_instead_of_relaxing_vegan_rule(self):
        profile = {"diet_type": "vegan", "allergies": [], "disliked_foods": []}
        with self.assertRaisesRegex(ValueError, "早餐.*vegan.*未放宽限制"):
            recipe_generator._select_safe_recipe(
                [{"name": "茶叶蛋", "foods": [{"name": "茶叶蛋"}]}],
                profile,
                "早餐",
                1,
            )

    def test_regular_control_keeps_existing_recipe_choices(self):
        profile = {
            "cuisine_preference": "chinese",
            "diet_type": "regular",
            "meals_per_day": 3,
            "allergies": [],
            "disliked_foods": [],
        }

        result = recipe_generator.generate_daily_meals(profile, TARGETS, day_number=1)

        self.assertEqual(
            [meal["name"] for meal in result["meals"]],
            ["豆浆配全麦馒头", "番茄炒蛋配杂粮饭", "清蒸鲈鱼配红薯"],
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
