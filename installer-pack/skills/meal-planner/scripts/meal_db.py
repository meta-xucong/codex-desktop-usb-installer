#!/usr/bin/env python
"""
Meal Planner Database Manager

Manages user nutrition profiles, meal plans, food logs, and shopping lists.
"""

import json
import os
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
    except (json.JSONDecodeError, FileNotFoundError):
        return {}


def save_json(file_path: Path, data: Dict[str, Any]) -> None:
    """Save JSON to file."""
    ensure_db_files()
    with open(file_path, 'w') as f:
        json.dump(data, f, indent=2)


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

def calculate_bmr(profile: Dict[str, Any]) -> float:
    """Calculate Basal Metabolic Rate using Mifflin-St Jeor equation."""
    weight = profile.get("weight_kg", 70)
    height = profile.get("height_cm", 170)
    age = profile.get("age", 30)
    gender = profile.get("gender", "male")
    
    if gender.lower() == "female":
        bmr = 10 * weight + 6.25 * height - 5 * age - 161
    else:
        bmr = 10 * weight + 6.25 * height - 5 * age + 5
    
    return round(bmr, 0)


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
    
    activity = profile.get("activity_level", "moderate")
    multiplier = activity_multipliers.get(activity, 1.55)
    
    return round(bmr * multiplier, 0)


def calculate_nutrition_targets(profile: Dict[str, Any] = None) -> Dict[str, Any]:
    """Calculate daily nutrition targets based on goal."""
    if profile is None:
        profile = get_profile()
    
    tdee = calculate_tdee(profile)
    goal = profile.get("goal", "maintain")
    weight = profile.get("weight_kg", 70)
    
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
    
    # Calculate macros
    protein_g = max(round(weight * protein_per_kg), round(target_calories * protein_pct / 4))
    fat_g = round(target_calories * fat_pct / 9)
    carbs_g = round(target_calories * carbs_pct / 4)
    
    # Recalculate percentages based on actual grams
    total_calculated_calories = protein_g * 4 + carbs_g * 4 + fat_g * 9
    
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


def add_plan(plan: Dict[str, Any], status: str = "active") -> str:
    """Add a new meal plan."""
    plans = load_json(PLANS_FILE)
    
    plan_id = str(int(datetime.now().timestamp()))
    plan["id"] = plan_id
    plan["created_at"] = datetime.now().isoformat()
    plan["status"] = status
    
    if status == "active":
        plans["active_plans"].append(plan)
    elif status == "completed":
        plans["completed_plans"].append(plan)
    elif status == "template":
        plans["plan_templates"].append(plan)
    
    save_json(PLANS_FILE, plans)
    return plan_id


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
    """Substitute an ingredient in a meal plan."""
    plan = get_plan_by_id(plan_id)
    if not plan:
        return False
    
    # Update in daily meals
    for day in plan.get("days", []):
        for meal in day.get("meals", []):
            for food in meal.get("foods", []):
                if food.get("name") == old_ingredient:
                    food["name"] = new_ingredient
                    food["substituted_from"] = old_ingredient
                    food["substitution_reason"] = reason
                    food["substituted_at"] = datetime.now().isoformat()
    
    return update_plan(plan_id, {"days": plan.get("days", [])})


# ============================================================================
# FOOD LOGGING
# ============================================================================

def log_meal(meal: Dict[str, Any]) -> str:
    """Log a meal."""
    logs = load_json(LOGS_FILE)
    
    log_id = str(int(datetime.now().timestamp()))
    meal["log_id"] = log_id
    meal["logged_at"] = datetime.now().isoformat()
    
    if "food_logs" not in logs:
        logs["food_logs"] = []
    
    logs["food_logs"].append(meal)
    save_json(LOGS_FILE, logs)
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
    """Generate shopping list from meal plan."""
    plan = get_plan_by_id(plan_id)
    if not plan:
        return {}
    
    # Aggregate ingredients
    ingredients = {}
    
    for day in plan.get("days", [])[:days]:
        for meal in day.get("meals", []):
            for food in meal.get("foods", []):
                name = food.get("name", "")
                amount = food.get("amount", 0)
                unit = food.get("unit", "g")
                
                # Categorize ingredient
                category = categorize_ingredient(name)
                
                key = f"{name}_{unit}"
                if key in ingredients:
                    ingredients[key]["amount"] += amount
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
            print(json.dumps(generate_shopping_list(plan_id, days), indent=2))
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
