#!/usr/bin/env python
"""
Meal Planner Database Manager

Manages user nutrition profiles, meal plans, food logs, and shopping lists.
"""

import json
import math
import numbers
import os
import tempfile
import uuid
from collections.abc import Mapping
from pathlib import Path
from datetime import datetime, timedelta
from typing import Dict, Any, List, Optional, Tuple

DB_DIR = Path(os.environ.get("CODEX_DATA_DIR", Path.cwd() / "codex-data")) / "meal_planner"
PROFILE_FILE = DB_DIR / "profile.json"
PLANS_FILE = DB_DIR / "plans.json"
LOGS_FILE = DB_DIR / "logs.json"
RECIPES_FILE = DB_DIR / "recipes.json"


def ensure_db_files() -> None:
    """Ensure all database files exist."""
    DB_DIR.mkdir(parents=True, exist_ok=True)

    if not PROFILE_FILE.exists():
        default_profile = {
            "initialized": False,
            "created_at": datetime.now().isoformat(),
            "age": None,
            "gender": None,
            "height_cm": None,
            "weight_kg": None,
            "activity_level": "",  # sedentary, light, moderate, high
            "goal": "",  # muscle_gain, fat_loss, maintain, health
            "cuisine_preference": "",  # chinese, western, japanese, mixed
            "diet_type": "",  # regular, vegetarian, vegan, keto, low_carb
            "taste_preference": "",  # light, moderate, strong
            "spice_level": "",  # none, mild, medium, hot
            "allergies": [],
            "disliked_foods": [],
            "cooking_condition": "",  # full_kitchen, microwave, mostly_eat_out
            "budget_level": "",  # economy, standard, premium
            "meals_per_day": 3,
            "meal_prep": False
        }
        PROFILE_FILE.write_text(json.dumps(default_profile, indent=2))

    if not PLANS_FILE.exists():
        default_plans = {
            "active_plans": [],
            "completed_plans": [],
            "plan_templates": []
        }
        PLANS_FILE.write_text(json.dumps(default_plans, indent=2))

    if not LOGS_FILE.exists():
        default_logs = {
            "food_logs": []
        }
        LOGS_FILE.write_text(json.dumps(default_logs, indent=2))

    if not RECIPES_FILE.exists():
        default_recipes = {
            "custom_recipes": [],
            "favorite_recipes": []
        }
        RECIPES_FILE.write_text(json.dumps(default_recipes, indent=2))


def load_json(file_path: Path) -> Dict[str, Any]:
    """Load JSON from file."""
    ensure_db_files()
    try:
        with open(file_path, 'r') as f:
            return json.load(f)
    except json.JSONDecodeError as exc:
        raise ValueError(
            f"Meal planner data file '{file_path.name}' contains invalid JSON; refusing to continue."
        ) from exc
    except FileNotFoundError:
        return {}


def save_json(file_path: Path, data: Dict[str, Any]) -> None:
    """Atomically save JSON without truncating the current copy on failure."""
    ensure_db_files()
    serialized = json.dumps(data, indent=2)
    temporary_path = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", dir=file_path.parent,
            prefix=f".{file_path.name}.", suffix=".tmp", delete=False,
        ) as handle:
            temporary_path = Path(handle.name)
            handle.write(serialized)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary_path, file_path)
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)


# ============================================================================
# PROFILE MANAGEMENT
# ============================================================================

def is_initialized() -> bool:
    """Check if nutrition profile is initialized."""
    profile = load_json(PROFILE_FILE)
    return profile.get("initialized", False)


def get_profile() -> Dict[str, Any]:
    """Get user nutrition profile."""
    return load_json(PROFILE_FILE)


def save_profile(profile: Dict[str, Any]) -> None:
    """Save user nutrition profile."""
    data = load_json(PROFILE_FILE)
    data.update(profile)
    data["initialized"] = True
    data["last_updated"] = datetime.now().isoformat()
    save_json(PROFILE_FILE, data)


