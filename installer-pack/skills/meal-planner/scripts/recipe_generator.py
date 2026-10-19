#!/usr/bin/env python
"""
Recipe Generator

Generates meal plans and recipes based on nutrition targets and preferences.
"""

import sys
import copy
import os
import re
import tempfile
import unicodedata
import uuid
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent))

from meal_db import (
    PROFILE_FILE,
    PLANS_FILE,
    LOGS_FILE,
    RECIPES_FILE,
    get_profile,
    add_plan,
    remove_plan,
    calculate_nutrition_targets,
)
import json
from datetime import datetime, timedelta
from typing import Dict, Any, Iterable, List, Sequence, Set, Tuple


def _reject_output_overwrite(output_path: str | Path, protected_paths: list[Path]) -> None:
    """Protect nutrition profile and databases from generated report output."""
    for protected_path in protected_paths:
        try:
            resolved_output = Path(output_path).expanduser().resolve()
            resolved_protected = Path(protected_path).expanduser().resolve()
            if resolved_output == resolved_protected:
                raise ValueError("--output must differ from meal data files.")
            try:
                resolved_protected.stat()
            except FileNotFoundError:
                continue
            try:
                resolved_output.stat()
            except FileNotFoundError:
                continue
            if resolved_output.samefile(resolved_protected):
                raise ValueError("--output must differ from meal data files.")
        except ValueError:
            raise
        except OSError as exc:
            raise ValueError("Cannot verify --output is distinct from meal data files.") from exc


def persist_plan_with_output(plan: Dict[str, Any], output_path: str | Path) -> str:
    """Stage the report, persist its plan, then atomically publish the report."""
    destination = Path(output_path).expanduser().resolve()
    if destination.is_dir():
        raise ValueError("--output must be a file path.")

    plan_record = copy.deepcopy(plan)
    plan_id = uuid.uuid4().hex
    created_at = datetime.now().isoformat()
    plan_record.update({"id": plan_id, "created_at": created_at, "status": "active"})
    serialized = json.dumps(plan_record, ensure_ascii=False, indent=2) + "\n"

    staged_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", newline="\n", dir=destination.parent,
            prefix=f".{destination.name}.", suffix=".tmp", delete=False,
        ) as handle:
            staged_path = Path(handle.name)
            handle.write(serialized)
            handle.flush()
            os.fsync(handle.fileno())

        add_plan(plan_record, plan_id=plan_id, created_at=created_at)
        try:
            staged_path.replace(destination)
        except OSError as output_error:
            try:
                if not remove_plan(plan_id):
                    raise RuntimeError("计划记录未能找到以回滚。")
            except Exception as rollback_error:
                raise RuntimeError(
                    f"输出报告发布失败，且计划回滚失败：{rollback_error}"
                ) from output_error
            raise OSError("输出报告发布失败；已回滚活动计划。") from output_error
        plan.clear()
        plan.update(plan_record)
        return plan_id
    finally:
        if staged_path is not None:
            try:
                staged_path.unlink(missing_ok=True)
            except OSError:
                pass


# ============================================================================
# RECIPE DATABASE
# ============================================================================

