#!/usr/bin/env python3
"""Generic seafood allergies must cover known fish and shellfish aliases."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import recipe_generator


ROOT = Path(__file__).resolve().parents[4]
RECIPES = [
    {"name": "三文鱼沙拉", "foods": [{"name": "三文鱼", "amount": 120}]},
    {"name": "虾仁炒饭", "foods": [{"name": "虾仁", "amount": 80}, {"name": "米饭", "amount": 150}]},
    {"name": "扇贝蒸蛋", "foods": [{"name": "扇贝", "amount": 60}, {"name": "鸡蛋", "amount": 1}]},
    {"name": "番茄燕麦碗", "foods": [{"name": "番茄", "amount": 100}, {"name": "燕麦", "amount": 50}]},
]
SEAFOOD_RECIPE_NAMES = {"三文鱼沙拉", "虾仁炒饭", "扇贝蒸蛋"}


class RecipeGeneratorSeafoodAllergyTests(unittest.TestCase):
    def test_chinese_and_english_generic_labels_filter_fish_and_shellfish(self) -> None:
        with tempfile.TemporaryDirectory(prefix="meal-seafood-allergy-", dir=ROOT / "tmp") as temporary:
            root = Path(temporary)
            recipes_path = root / "recipes.json"
            recipes_path.write_text(json.dumps(RECIPES, ensure_ascii=False), encoding="utf-8")
            loaded_recipes = json.loads(recipes_path.read_text(encoding="utf-8"))

            for allergen in ("海鲜", "seafood"):
                profile_path = root / "profile.json"
                profile_path.write_text(
                    json.dumps({"allergies": [allergen]}, ensure_ascii=False), encoding="utf-8"
                )
                profile = json.loads(profile_path.read_text(encoding="utf-8"))
                retained = recipe_generator.filter_recipes_by_diet(
                    loaded_recipes, "regular", allergies=profile["allergies"]
                )
                retained_names = {recipe["name"] for recipe in retained}

                with self.subTest(allergen=allergen):
                    self.assertTrue(SEAFOOD_RECIPE_NAMES.isdisjoint(retained_names))
                    self.assertEqual(retained_names, {"番茄燕麦碗"})


if __name__ == "__main__":
    unittest.main()