def update_profile_field(key: str, value: Any) -> None:
    """Update a specific profile field."""
    profile = load_json(PROFILE_FILE)
    profile[key] = value
    profile["last_updated"] = datetime.now().isoformat()
    save_json(PROFILE_FILE, profile)


# ============================================================================
# NUTRITION CALCULATIONS
# ============================================================================

def _positive_profile_number(profile: Dict[str, Any], field: str) -> float:
    """Read one required, finite, positive numeric profile field."""
    value = profile.get(field)
    if isinstance(value, bool) or not isinstance(value, numbers.Real):
        raise ValueError(f"{field} 必须是有限且大于 0 的数值。")
    try:
        value = float(value)
    except (OverflowError, TypeError, ValueError) as exc:
        raise ValueError(f"{field} 必须是有限且大于 0 的数值。") from exc
    if not math.isfinite(value) or value <= 0:
        raise ValueError(f"{field} 必须是有限且大于 0 的数值。")
    return value


def _finite_positive_result(value: float, field: str) -> float:
    """Reject unusable derived nutrition values before rounding or division."""
    try:
        finite = math.isfinite(value)
    except (OverflowError, TypeError, ValueError):
        finite = False
    if not finite or value <= 0:
        raise ValueError(f"{field} 必须是有限且大于 0 的数值。")
    return value


def _profile_enum(profile: Dict[str, Any], field: str, default: str, choices: set[str]) -> str:
    """Use a compatibility default only when an enum field is truly absent."""
    value = profile.get(field, default)
    if not isinstance(value, str):
        raise ValueError(f"{field} 必须是以下值之一: {', '.join(sorted(choices))}。")
    normalized = value.strip().casefold()
    if not normalized or normalized not in choices:
        raise ValueError(f"{field} 必须是以下值之一: {', '.join(sorted(choices))}。")
    return normalized

def calculate_bmr(profile: Dict[str, Any]) -> float:
    """Calculate Basal Metabolic Rate using Mifflin-St Jeor equation."""
    weight = _positive_profile_number(profile, "weight_kg")
    height = _positive_profile_number(profile, "height_cm")
    age = _positive_profile_number(profile, "age")
    gender = profile.get("gender")
    if not isinstance(gender, str) or not gender.strip():
        raise ValueError("gender 必须是 male 或 female。")
    gender = gender.strip().casefold()
    if gender not in {"male", "female"}:
        raise ValueError("gender 必须是 male 或 female。")

    try:
        if gender == "female":
            bmr = 10 * weight + 6.25 * height - 5 * age - 161
        else:
            bmr = 10 * weight + 6.25 * height - 5 * age + 5
    except OverflowError as exc:
        raise ValueError("BMR 必须是有限且大于 0 的数值。") from exc
    return _finite_positive_result(round(_finite_positive_result(bmr, "BMR"), 0), "BMR")


def calculate_tdee(profile: Dict[str, Any]) -> float:
    """Calculate Total Daily Energy Expenditure."""
    bmr = calculate_bmr(profile)
    
    activity_multipliers = {
        "sedentary": 1.2,      # 久坐
        "light": 1.375,        # 轻度活动
        "moderate": 1.55,      # 中度活动
        "high": 1.725,         # 高度活动
        "very_high": 1.9       # 极高活动
    }
    
    activity = _profile_enum(profile, "activity_level", "moderate", set(activity_multipliers))
    multiplier = activity_multipliers[activity]
    
    try:
        tdee = bmr * multiplier
    except OverflowError as exc:
        raise ValueError("TDEE 必须是有限且大于 0 的数值。") from exc
    return _finite_positive_result(round(_finite_positive_result(tdee, "TDEE"), 0), "TDEE")