RECIPE_DATABASE = {
    "breakfast": {
        "chinese": [
            {
                "name": "杂粮粥套餐",
                "foods": [
                    {"name": "杂粮粥", "amount": 250, "unit": "g", "calories": 180, "protein": 6, "carbs": 35, "fat": 2},
                    {"name": "水煮蛋", "amount": 100, "unit": "g", "calories": 140, "protein": 12, "carbs": 1, "fat": 10},
                    {"name": "牛奶", "amount": 250, "unit": "ml", "calories": 165, "protein": 8, "carbs": 12, "fat": 9}
                ],
                "prep_time": 15,
                "cook_time": 30,
                "difficulty": "easy"
            },
            {
                "name": "豆浆配全麦馒头",
                "foods": [
                    {"name": "全麦馒头", "amount": 120, "unit": "g", "calories": 280, "protein": 9, "carbs": 55, "fat": 2},
                    {"name": "无糖豆浆", "amount": 300, "unit": "ml", "calories": 90, "protein": 6, "carbs": 6, "fat": 4},
                    {"name": "茶叶蛋", "amount": 100, "unit": "g", "calories": 140, "protein": 12, "carbs": 1, "fat": 10}
                ],
                "prep_time": 5,
                "cook_time": 10,
                "difficulty": "easy"
            },
            {
                "name": "燕麦鸡蛋饼",
                "foods": [
                    {"name": "燕麦片", "amount": 60, "unit": "g", "calories": 225, "protein": 8, "carbs": 38, "fat": 5},
                    {"name": "鸡蛋", "amount": 100, "unit": "g", "calories": 140, "protein": 12, "carbs": 1, "fat": 10},
                    {"name": "香蕉", "amount": 100, "unit": "g", "calories": 89, "protein": 1, "carbs": 23, "fat": 0}
                ],
                "prep_time": 5,
                "cook_time": 10,
                "difficulty": "easy"
            }
        ],
        "western": [
            {
                "name": "燕麦蛋白碗",
                "foods": [
                    {"name": "燕麦片", "amount": 80, "unit": "g", "calories": 300, "protein": 10, "carbs": 51, "fat": 6},
                    {"name": "希腊酸奶", "amount": 150, "unit": "g", "calories": 90, "protein": 15, "carbs": 6, "fat": 0},
                    {"name": "蓝莓", "amount": 100, "unit": "g", "calories": 57, "protein": 0.7, "carbs": 14, "fat": 0.3},
                    {"name": "坚果", "amount": 15, "unit": "g", "calories": 90, "protein": 3, "carbs": 3, "fat": 8}
                ],
                "prep_time": 5,
                "cook_time": 5,
                "difficulty": "easy"
            },
            {
                "name": "全麦吐司鸡蛋",
                "foods": [
                    {"name": "全麦面包", "amount": 80, "unit": "g", "calories": 200, "protein": 8, "carbs": 36, "fat": 3},
                    {"name": "鸡蛋", "amount": 100, "unit": "g", "calories": 140, "protein": 12, "carbs": 1, "fat": 10},
                    {"name": "牛油果", "amount": 50, "unit": "g", "calories": 80, "protein": 1, "carbs": 4, "fat": 7}
                ],
                "prep_time": 5,
                "cook_time": 5,
                "difficulty": "easy"
            }
        ]
    },
    "lunch_dinner": {
        "chinese": [
            {
                "name": "青椒炒鸡胸配糙米饭",
                "foods": [
                    {"name": "糙米饭", "amount": 150, "unit": "g", "calories": 180, "protein": 4, "carbs": 38, "fat": 1},
                    {"name": "鸡胸肉", "amount": 150, "unit": "g", "calories": 165, "protein": 31, "carbs": 0, "fat": 3.6},
                    {"name": "青椒", "amount": 100, "unit": "g", "calories": 22, "protein": 1, "carbs": 5, "fat": 0.2},
                    {"name": "橄榄油", "amount": 10, "unit": "ml", "calories": 90, "protein": 0, "carbs": 0, "fat": 10}
                ],
                "prep_time": 10,
                "cook_time": 15,
                "difficulty": "easy"
            },
            {
                "name": "番茄炒蛋配杂粮饭",
                "foods": [
                    {"name": "杂粮饭", "amount": 150, "unit": "g", "calories": 180, "protein": 4, "carbs": 38, "fat": 1},
                    {"name": "鸡蛋", "amount": 100, "unit": "g", "calories": 140, "protein": 12, "carbs": 1, "fat": 10},
                    {"name": "番茄", "amount": 200, "unit": "g", "calories": 36, "protein": 2, "carbs": 8, "fat": 0.2},
                    {"name": "橄榄油", "amount": 10, "unit": "ml", "calories": 90, "protein": 0, "carbs": 0, "fat": 10}
                ],
                "prep_time": 5,
                "cook_time": 10,
                "difficulty": "easy"
            },
            {
                "name": "清蒸鲈鱼配红薯",
                "foods": [
                    {"name": "红薯", "amount": 200, "unit": "g", "calories": 172, "protein": 3, "carbs": 40, "fat": 0.2},
                    {"name": "鲈鱼", "amount": 150, "unit": "g", "calories": 150, "protein": 25, "carbs": 0, "fat": 5},
                    {"name": "西兰花", "amount": 150, "unit": "g", "calories": 51, "protein": 4, "carbs": 10, "fat": 0.5},
                    {"name": "橄榄油", "amount": 5, "unit": "ml", "calories": 45, "protein": 0, "carbs": 0, "fat": 5}
                ],
                "prep_time": 10,
                "cook_time": 20,
                "difficulty": "medium"
            },
            {
                "name": "蒜蓉西兰花配牛肉",
                "foods": [
                    {"name": "糙米饭", "amount": 120, "unit": "g", "calories": 144, "protein": 3, "carbs": 30, "fat": 0.8},
                    {"name": "瘦牛肉", "amount": 120, "unit": "g", "calories": 180, "protein": 26, "carbs": 0, "fat": 8},
                    {"name": "西兰花", "amount": 200, "unit": "g", "calories": 68, "protein": 6, "carbs": 13, "fat": 0.7},
                    {"name": "橄榄油", "amount": 10, "unit": "ml", "calories": 90, "protein": 0, "carbs": 0, "fat": 10}
                ],
                "prep_time": 10,
                "cook_time": 15,
                "difficulty": "easy"
            }
        ],
        "western": [
            {
                "name": "煎鸡胸配蔬菜",
                "foods": [
                    {"name": "鸡胸肉", "amount": 150, "unit": "g", "calories": 165, "protein": 31, "carbs": 0, "fat": 3.6},
                    {"name": "红薯", "amount": 200, "unit": "g", "calories": 172, "protein": 3, "carbs": 40, "fat": 0.2},
                    {"name": "混合蔬菜", "amount": 200, "unit": "g", "calories": 70, "protein": 3, "carbs": 15, "fat": 0.5},
                    {"name": "橄榄油", "amount": 10, "unit": "ml", "calories": 90, "protein": 0, "carbs": 0, "fat": 10}
                ],
                "prep_time": 10,
                "cook_time": 20,
                "difficulty": "easy"
            },
            {
                "name": "三文鱼沙拉",
                "foods": [
                    {"name": "三文鱼", "amount": 120, "unit": "g", "calories": 250, "protein": 20, "carbs": 0, "fat": 18},
                    {"name": "藜麦", "amount": 100, "unit": "g", "calories": 120, "protein": 4, "carbs": 21, "fat": 2},
                    {"name": "混合生菜", "amount": 150, "unit": "g", "calories": 25, "protein": 2, "carbs": 5, "fat": 0},
                    {"name": "橄榄油", "amount": 10, "unit": "ml", "calories": 90, "protein": 0, "carbs": 0, "fat": 10}
                ],
                "prep_time": 10,
                "cook_time": 15,
                "difficulty": "easy"
            }
        ]
    },
    "snack": [
        {
            "name": "希腊酸奶配坚果",
            "foods": [
                {"name": "希腊酸奶", "amount": 200, "unit": "g", "calories": 120, "protein": 20, "carbs": 8, "fat": 0},
                {"name": "混合坚果", "amount": 15, "unit": "g", "calories": 90, "protein": 3, "carbs": 3, "fat": 8}
            ],
            "prep_time": 2,
            "cook_time": 0,
            "difficulty": "easy"
        },
        {
            "name": "水煮蛋配水果",
            "foods": [
                {"name": "水煮蛋", "amount": 100, "unit": "g", "calories": 140, "protein": 12, "carbs": 1, "fat": 10},
                {"name": "苹果", "amount": 150, "unit": "g", "calories": 78, "protein": 0.4, "carbs": 21, "fat": 0.3}
            ],
            "prep_time": 2,
            "cook_time": 10,
            "difficulty": "easy"
        },
        {
            "name": "蛋白粉奶昔",
            "foods": [
                {"name": "乳清蛋白粉", "amount": 30, "unit": "g", "calories": 120, "protein": 24, "carbs": 3, "fat": 2},
                {"name": "牛奶", "amount": 250, "unit": "ml", "calories": 165, "protein": 8, "carbs": 12, "fat": 9},
                {"name": "香蕉", "amount": 100, "unit": "g", "calories": 89, "protein": 1, "carbs": 23, "fat": 0}
            ],
            "prep_time": 2,
            "cook_time": 0,
            "difficulty": "easy"
        }
    ]
}


