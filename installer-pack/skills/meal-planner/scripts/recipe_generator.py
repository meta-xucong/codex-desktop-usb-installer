#!/usr/bin/env python
"""
Recipe Generator

Generates meal plans and recipes based on nutrition targets and preferences.
"""

import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent))

from meal_db import get_profile, add_plan, calculate_nutrition_targets
import json
from datetime import datetime, timedelta
from typing import Dict, Any, List


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


# ============================================================================
# MEAL PLAN GENERATION
# ============================================================================

def filter_recipes_by_diet(recipes: List[Dict], diet_type: str) -> List[Dict]:
    """Filter recipes based on diet type."""
    if diet_type == "regular":
        return recipes
    
    # For now, return all recipes (can be expanded with diet-specific filtering)
    return recipes


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
    
    # Select meals (rotate through options based on day number)
    breakfast = breakfast_options[day_number % len(breakfast_options)]
    lunch = lunch_dinner_options[day_number % len(lunch_dinner_options)]
    dinner = lunch_dinner_options[(day_number + 1) % len(lunch_dinner_options)]
    
    meals = [
        {"type": "breakfast", "time": "07:30", **breakfast},
        {"type": "lunch", "time": "12:30", **lunch},
        {"type": "dinner", "time": "19:00", **dinner}
    ]
    
    # Add snacks if 4+ meals
    if meals_per_day >= 4:
        snack1 = snack_options[day_number % len(snack_options)]
        meals.insert(1, {"type": "snack", "time": "10:30", **snack1})
    
    if meals_per_day >= 5:
        snack2 = snack_options[(day_number + 1) % len(snack_options)]
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
    duration = plan_data.get("duration_days", 7)
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
            "meals_per_day": profile.get("meals_per_day")
        },
        "nutrition_targets": targets,
        "days": days,
        "cooking_tips": generate_cooking_tips(profile),
        "meal_prep_suggestions": generate_meal_prep_suggestions(days)
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
    
    return tips


def generate_meal_prep_suggestions(days: List[Dict]) -> List[Dict]:
    """Generate meal prep suggestions based on the plan."""
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
    
    if proteins:
        suggestions.append({
            "category": "蛋白质批量准备",
            "items": [f"{name} {amount}g" for name, amount in proteins.items()],
            "method": "周末一次性烤制/煮熟，分装冷藏"
        })
    
    # Find staple carbs
    suggestions.append({
        "category": "主食准备",
        "items": ["杂粮饭/糙米 3天份", "红薯 2kg"],
        "method": "杂粮饭煮好分装冷冻，红薯洗净备用"
    })
    
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
    plan = generate_weekly_plan(profile, plan_data)

    # Save to database
    plan_id = add_plan(plan)
    plan["id"] = plan_id

    # Output
    if args.output:
        with open(args.output, 'w') as f:
            json.dump(plan, f, indent=2)
        print(f"Meal plan generated: {args.output}")
        print(f"  Plan ID: {plan_id}")
    else:
        print(json.dumps(plan, indent=2))