def calculate_nutrition_targets(profile: Dict[str, Any] = None) -> Dict[str, Any]:
    """Calculate daily nutrition targets based on goal."""
    if profile is None:
        profile = get_profile()
    
    tdee = calculate_tdee(profile)
    goal = _profile_enum(profile, "goal", "maintain", {"muscle_gain", "fat_loss", "maintain", "health"})
    weight = _positive_profile_number(profile, "weight_kg")
    
    # Adjust calories based on goal
    if goal == "muscle_gain":
        target_calories = tdee + 400
        protein_per_kg = 2.0
        protein_pct = 0.30
        carbs_pct = 0.45
        fat_pct = 0.25
    elif goal == "fat_loss":
        target_calories = tdee - 400
        protein_per_kg = 2.2
        protein_pct = 0.35
        carbs_pct = 0.35
        fat_pct = 0.30
    else:  # maintain or health
        target_calories = tdee
        protein_per_kg = 1.6
        protein_pct = 0.25
        carbs_pct = 0.50
        fat_pct = 0.25
    
    _finite_positive_result(target_calories, "target_calories")

    # Calculate macros
    try:
        protein_g = max(round(weight * protein_per_kg), round(target_calories * protein_pct / 4))
        fat_g = round(target_calories * fat_pct / 9)
        carbs_g = round(target_calories * carbs_pct / 4)
    
        # Recalculate percentages based on actual grams
        total_calculated_calories = protein_g * 4 + carbs_g * 4 + fat_g * 9
    except (OverflowError, ZeroDivisionError) as exc:
        raise ValueError("nutrition targets 必须是有限且有效的数值。") from exc
    _finite_positive_result(total_calculated_calories, "total_calculated_calories")
    
    return {
        "calories": round(target_calories),
        "protein_g": protein_g,
        "carbs_g": carbs_g,
        "fat_g": fat_g,
        "protein_pct": round(protein_g * 4 / total_calculated_calories * 100),
        "carbs_pct": round(carbs_g * 4 / total_calculated_calories * 100),
        "fat_pct": round(fat_g * 9 / total_calculated_calories * 100),
        "bmr": calculate_bmr(profile),
        "tdee": tdee
    }


# ============================================================================
# MEAL PLAN MANAGEMENT
# ============================================================================

def get_plans(status: str = "all") -> Dict[str, List[Dict]]:
    """Get meal plans by status."""
    plans = load_json(PLANS_FILE)
    
    if status == "all":
        return plans
    elif status == "active":
        return {"active_plans": plans.get("active_plans", [])}
    elif status == "completed":
        return {"completed_plans": plans.get("completed_plans", [])}
    elif status == "templates":
        return {"plan_templates": plans.get("plan_templates", [])}
    return {}


def add_plan(
    plan: Dict[str, Any],
    status: str = "active",
    *,
    plan_id: str | None = None,
    created_at: str | None = None,
) -> str:
    """Add a new meal plan."""
    plans = load_json(PLANS_FILE)
    
    plan_id = plan_id or uuid.uuid4().hex
    plan["id"] = plan_id
    plan["created_at"] = created_at or datetime.now().isoformat()
    plan["status"] = status
    
    if status == "active":
        plans["active_plans"].append(plan)
    elif status == "completed":
        plans["completed_plans"].append(plan)
    elif status == "template":
        plans["plan_templates"].append(plan)
    
    save_json(PLANS_FILE, plans)
    return plan_id


def remove_plan(plan_id: str) -> bool:
    """Remove a plan by ID; used to compensate a failed paired CLI export."""
    plans = load_json(PLANS_FILE)
    for key in ["active_plans", "completed_plans", "plan_templates"]:
        entries = plans.get(key, [])
        for index, plan in enumerate(entries):
            if plan.get("id") == plan_id:
                del entries[index]
                save_json(PLANS_FILE, plans)
                return True
    return False


def get_plan_by_id(plan_id: str) -> Optional[Dict[str, Any]]:
    """Get a specific plan by ID."""
    plans = load_json(PLANS_FILE)
    
    for key in ["active_plans", "completed_plans", "plan_templates"]:
        for plan in plans.get(key, []):
            if plan.get("id") == plan_id:
                return plan
    return None