def _existing_food_record(name: str) -> Dict[str, Any]:
    """Return a copy of an existing recipe ingredient without changing its values."""
    for recipe_collection in RECIPE_DATABASE.values():
        groups = recipe_collection.values() if isinstance(recipe_collection, dict) else (recipe_collection,)
        for recipes in groups:
            for recipe in recipes:
                for food in recipe.get("foods", []):
                    if food.get("name") == name:
                        return copy.deepcopy(food)
    raise ValueError(f"Vegan recipe ingredient {name!r} has no existing nutrition record.")


# These are combinations of ingredient records already present above. Portions
# and kcal/protein/carbs/fat are copied unchanged from those source records.
_VEGAN_RECIPE_DATABASE = {
    "breakfast": {
        "chinese": [
            {
                "name": "杂粮粥无糖豆浆香蕉",
                "foods": [
                    _existing_food_record("杂粮粥"),
                    _existing_food_record("无糖豆浆"),
                    _existing_food_record("香蕉"),
                ],
            },
        ],
    },
    "lunch_dinner": {
        "chinese": [
            {
                "name": "糙米青椒西兰花",
                "foods": [
                    _existing_food_record("糙米饭"),
                    _existing_food_record("青椒"),
                    _existing_food_record("西兰花"),
                    _existing_food_record("橄榄油"),
                ],
            },
            {
                "name": "红薯藜麦生菜",
                "foods": [
                    _existing_food_record("红薯"),
                    _existing_food_record("藜麦"),
                    _existing_food_record("混合生菜"),
                    _existing_food_record("橄榄油"),
                ],
            },
        ],
    },
    "snack": [
        {
            "name": "苹果香蕉水果杯",
            "foods": [
                _existing_food_record("苹果"),
                _existing_food_record("香蕉"),
            ],
        },
    ],
}


# ============================================================================
# MEAL PLAN GENERATION
# ============================================================================

_FOOD_ALIASES = {
    "tree_nuts": (
        "坚果", "混合坚果", "nut", "tree nut", "tree nuts", "nut butter", "nut oil", "nut flour",
        "almond", "almonds", "杏仁", "扁桃仁", "杏仁酱", "杏仁粉",
        "walnut", "walnuts", "核桃", "cashew", "cashews", "腰果", "pistachio", "pistachios",
        "开心果", "hazelnut", "hazelnuts", "榛子", "pecan", "pecans", "碧根果", "macadamia",
        "夏威夷果", "pine nut", "pine nuts", "松子", "brazil nut", "巴西坚果",
    ),
    "peanut": (
        "花生", "花生酱", "花生油", "花生碎", "花生粉", "peanut", "peanuts", "groundnut",
        "groundnuts", "peanut butter", "peanut oil", "peanut flour", "peanut protein", "arachis",
    ),
    "coriander": (
        "香菜", "芫荽", "胡荽", "cilantro", "coriander", "chinese parsley", "coriander seed",
        "coriander seeds",
    ),
    "shellfish": (
        "贝类", "甲壳类", "虾", "虾皮", "虾酱", "虾粉", "蟹", "蟹肉", "蟹酱", "龙虾", "小龙虾",
        "牡蛎", "生蚝", "蚝油", "蛤蜊", "扇贝", "贻贝", "青口", "shellfish", "shrimp", "prawn",
        "crab", "lobster", "crayfish", "oyster", "clam", "mussel", "scallop", "shrimp paste",
    ),
    "dairy": (
        "乳制品", "牛奶", "奶粉", "炼乳", "奶油", "黄油", "乳清", "酪蛋白", "酸奶", "芝士", "奶酪",
        "dairy", "milk", "milk powder", "condensed milk", "cream", "butter", "whey", "casein",
        "yogurt", "yoghurt", "cheese", "ghee",
    ),
    "egg": ("鸡蛋", "蛋液", "蛋白", "蛋黄", "egg", "eggs", "egg white", "egg yolk", "albumin"),
    "soy": (
        "大豆", "黄豆", "豆浆", "豆腐", "豆豉", "酱油", "味噌", "豆粕", "soy", "soybean", "soya",
        "tofu", "soy milk", "soy sauce", "miso", "tempeh",
    ),
    "fish": (
        "鱼", "鱼肉", "鱼露", "鱼粉", "鱼油", "fish", "anchovy", "cod", "salmon", "tuna", "fish sauce",
    ),
    "wheat_gluten": (
        "小麦", "面粉", "面筋", "麸质", "麦芽", "wheat", "flour", "gluten", "malt", "semolina",
    ),
    "meat": (
        "猪肉", "牛肉", "羊肉", "鸡肉", "鸭肉", "火腿", "培根", "肉类", "pork", "beef", "lamb",
        "chicken", "turkey", "duck", "ham", "bacon", "meat",
    ),
    "honey": ("蜂蜜", "蜂王浆", "honey", "royal jelly"),
}

_BROAD_NUT_ALIASES = ("坚果", "混合坚果", "nut", "nuts", "tree nut", "tree nuts")
_BROAD_SEAFOOD_ALIASES = ("海鲜", "seafood")
_VEGAN_ANIMAL_GROUPS = ("dairy", "egg", "meat", "fish", "shellfish", "honey")
_VEGAN_EXTRA_ANIMAL_ALIASES = (
    "茶叶蛋", "水煮蛋", "咸鸭蛋", "鹌鹑蛋", "皮蛋", "鸭蛋", "鸡蛋", "煎蛋", "炒蛋", "蛋黄酱",
    "鸡胸肉", "鸡胸", "鸭胸肉", "鸭胸", "禽肉", "动物油", "猪油", "牛油",
)
_PLANT_BASED_MILK_ALIASES = (
    "豆浆", "豆奶", "椰奶", "杏仁奶", "燕麦奶", "米浆", "soy milk", "oat milk",
    "almond milk", "coconut milk", "rice milk", "plant milk",
)