def update_plan(plan_id: str, updates: Dict[str, Any]) -> bool:
    """Update a meal plan."""
    plans = load_json(PLANS_FILE)
    
    for key in ["active_plans", "completed_plans", "plan_templates"]:
        for plan in plans.get(key, []):
            if plan.get("id") == plan_id:
                plan.update(updates)
                plan["updated_at"] = datetime.now().isoformat()
                save_json(PLANS_FILE, plans)
                return True
    return False


def substitute_ingredient(plan_id: str, old_ingredient: str, 
                         new_ingredient: str, reason: str = "") -> bool:
    """Substitute an ingredient and invalidate nutrition derived from it.

    This API has no replacement nutrition source. Once a food name changes,
    its old nutrient values and every containing aggregate are therefore
    removed and marked for recalculation instead of being presented as current.
    """
    plan = get_plan_by_id(plan_id)
    if not plan:
        return False
    
    matched = False
    for day in plan.get("days", []):
        day_matched = False
        for meal in day.get("meals", []):
            meal_matched = False
            for food in meal.get("foods", []):
                if food.get("name") == old_ingredient:
                    matched = True
                    day_matched = True
                    meal_matched = True
                    food["name"] = new_ingredient
                    food["substituted_from"] = old_ingredient
                    food["substitution_reason"] = reason
                    food["substituted_at"] = datetime.now().isoformat()
                    food["nutrition_status"] = "needs_recalculation"
                    for nutrient in ("calories", "protein", "carbs", "fat"):
                        food.pop(nutrient, None)

            if meal_matched:
                meal["nutrition_status"] = "needs_recalculation"
                meal.pop("totals", None)
        if day_matched:
            day["nutrition_status"] = "needs_recalculation"
            day.pop("daily_totals", None)
            day.pop("targets_comparison", None)

    if not matched:
        return False

    plan["nutrition_status"] = "needs_recalculation"
    return update_plan(plan_id, {
        "days": plan.get("days", []),
        "nutrition_status": "needs_recalculation",
    })


# ============================================================================
# FOOD LOGGING
# ============================================================================

def log_meal(meal: Dict[str, Any]) -> str:
    """Log a meal after validating all supplied food nutrient values."""
    if not isinstance(meal, dict):
        raise ValueError("meal must be an object")
    foods = meal.get("foods", [])
    if not isinstance(foods, list):
        raise ValueError("foods must be a list")
    for index, food in enumerate(foods):
        if not isinstance(food, dict):
            raise ValueError(f"foods[{index}] must be an object")
        for nutrient in ("calories", "protein", "carbs", "fat"):
            value = food.get(nutrient, 0)
            if isinstance(value, bool) or not isinstance(value, (int, float)):
                raise ValueError(f"foods[{index}].{nutrient} must be a finite non-negative number")
            try:
                finite = math.isfinite(value)
            except (OverflowError, TypeError, ValueError):
                finite = False
            if not finite or value < 0:
                raise ValueError(f"foods[{index}].{nutrient} must be a finite non-negative number")

    logs = load_json(LOGS_FILE)
    
    log_id = uuid.uuid4().hex
    logged_meal = dict(meal)
    logged_meal["log_id"] = log_id
    logged_meal["logged_at"] = datetime.now().isoformat()
    
    if "food_logs" not in logs:
        logs["food_logs"] = []
    
    logs["food_logs"].append(logged_meal)
    save_json(LOGS_FILE, logs)
    # Preserve the historical successful-call behavior while invalid input
    # never mutates the caller and failed persistence does not mutate it either.
    meal.update({"log_id": log_id, "logged_at": logged_meal["logged_at"]})
    return log_id


def get_logs(date: str = None, limit: int = None) -> List[Dict[str, Any]]:
    """Get food logs, optionally filtered by date."""
    logs = load_json(LOGS_FILE)
    all_logs = logs.get("food_logs", [])
    
    if date:
        all_logs = [l for l in all_logs if l.get("date") == date]
    
    # Sort by date, newest first
    all_logs.sort(key=lambda x: x.get("logged_at", ""), reverse=True)
    
    if limit:
        return all_logs[:limit]
    return all_logs