def _normalize_text(value: str) -> str:
    value = unicodedata.normalize("NFKC", str(value)).casefold()
    value = re.sub(r"[^\w\u3400-\u9fff]+", " ", value, flags=re.UNICODE)
    return re.sub(r"\s+", " ", value).strip()


def _contains_alias(text: str, alias: str) -> bool:
    normalized_text = _normalize_text(text)
    normalized_alias = _normalize_text(alias)
    if not normalized_text or not normalized_alias:
        return False
    if any("\u3400" <= char <= "\u9fff" for char in normalized_alias):
        return normalized_alias in normalized_text
    return re.search(r"(?<![a-z0-9])" + re.escape(normalized_alias) + r"(?![a-z0-9])", normalized_text) is not None


def _as_terms(values: Any) -> List[str]:
    if values is None:
        return []
    if isinstance(values, str):
        values = [values]
    if not isinstance(values, (list, tuple, set)):
        return [str(values).strip()] if str(values).strip() else []
    return [str(value).strip() for value in values if value is not None and str(value).strip()]


def _restriction_groups(terms: Iterable[str]) -> Tuple[Set[str], List[str]]:
    groups: Set[str] = set()
    literals: List[str] = []
    for term in terms:
        matched = False
        for group, aliases in _FOOD_ALIASES.items():
            if any(_contains_alias(term, alias) for alias in aliases):
                groups.add(group)
                matched = True
        if any(_contains_alias(term, alias) for alias in _BROAD_NUT_ALIASES):
            groups.add("peanut")
            groups.add("tree_nuts")
            matched = True
        if any(_contains_alias(term, alias) for alias in _BROAD_SEAFOOD_ALIASES):
            groups.add("fish")
            groups.add("shellfish")
            matched = True
        if not matched:
            literals.append(_normalize_text(term))
    return groups, literals


def _restriction_terms(profile: Dict[str, Any]) -> Tuple[List[str], List[str]]:
    return _as_terms(profile.get("allergies")), _as_terms(profile.get("disliked_foods"))


def _recipe_texts(recipe: Dict[str, Any]) -> List[str]:
    texts: List[str] = []
    for key in ("name", "foods", "ingredients", "ingredient", "sauce", "toppings", "garnish", "items"):
        value = recipe.get(key)
        if key == "name" and isinstance(value, str):
            texts.append(value)
        elif isinstance(value, list):
            for item in value:
                if isinstance(item, dict):
                    name = item.get("name")
                    if isinstance(name, str):
                        texts.append(name)
                    for nested_key in ("ingredients", "ingredient", "sauce", "toppings", "garnish"):
                        nested = item.get(nested_key)
                        if isinstance(nested, str):
                            texts.append(nested)
                        elif isinstance(nested, list):
                            texts.extend(str(part) for part in nested)
                elif isinstance(item, str):
                    texts.append(item)
        elif isinstance(value, str):
            texts.append(value)
    return texts


def _recipe_has_vegan_forbidden_ingredient(recipe: Dict[str, Any]) -> bool:
    texts = _recipe_texts(recipe)
    for text in texts:
        if any(_contains_alias(text, alias) for alias in _VEGAN_EXTRA_ANIMAL_ALIASES):
            return True
        plant_milk = any(_contains_alias(text, alias) for alias in _PLANT_BASED_MILK_ALIASES)
        for group in _VEGAN_ANIMAL_GROUPS:
            aliases = _FOOD_ALIASES[group]
            if group == "dairy" and plant_milk:
                aliases = tuple(alias for alias in aliases if alias not in {"milk", "牛奶"})
            if any(_contains_alias(text, alias) for alias in aliases):
                return True
    return False


def _matches_restrictions(text: str, terms: Sequence[str]) -> bool:
    groups, literals = _restriction_groups(terms)
    if any(_contains_alias(text, alias) for alias in literals):
        return True
    return any(_contains_alias(text, alias) for group in groups for alias in _FOOD_ALIASES[group])