def get_daily_summary(date: str = None) -> Dict[str, Any]:
    """Get nutrition summary for a specific date."""
    if date is None:
        date = datetime.now().strftime("%Y-%m-%d")
    
    logs = get_logs(date)
    profile = get_profile()
    targets = calculate_nutrition_targets(profile)
    
    total_calories = 0
    total_protein = 0
    total_carbs = 0
    total_fat = 0
    
    meals_breakdown = {
        "breakfast": {"calories": 0, "foods": []},
        "lunch": {"calories": 0, "foods": []},
        "dinner": {"calories": 0, "foods": []},
        "snack": {"calories": 0, "foods": []}
    }
    
    for log in logs:
        meal_type = log.get("meal_type", "snack")
        foods = log.get("foods", [])
        
        for food in foods:
            cals = food.get("calories", 0)
            total_calories += cals
            total_protein += food.get("protein", 0)
            total_carbs += food.get("carbs", 0)
            total_fat += food.get("fat", 0)
            
            if meal_type in meals_breakdown:
                meals_breakdown[meal_type]["calories"] += cals
                meals_breakdown[meal_type]["foods"].append(food.get("name"))
    
    return {
        "date": date,
        "totals": {
            "calories": round(total_calories, 1),
            "protein_g": round(total_protein, 1),
            "carbs_g": round(total_carbs, 1),
            "fat_g": round(total_fat, 1)
        },
        "targets": targets,
        "percentages": {
            "calories": round(total_calories / targets["calories"] * 100, 1) if targets["calories"] > 0 else 0,
            "protein": round(total_protein / targets["protein_g"] * 100, 1) if targets["protein_g"] > 0 else 0,
            "carbs": round(total_carbs / targets["carbs_g"] * 100, 1) if targets["carbs_g"] > 0 else 0,
            "fat": round(total_fat / targets["fat_g"] * 100, 1) if targets["fat_g"] > 0 else 0
        },
        "meals": meals_breakdown,
        "remaining": {
            "calories": max(0, round(targets["calories"] - total_calories, 1)),
            "protein_g": max(0, round(targets["protein_g"] - total_protein, 1))
        }
    }


# ============================================================================
# SHOPPING LIST GENERATION
# ============================================================================