def _recipe_is_safe(recipe: Dict[str, Any], allergy_terms: Sequence[str], disliked_terms: Sequence[str]) -> bool:
    texts = _recipe_texts(recipe)
    if allergy_terms or disliked_terms:
        has_ingredient_details = False
        for key in ("foods", "ingredients", "ingredient"):
            value = recipe.get(key)
            if isinstance(value, str) and value.strip():
                has_ingredient_details = True
            elif isinstance(value, list) and any(
                (isinstance(item, str) and item.strip())
                or (isinstance(item, dict) and bool(str(item.get("name", "")).strip()))
                for item in value
            ):
                has_ingredient_details = True
        if not has_ingredient_details:
            return False
    return not any(_matches_restrictions(text, allergy_terms) or _matches_restrictions(text, disliked_terms) for text in texts)


def filter_recipes_by_diet(
    recipes: List[Dict],
    diet_type: str,
    allergies: Sequence[str] = (),
    disliked_foods: Sequence[str] = (),
) -> List[Dict]:
    """Filter whole recipes by diet type and declared allergens/dislikes."""
    allergy_terms = _as_terms(allergies)
    disliked_terms = _as_terms(disliked_foods)
    is_vegan = str(diet_type).strip().casefold() == "vegan"
    safe: List[Dict] = []
    for recipe in recipes:
        if is_vegan:
            has_ingredient_details = any(
                (isinstance(recipe.get(key), str) and recipe[key].strip())
                or (
                    isinstance(recipe.get(key), list)
                    and any(
                        (isinstance(item, str) and item.strip())
                        or (isinstance(item, dict) and bool(str(item.get("name", "")).strip()))
                        for item in recipe[key]
                    )
                )
                for key in ("foods", "ingredients", "ingredient")
            )
            if not has_ingredient_details or _recipe_has_vegan_forbidden_ingredient(recipe):
                continue
        if not _recipe_is_safe(recipe, allergy_terms, disliked_terms):
            continue
        safe.append(recipe)
    return safe


def _select_safe_recipe(
    recipes: List[Dict], profile: Dict[str, Any], category: str, day_number: int
) -> Dict[str, Any]:
    allergy_terms, disliked_terms = _restriction_terms(profile)
    safe = filter_recipes_by_diet(
        recipes,
        profile.get("diet_type") or "regular",
        allergy_terms,
        disliked_terms,
    )
    if not safe:
        constraints = ", ".join(allergy_terms + disliked_terms) or (profile.get("diet_type") or "regular")
        raise ValueError(f"{category} 没有满足已声明限制的安全选项（{constraints}）；未放宽限制。")
    return copy.deepcopy(safe[day_number % len(safe)])


def calculate_meal_totals(foods: List[Dict]) -> Dict[str, float]:
    """Calculate total nutrition for a meal."""
    totals = {"calories": 0, "protein": 0, "carbs": 0, "fat": 0}
    
    for food in foods:
        totals["calories"] += food.get("calories", 0)
        totals["protein"] += food.get("protein", 0)
        totals["carbs"] += food.get("carbs", 0)
        totals["fat"] += food.get("fat", 0)
    
    return totals


def generate_daily_meals(profile: Dict, targets: Dict, day_number: int) -> Dict[str, Any]:
    """Generate meals for a single day."""
    cuisine = profile.get("cuisine_preference", "chinese")
    meals_per_day = profile.get("meals_per_day", 3)
    
    # Get available recipes
    breakfast_options = RECIPE_DATABASE["breakfast"].get(cuisine, RECIPE_DATABASE["breakfast"]["chinese"])
    lunch_dinner_options = RECIPE_DATABASE["lunch_dinner"].get(cuisine, RECIPE_DATABASE["lunch_dinner"]["chinese"])
    snack_options = RECIPE_DATABASE["snack"]

    if str(profile.get("diet_type") or "regular").strip().casefold() == "vegan":
        breakfast_options = breakfast_options + _VEGAN_RECIPE_DATABASE["breakfast"].get(cuisine, [])
        lunch_dinner_options = lunch_dinner_options + _VEGAN_RECIPE_DATABASE["lunch_dinner"].get(cuisine, [])
        snack_options = snack_options + _VEGAN_RECIPE_DATABASE["snack"]
    
    # Filter whole recipes before selecting; never remove an ingredient and imply
    # that the remainder is safe from cross-contact or hidden ingredients.
    breakfast = _select_safe_recipe(breakfast_options, profile, "早餐", day_number)
    lunch = _select_safe_recipe(lunch_dinner_options, profile, "午餐", day_number)
    dinner = _select_safe_recipe(lunch_dinner_options, profile, "晚餐", day_number + 1)
    
    meals = [
        {"type": "breakfast", "time": "07:30", **breakfast},
        {"type": "lunch", "time": "12:30", **lunch},
        {"type": "dinner", "time": "19:00", **dinner}
    ]
    
    # Add snacks if 4+ meals
    if meals_per_day >= 4:
        snack1 = _select_safe_recipe(snack_options, profile, "加餐", day_number)
        meals.insert(1, {"type": "snack", "time": "10:30", **snack1})
    
    if meals_per_day >= 5:
        snack2 = _select_safe_recipe(snack_options, profile, "第二加餐", day_number + 1)
        meals.insert(-1, {"type": "snack", "time": "15:30", **snack2})
    
    # Calculate daily totals
    daily_totals = {"calories": 0, "protein": 0, "carbs": 0, "fat": 0}
    for meal in meals:
        meal_totals = calculate_meal_totals(meal.get("foods", []))
        meal["totals"] = meal_totals
        for key in daily_totals:
            daily_totals[key] += meal_totals[key]
    
    return {
        "day": day_number,
        "meals": meals,
        "daily_totals": daily_totals,
        "targets_comparison": {
            "calories_pct": round(daily_totals["calories"] / targets["calories"] * 100, 1),
            "protein_pct": round(daily_totals["protein"] / targets["protein_g"] * 100, 1)
        }
    }


def _validate_duration_days(value: Any) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise ValueError("duration_days 必须是正整数。")
    return value


def generate_weekly_plan(profile: Dict, plan_data: Dict, plan_id: str = None) -> Dict[str, Any]:
    """
    Generate a complete weekly meal plan.
    
    Args:
        profile: User nutrition profile
        plan_data: Plan specifications
        plan_id: Optional existing plan ID
    
    Returns:
        Complete meal plan structure
    """
    duration = _validate_duration_days(plan_data.get("duration_days", 7))
    goal = plan_data.get("goal", "maintain")
    
    # Calculate nutrition targets
    targets = calculate_nutrition_targets(profile)
    
    # Generate days
    days = []
    for day_num in range(1, duration + 1):
        day_plan = generate_daily_meals(profile, targets, day_num)
        days.append(day_plan)
    
    plan = {
        "id": plan_id or str(int(datetime.now().timestamp())),
        "name": plan_data.get("name", f"{duration}天饮食计划"),
        "goal": goal,
        "duration_days": duration,
        "created_at": datetime.now().isoformat(),
        "profile_snapshot": {
            "cuisine": profile.get("cuisine_preference"),
            "diet_type": profile.get("diet_type"),
            "meals_per_day": profile.get("meals_per_day"),
            "allergies": _as_terms(profile.get("allergies")),
            "disliked_foods": _as_terms(profile.get("disliked_foods")),
        },
        "restriction_snapshot": {
            "allergies": _as_terms(profile.get("allergies")),
            "disliked_foods": _as_terms(profile.get("disliked_foods")),
        },
        "allergy_safety_notice": (
            "本计划仅按已知食材名称筛查，不保证无交叉接触。食用前请核对包装配料、"
            "‘可能含有/同线生产’标识及餐馆厨房信息；严重过敏或成分/操作无法确认时请避免食用，"
            "并咨询过敏专科医生。"
        ),
        "nutrition_targets": targets,
        "days": days,
        "cooking_tips": generate_cooking_tips(profile),
        "meal_prep_suggestions": generate_meal_prep_suggestions(days, profile)
    }
    
    return plan


def generate_cooking_tips(profile: Dict) -> List[str]:
    """Generate cooking tips based on profile."""
    tips = []
    
    if profile.get("cooking_condition") == "full_kitchen":
        tips.extend([
            "周末可以批量烤制鸡胸肉，冷藏保存3-4天",
            "杂粮饭一次煮3天的量，分装冷冻",
            "蔬菜提前洗好切好，密封保存"
        ])
    
    if profile.get("meal_prep"):
        tips.extend([
            "带饭的蔬菜建议选根茎类（西兰花/胡萝卜），绿叶菜当晚吃",
            "准备便携蛋白：水煮蛋、即食鸡胸、蛋白棒",
            "用分隔饭盒，主食/蛋白质/蔬菜分开存放"
        ])
    
    if profile.get("goal") == "muscle_gain":
        tips.extend([
            "训练后30分钟内补充蛋白质和碳水",
            "睡前可以补充酪蛋白（如希腊酸奶）",
            "每餐优先吃够蛋白质"
        ])
    elif profile.get("goal") == "fat_loss":
        tips.extend([
            "饭前喝一杯水，增加饱腹感",
            "蔬菜不限量，饿了先吃蔬菜",
            "烹饪方式优先蒸、煮、烤，减少油煎"
        ])
    
    allergy_terms, disliked_terms = _restriction_terms(profile)
    return [tip for tip in tips if not _matches_restrictions(tip, allergy_terms + disliked_terms)]