def generate_shopping_list(plan_id: str, days: int = 7) -> Dict[str, List[Dict]]:
    """Generate a shopping list using the restrictions frozen into the plan."""
    plan = get_plan_by_id(plan_id)
    if not plan:
        return {}

    snapshot = plan.get("restriction_snapshot")
    if not isinstance(snapshot, dict):
        raise ValueError("plan restriction_snapshot is missing or invalid")
    restrictions = {}
    for key in ("allergies", "disliked_foods"):
        values = snapshot.get(key)
        if not isinstance(values, list) or any(
            not isinstance(value, str) or not value.strip() for value in values
        ):
            raise ValueError(f"plan restriction_snapshot.{key} must be a list of non-empty strings")
        restrictions[key] = [value.strip() for value in values]

    is_restricted = bool(restrictions["allergies"] or restrictions["disliked_foods"])
    matches_restriction = None
    if is_restricted:
        # Keep the same Unicode normalization and food alias table as recipe
        # generation. If that canonical filter is unavailable, fail closed.
        try:
            from recipe_generator import _matches_restrictions
        except (ImportError, AttributeError) as exc:
            raise ValueError("cannot safely apply plan restrictions to shopping list") from exc
        matches_restriction = _matches_restrictions

    plan_days = plan.get("days")
    if not isinstance(plan_days, list):
        raise ValueError("plan.days must be a list")

    # Validate every nested entry and amount before aggregation. A malformed
    # later day must not leave a partial list that looks complete.
    validated_days = []
    for day_index, day in enumerate(plan_days):
        day_path = f"plan.days[{day_index}]"
        if not isinstance(day, Mapping):
            raise ValueError(f"{day_path} must be a mapping")
        meals = day.get("meals")
        if not isinstance(meals, list):
            raise ValueError(f"{day_path}.meals must be a list")
        validated_foods = []
        for meal_index, meal in enumerate(meals):
            meal_path = f"{day_path}.meals[{meal_index}]"
            if not isinstance(meal, Mapping):
                raise ValueError(f"{meal_path} must be a mapping")
            foods = meal.get("foods")
            if not isinstance(foods, list):
                raise ValueError(f"{meal_path}.foods must be a list")
            for food_index, food in enumerate(foods):
                food_path = f"{meal_path}.foods[{food_index}]"
                if not isinstance(food, Mapping):
                    raise ValueError(f"{food_path} must be a mapping")
                name = food.get("name")
                if not isinstance(name, str) or not name.strip():
                    raise ValueError(f"{food_path}.name must be a non-empty string")
                amount = food.get("amount")
                try:
                    amount_is_finite = (
                        not isinstance(amount, bool)
                        and isinstance(amount, numbers.Real)
                        and math.isfinite(amount)
                    )
                except (OverflowError, TypeError, ValueError):
                    amount_is_finite = False
                if not amount_is_finite or amount < 0:
                    raise ValueError(f"{food_path}.amount must be a finite non-negative number")
                validated_foods.append((name.strip(), amount, food.get("unit", "g")))
        validated_days.append(validated_foods)

    # Aggregate only after the complete plan has passed structural and amount checks.
    ingredients = {}
    for day_foods in validated_days[:days]:
        for name, amount, unit in day_foods:
            if matches_restriction and any(
                matches_restriction(name, terms)
                for terms in (restrictions["allergies"], restrictions["disliked_foods"])
                if terms
            ):
                continue
            category = categorize_ingredient(name)
            key = f"{name}_{unit}"
            if key in ingredients:
                combined_amount = ingredients[key]["amount"] + amount
                if not math.isfinite(combined_amount):
                    raise ValueError(f"shopping list amount for {name!r} overflowed its finite range")
                ingredients[key]["amount"] = combined_amount
            else:
                ingredients[key] = {
                    "name": name,
                    "amount": amount,
                    "unit": unit,
                    "category": category
                }
    
    # Group by category
    shopping_list = {
        "meat_protein": [],
        "vegetables": [],
        "fruits": [],
        "grains": [],
        "dairy": [],
        "condiments": [],
        "other": []
    }
    
    for item in ingredients.values():
        category = item["category"]
        if category in shopping_list:
            shopping_list[category].append(item)
    
    # Sort items by name within each category
    for category in shopping_list:
        shopping_list[category].sort(key=lambda x: x["name"])
    
    return shopping_list


def categorize_ingredient(name: str) -> str:
    """Categorize an ingredient."""
    categories = {
        "meat_protein": ["鸡胸", "牛肉", "鱼", "虾", "蛋", "肉", "鸡胸", "三文鱼", "鳕鱼", "豆腐", "希腊酸奶"],
        "vegetables": ["菜", "瓜", "茄", "菇", "笋", "豆", "西兰花", "菠菜", "番茄", "黄瓜", "胡萝卜"],
        "fruits": ["果", "蕉", "莓", "瓜", "苹", "橙", "梨", "桃", "葡萄"],
        "grains": ["米", "面", "麦", "燕麦", "红薯", "土豆", "玉米", "面包", "馒头"],
        "dairy": ["奶", "奶酪", "酸奶", "芝士", "黄油"],
        "condiments": ["油", "盐", "酱", "醋", "糖", "胡椒", "调料"]
    }
    
    for category, keywords in categories.items():
        if any(kw in name for kw in keywords):
            return category
    
    return "other"


# ============================================================================
# STATISTICS & INSIGHTS
# ============================================================================

def get_nutrition_stats(days: int = 7) -> Dict[str, Any]:
    """Get nutrition statistics for recent days."""
    profile = get_profile()
    targets = calculate_nutrition_targets(profile)
    
    # Get last N days
    dates = []
    for i in range(days):
        date = (datetime.now() - timedelta(days=i)).strftime("%Y-%m-%d")
        dates.append(date)
    
    daily_summaries = []
    for date in dates:
        summary = get_daily_summary(date)
        daily_summaries.append(summary)
    
    # Calculate averages
    avg_calories = sum(s["totals"]["calories"] for s in daily_summaries) / len(daily_summaries)
    avg_protein = sum(s["totals"]["protein_g"] for s in daily_summaries) / len(daily_summaries)
    
    # Count days meeting protein target
    protein_target_days = sum(1 for s in daily_summaries if s["percentages"]["protein"] >= 90)
    
    return {
        "period_days": days,
        "average_daily": {
            "calories": round(avg_calories, 1),
            "protein_g": round(avg_protein, 1)
        },
        "targets": targets,
        "compliance": {
            "protein_target_days": protein_target_days,
            "protein_target_percentage": round(protein_target_days / days * 100, 1)
        },
        "daily_breakdown": daily_summaries
    }


# ============================================================================
# UTILITY FUNCTIONS
# ============================================================================

def export_all() -> Dict[str, Any]:
    """Export all meal planning data."""
    return {
        "profile": get_profile(),
        "plans": get_plans("all"),
        "logs": get_logs(),
        "stats": get_nutrition_stats(30),
        "exported_at": datetime.now().isoformat()
    }


def reset_all() -> None:
    """Reset all data (use with caution)."""
    for file_path in [PROFILE_FILE, PLANS_FILE, LOGS_FILE, RECIPES_FILE]:
        if file_path.exists():
            file_path.unlink()
    ensure_db_files()


# ============================================================================
# CLI INTERFACE
# ============================================================================

if __name__ == "__main__":
    import sys

    if len(sys.argv) < 2:
        print("Meal Planner Database Manager")
        print("\nUsage:")
        print("  python meal_db.py is_initialized")
        print("  python meal_db.py get_profile")
        print("  python meal_db.py targets")
        print("  python meal_db.py today")
        print("  python meal_db.py stats [days]")
        print("  python meal_db.py shopping_list [plan_id] --days 7")
        print("  python meal_db.py export")
        print("  python meal_db.py reset")
        sys.exit(1)

    command = sys.argv[1]

    if command == "is_initialized":
        print("true" if is_initialized() else "false")
    elif command == "get_profile":
        print(json.dumps(get_profile(), indent=2))
    elif command == "targets":
        print(json.dumps(calculate_nutrition_targets(), indent=2))
    elif command == "today":
        print(json.dumps(get_daily_summary(), indent=2))
    elif command == "stats":
        days = int(sys.argv[2]) if len(sys.argv) > 2 else 7
        print(json.dumps(get_nutrition_stats(days), indent=2))
    elif command == "shopping_list":
        plan_id = sys.argv[2] if len(sys.argv) > 2 else None
        days = int(sys.argv[4]) if len(sys.argv) > 4 and sys.argv[3] == "--days" else 7
        if plan_id:
            try:
                print(json.dumps(generate_shopping_list(plan_id, days), indent=2))
            except ValueError as exc:
                print(f"Error: cannot generate shopping list: {exc}", file=sys.stderr)
                sys.exit(1)
        else:
            print("Error: plan_id required")
    elif command == "export":
        print(json.dumps(export_all(), indent=2))
    elif command == "reset":
        confirm = input("Are you sure you want to reset all meal planning data? (yes/no): ")
        if confirm.lower() == "yes":
            reset_all()
            print("All meal planning data has been reset.")
        else:
            print("Reset cancelled.")
    else:
        print(f"Unknown command: {command}")
        sys.exit(1)