def generate_meal_prep_suggestions(days: List[Dict], profile: Dict[str, Any] = None) -> List[Dict]:
    """Generate meal prep suggestions based on the plan."""
    profile = profile or {}
    allergy_terms, disliked_terms = _restriction_terms(profile)
    restrictions = allergy_terms + disliked_terms
    suggestions = []
    
    # Find proteins used across days
    proteins = {}
    for day in days[:3]:  # Look at first 3 days
        for meal in day.get("meals", []):
            for food in meal.get("foods", []):
                name = food.get("name", "")
                if any(p in name for p in ["鸡胸", "牛肉", "鱼", "蛋", "豆腐"]):
                    if name not in proteins:
                        proteins[name] = 0
                    proteins[name] += food.get("amount", 0)
    
    safe_proteins = {
        name: amount for name, amount in proteins.items()
        if not _matches_restrictions(name, restrictions)
    }
    if safe_proteins:
        suggestion = {
            "category": "蛋白质批量准备",
            "items": [f"{name} {amount}g" for name, amount in safe_proteins.items()],
            "method": "周末一次性烤制/煮熟，分装冷藏"
        }
        if not _matches_restrictions(suggestion["method"], restrictions):
            suggestions.append(suggestion)
    
    # Find staple carbs
    staple_items = [
        item for item in ["杂粮饭/糙米 3天份", "红薯 2kg"]
        if not _matches_restrictions(item, restrictions)
    ]
    staple = {
        "category": "主食准备",
        "items": staple_items,
        "method": "按所列食材备料并分装冷冻"
    }
    if staple_items and not _matches_restrictions(staple["method"], restrictions):
        suggestions.append(staple)
    
    return suggestions


# ============================================================================
# FORMATTING
# ============================================================================

def format_meal_for_display(meal: Dict) -> str:
    """Format a meal for display."""
    lines = [
        f"{meal['type'].upper()} ({meal.get('time', '')}) - {meal['name']}",
        "-" * 40
    ]
    
    for food in meal.get("foods", []):
        lines.append(f"• {food['name']} {food['amount']}{food['unit']}")
    
    totals = meal.get("totals", {})
    lines.append(f"\n热量: {totals.get('calories', 0):.0f} kcal | "
                f"蛋白质: {totals.get('protein', 0):.1f}g | "
                f"碳水: {totals.get('carbs', 0):.1f}g | "
                f"脂肪: {totals.get('fat', 0):.1f}g")
    
    return "\n".join(lines)


# ============================================================================
# CLI INTERFACE
# ============================================================================

if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description='Generate meal plan')
    parser.add_argument('--days', type=int, default=7,
                       help='Plan duration in days')
    parser.add_argument('--goal', default='maintain',
                       choices=['muscle_gain', 'fat_loss', 'maintain', 'health'],
                       help='Nutrition goal')
    parser.add_argument('--output', help='Output JSON file path')

    args = parser.parse_args()

    try:
        _validate_duration_days(args.days)
    except ValueError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        sys.exit(2)

    if args.output:
        try:
            _reject_output_overwrite(
                args.output,
                [PROFILE_FILE, PLANS_FILE, LOGS_FILE, RECIPES_FILE],
            )
        except ValueError as exc:
            parser.error(str(exc))

    # Load profile
    profile = get_profile()
    if not profile.get("initialized"):
        print("Error: Nutrition profile not initialized. Please set up profile first.")
        sys.exit(1)

    # Create plan data
    plan_data = {
        "name": f"{args.days}天饮食计划",
        "duration_days": args.days,
        "goal": args.goal
    }

    # Generate plan
    try:
        plan = generate_weekly_plan(profile, plan_data)
    except ValueError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        sys.exit(2)

    # Persist the plan and optional report with compensating rollback on export failure.
    try:
        if args.output:
            plan_id = persist_plan_with_output(plan, args.output)
        else:
            plan_id = add_plan(plan)
            plan["id"] = plan_id
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"Error: {exc}", file=sys.stderr)
        sys.exit(1)

    if args.output:
        print(f"Meal plan generated: {args.output}")
        print(f"  Plan ID: {plan_id}")
    else:
        print(json.dumps(plan, indent=2))
