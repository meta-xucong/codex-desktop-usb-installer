#!/usr/bin/env python
"""
Fitness Program Generator

Generates periodized training programs based on user profile and goals.
"""

import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent))

from fitness_db import get_profile, add_program
import json
from datetime import datetime, timedelta
from typing import Dict, Any, List


# ============================================================================
# EXERCISE DATABASE
# ============================================================================

EXERCISE_LIBRARY = {
    "chest": {
        "compound": [
            {"name": "杠铃卧推", "equipment": ["barbell", "gym"], "difficulty": "intermediate"},
            {"name": "哑铃卧推", "equipment": ["dumbbells", "gym"], "difficulty": "beginner"},
            {"name": "俯卧撑", "equipment": ["none"], "difficulty": "beginner"},
            {"name": "上斜杠铃卧推", "equipment": ["barbell", "gym"], "difficulty": "intermediate"},
            {"name": "上斜哑铃推举", "equipment": ["dumbbells", "gym"], "difficulty": "beginner"},
        ],
        "isolation": [
            {"name": "哑铃飞鸟", "equipment": ["dumbbells", "gym"], "difficulty": "beginner"},
            {"name": "绳索夹胸", "equipment": ["gym"], "difficulty": "beginner"},
            {"name": "器械夹胸", "equipment": ["gym"], "difficulty": "beginner"},
        ]
    },
    "back": {
        "compound": [
            {"name": "引体向上", "equipment": ["none", "gym"], "difficulty": "intermediate"},
            {"name": "高位下拉", "equipment": ["gym"], "difficulty": "beginner"},
            {"name": "杠铃划船", "equipment": ["barbell", "gym"], "difficulty": "intermediate"},
            {"name": "哑铃划船", "equipment": ["dumbbells", "gym"], "difficulty": "beginner"},
            {"name": "坐姿划船", "equipment": ["gym"], "difficulty": "beginner"},
        ],
        "isolation": [
            {"name": "直臂下压", "equipment": ["gym"], "difficulty": "intermediate"},
            {"name": "面拉", "equipment": ["gym", "bands"], "difficulty": "beginner"},
            {"name": "哑铃耸肩", "equipment": ["dumbbells", "gym"], "difficulty": "beginner"},
        ]
    },
    "legs": {
        "compound": [
            {"name": "杠铃深蹲", "equipment": ["barbell", "gym"], "difficulty": "intermediate"},
            {"name": "高脚杯深蹲", "equipment": ["dumbbells", "gym"], "difficulty": "beginner"},
            {"name": "哑铃深蹲", "equipment": ["dumbbells"], "difficulty": "beginner"},
            {"name": "罗马尼亚硬拉", "equipment": ["barbell", "dumbbells", "gym"], "difficulty": "intermediate"},
            {"name": "传统硬拉", "equipment": ["barbell", "gym"], "difficulty": "advanced"},
            {"name": "腿举", "equipment": ["gym"], "difficulty": "beginner"},
            {"name": "箭步蹲", "equipment": ["none", "dumbbells", "gym"], "difficulty": "beginner"},
            {"name": "保加利亚分腿蹲", "equipment": ["dumbbells", "gym"], "difficulty": "intermediate"},
        ],
        "isolation": [
            {"name": "腿屈伸", "equipment": ["gym"], "difficulty": "beginner"},
            {"name": "腿弯举", "equipment": ["gym"], "difficulty": "beginner"},
            {"name": "提踵", "equipment": ["none", "dumbbells", "gym"], "difficulty": "beginner"},
        ]
    },
    "shoulders": {
        "compound": [
            {"name": "杠铃肩推", "equipment": ["barbell", "gym"], "difficulty": "intermediate"},
            {"name": "哑铃肩推", "equipment": ["dumbbells", "gym"], "difficulty": "beginner"},
            {"name": "阿诺德推举", "equipment": ["dumbbells", "gym"], "difficulty": "intermediate"},
        ],
        "isolation": [
            {"name": "侧平举", "equipment": ["dumbbells", "gym"], "difficulty": "beginner"},
            {"name": "前平举", "equipment": ["dumbbells", "gym"], "difficulty": "beginner"},
            {"name": "俯身飞鸟", "equipment": ["dumbbells", "gym"], "difficulty": "beginner"},
            {"name": "面拉", "equipment": ["gym", "bands"], "difficulty": "beginner"},
        ]
    },
    "arms": {
        "biceps": [
            {"name": "杠铃弯举", "equipment": ["barbell", "gym"], "difficulty": "beginner"},
            {"name": "哑铃弯举", "equipment": ["dumbbells", "gym"], "difficulty": "beginner"},
            {"name": "锤式弯举", "equipment": ["dumbbells", "gym"], "difficulty": "beginner"},
            {"name": "牧师凳弯举", "equipment": ["gym"], "difficulty": "intermediate"},
        ],
        "triceps": [
            {"name": "绳索下压", "equipment": ["gym"], "difficulty": "beginner"},
            {"name": "仰卧臂屈伸", "equipment": ["barbell", "dumbbells", "gym"], "difficulty": "intermediate"},
            {"name": "双杠臂屈伸", "equipment": ["none", "gym"], "difficulty": "intermediate"},
            {"name": "哑铃颈后臂屈伸", "equipment": ["dumbbells"], "difficulty": "beginner"},
            {"name": "窄距卧推", "equipment": ["barbell", "gym"], "difficulty": "intermediate"},
        ]
    },
    "core": [
        {"name": "平板支撑", "equipment": ["none"], "difficulty": "beginner"},
        {"name": "死虫式", "equipment": ["none"], "difficulty": "beginner"},
        {"name": "卷腹", "equipment": ["none"], "difficulty": "beginner"},
        {"name": "悬垂举腿", "equipment": ["gym"], "difficulty": "intermediate"},
        {"name": "俄罗斯转体", "equipment": ["none", "dumbbells"], "difficulty": "beginner"},
        {"name": "Pallof Press", "equipment": ["gym", "bands"], "difficulty": "intermediate"},
    ]
}


# ============================================================================
# PROGRAM TEMPLATES
# ============================================================================

SPLIT_TEMPLATES = {
    "full_body": {
        "name": "全身训练",
        "description": "适合2-3天/周，每次训练全身主要肌群",
        "days_per_week": [2, 3],
        "structure": [
            {"day": 1, "focus": "全身A", "muscle_groups": ["chest", "back", "legs", "shoulders"]},
            {"day": 2, "focus": "全身B", "muscle_groups": ["back", "chest", "legs", "arms"]},
            {"day": 3, "focus": "全身C", "muscle_groups": ["legs", "shoulders", "back", "core"]},
        ]
    },
    "upper_lower": {
        "name": "上下肢分化",
        "description": "适合4天/周，上肢和下肢分开训练",
        "days_per_week": [4],
        "structure": [
            {"day": 1, "focus": "上肢推", "muscle_groups": ["chest", "shoulders", "triceps"]},
            {"day": 2, "focus": "下肢", "muscle_groups": ["legs", "core"]},
            {"day": 3, "focus": "上肢拉", "muscle_groups": ["back", "biceps", "rear_delts"]},
            {"day": 4, "focus": "下肢+核心", "muscle_groups": ["legs", "core"]},
        ]
    },
    "ppl": {
        "name": "推拉腿",
        "description": "适合5-6天/周，按动作模式分化",
        "days_per_week": [5, 6],
        "structure": [
            {"day": 1, "focus": "推日", "muscle_groups": ["chest", "shoulders", "triceps"]},
            {"day": 2, "focus": "拉日", "muscle_groups": ["back", "biceps", "rear_delts"]},
            {"day": 3, "focus": "腿日", "muscle_groups": ["legs", "core"]},
            {"day": 4, "focus": "推日", "muscle_groups": ["chest", "shoulders", "triceps"]},
            {"day": 5, "focus": "拉日", "muscle_groups": ["back", "biceps", "rear_delts"]},
            {"day": 6, "focus": "腿日", "muscle_groups": ["legs", "core"]},
        ]
    },
    "bro_split": {
        "name": "部位分化",
        "description": "适合5-6天/周，每天专注1-2个肌群",
        "days_per_week": [5, 6],
        "structure": [
            {"day": 1, "focus": "胸部", "muscle_groups": ["chest", "triceps"]},
            {"day": 2, "focus": "背部", "muscle_groups": ["back", "biceps"]},
            {"day": 3, "focus": "肩部", "muscle_groups": ["shoulders", "traps"]},
            {"day": 4, "focus": "腿部", "muscle_groups": ["legs", "calves"]},
            {"day": 5, "focus": "手臂", "muscle_groups": ["biceps", "triceps"]},
            {"day": 6, "focus": "弱项强化", "muscle_groups": ["focus_areas"]},
        ]
    }
}


# ============================================================================
# PROGRAM PARAMETERS BY GOAL
# ============================================================================

PROGRAM_PARAMETERS = {
    "hypertrophy": {
        "sets_per_exercise": {"beginner": 3, "intermediate": 4, "advanced": 4},
        "reps_range": {"week1_2": "12-15", "week3_6": "10-12", "week7_10": "8-10", "week11_12": "6-10"},
        "rest_seconds": {"compound": 90, "isolation": 60},
        "volume_progression": "increase_sets_then_weight",
        "intensity_techniques": ["drop_sets", "rest_pause"]
    },
    "strength": {
        "sets_per_exercise": {"beginner": 5, "intermediate": 5, "advanced": 5},
        "reps_range": {"week1_2": "8-10", "week3_6": "5-8", "week7_10": "3-6", "week11_12": "1-5"},
        "rest_seconds": {"compound": 180, "isolation": 90},
        "volume_progression": "increase_weight_reduce_reps",
        "intensity_techniques": ["clusters", "paused_reps"]
    },
    "fat_loss": {
        "sets_per_exercise": {"beginner": 3, "intermediate": 4, "advanced": 4},
        "reps_range": {"week1_2": "12-15", "week3_6": "10-15", "week7_10": "12-20", "week11_12": "15-20"},
        "rest_seconds": {"compound": 60, "isolation": 45},
        "volume_progression": "increase_density",
        "intensity_techniques": ["supersets", "circuits"]
    },
    "endurance": {
        "sets_per_exercise": {"beginner": 2, "intermediate": 3, "advanced": 3},
        "reps_range": {"week1_2": "15-20", "week3_6": "15-25", "week7_10": "20-30", "week11_12": "15-20"},
        "rest_seconds": {"compound": 45, "isolation": 30},
        "volume_progression": "increase_reps",
        "intensity_techniques": ["circuit_training"]
    }
}


# ============================================================================
# EXERCISE SELECTION
# ============================================================================

def filter_exercises_by_equipment(exercises: List[Dict], available_equipment: List[str]) -> List[Dict]:
    """Filter exercises based on available equipment."""
    filtered = []
    for ex in exercises:
        # Check if any required equipment is available
        if any(eq in available_equipment for eq in ex.get("equipment", [])):
            filtered.append(ex)
    return filtered


def select_exercises(muscle_groups: List[str], equipment: List[str], 
                     experience: str, num_exercises: int = 4) -> List[Dict]:
    """Select appropriate exercises for given muscle groups."""
    selected = []
    
    for muscle in muscle_groups:
        muscle_key = muscle.replace("_", " ")
        
        # Get exercises for this muscle group
        if muscle in EXERCISE_LIBRARY:
            muscle_data = EXERCISE_LIBRARY[muscle]
            
            # Collect all exercises for this muscle
            all_exercises = []
            
            if isinstance(muscle_data, dict):
                for category in ["compound", "isolation"]:
                    if category in muscle_data:
                        exercises = filter_exercises_by_equipment(
                            muscle_data[category], equipment
                        )
                        all_exercises.extend(exercises)
            elif isinstance(muscle_data, list):
                all_exercises = filter_exercises_by_equipment(muscle_data, equipment)
            
            # Filter by experience level
            suitable = [ex for ex in all_exercises 
                       if experience == "advanced" or 
                       ex.get("difficulty", "beginner") in ["beginner", experience]]
            
            # Select first suitable exercise
            if suitable and len(selected) < num_exercises:
                selected.append(suitable[0])
    
    return selected


def get_exercise_tips(exercise_name: str) -> str:
    """Get form tips for an exercise."""
    tips = {
        "杠铃卧推": "肩胛骨收紧，核心稳定，控制离心",
        "哑铃卧推": "手腕保持中立，底部充分拉伸",
        "俯卧撑": "身体成一条直线，核心收紧",
        "引体向上": "肩胛骨下沉启动，控制下降",
        "杠铃划船": "背部平直，肩胛骨后收",
        "哑铃划船": "支撑手稳定，背部发力",
        "杠铃深蹲": "膝盖对准脚尖，蹲至大腿平行",
        "高脚杯深蹲": "哑铃贴胸，核心收紧",
        "罗马尼亚硬拉": "膝盖微屈，感受腘绳肌拉伸",
        "杠铃肩推": "核心收紧，避免过度后仰",
        "哑铃肩推": "手腕中立，不要耸肩",
        "侧平举": "肘部微屈，控制节奏",
        "杠铃弯举": "肘部固定，不要借力",
        "绳索下压": "大臂固定，充分收缩三头",
        "平板支撑": "身体成一条直线，核心收紧",
    }
    return tips.get(exercise_name, "保持标准姿势，控制动作节奏")


# ============================================================================
# PROGRAM GENERATION
# ============================================================================

def generate_workout_day(day_structure: Dict, profile: Dict, 
                         goal: str, week: int) -> Dict:
    """Generate a single workout day."""
    experience = profile.get("experience_level", "beginner")
    equipment = profile.get("equipment", ["none"])
    muscle_groups = day_structure.get("muscle_groups", [])
    
    # Get program parameters
    params = PROGRAM_PARAMETERS.get(goal, PROGRAM_PARAMETERS["hypertrophy"])
    
    # Determine rep range based on week
    if week <= 2:
        reps = params["reps_range"]["week1_2"]
    elif week <= 6:
        reps = params["reps_range"]["week3_6"]
    elif week <= 10:
        reps = params["reps_range"]["week7_10"]
    else:
        reps = params["reps_range"]["week11_12"]
    
    # Select exercises
    exercises = select_exercises(muscle_groups, equipment, experience, num_exercises=4)
    
    # Build exercise list with parameters
    workout_exercises = []
    for i, ex in enumerate(exercises):
        is_compound = i < 2  # First 2 exercises are compound
        category = "compound" if is_compound else "isolation"
        
        sets = params["sets_per_exercise"].get(experience, 3)
        rest = params["rest_seconds"].get(category, 60)
        
        workout_exercises.append({
            "name": ex["name"],
            "sets": sets,
            "reps": reps,
            "rest_seconds": rest,
            "category": category,
            "tips": get_exercise_tips(ex["name"])
        })
    
    return {
        "day": day_structure["day"],
        "focus": day_structure["focus"],
        "exercises": workout_exercises,
        "estimated_duration": 45 + len(workout_exercises) * 10
    }


def generate_program(profile: Dict, program_data: Dict, 
                    program_id: str = None) -> Dict[str, Any]:
    """
    Generate a complete periodized training program.
    
    Args:
        profile: User fitness profile
        program_data: Program specifications
        program_id: Optional existing program ID
    
    Returns:
        Complete program structure
    """
    goal = program_data.get("goal", "hypertrophy")
    duration = program_data.get("duration_weeks", 12)
    split_type = program_data.get("split_type", "upper_lower")
    days_per_week = program_data.get("days_per_week", 4)
    
    # Get split template
    split_template = SPLIT_TEMPLATES.get(split_type, SPLIT_TEMPLATES["upper_lower"])
    
    # Generate workout days for the split
    workout_days = []
    for day_struct in split_template["structure"][:days_per_week]:
        workout_days.append({
            "day_number": day_struct["day"],
            "focus": day_struct["focus"],
            "muscle_groups": day_struct["muscle_groups"]
        })
    
    # Generate weekly structure
    weeks = []
    for week in range(1, duration + 1):
        week_workouts = []
        for day in workout_days:
            workout = generate_workout_day(day, profile, goal, week)
            workout["week"] = week
            week_workouts.append(workout)
        
        # Determine phase
        if week <= 2:
            phase = "适应期"
        elif week <= 6:
            phase = "积累期"
        elif week <= 10:
            phase = "强化期"
        else:
            phase = "峰值/减载期"
        
        weeks.append({
            "week": week,
            "phase": phase,
            "workouts": week_workouts,
            "notes": get_week_notes(goal, week, duration)
        })
    
    program = {
        "id": program_id or str(int(datetime.now().timestamp())),
        "name": program_data.get("name", f"{duration}周{goal}计划"),
        "goal": goal,
        "duration_weeks": duration,
        "split_type": split_type,
        "split_name": split_template["name"],
        "days_per_week": days_per_week,
        "created_at": datetime.now().isoformat(),
        "profile_snapshot": {
            "experience": profile.get("experience_level"),
            "equipment": profile.get("equipment"),
            "goals": profile.get("goals")
        },
        "weeks": weeks,
        "progression_strategy": PROGRAM_PARAMETERS.get(goal, PROGRAM_PARAMETERS["hypertrophy"]).get("volume_progression"),
        "intensity_techniques": PROGRAM_PARAMETERS.get(goal, PROGRAM_PARAMETERS["hypertrophy"]).get("intensity_techniques", [])
    }
    
    return program


def get_week_notes(goal: str, week: int, total_weeks: int) -> str:
    """Get progression notes for a specific week."""
    if week == 1:
        return "适应期：学习动作，建立基础，不要追求大重量"
    elif week == 3:
        return "开始渐进超负荷：尝试每周增加重量或次数"
    elif week == 6:
        return "中期评估：回顾进度，必要时调整计划"
    elif week == total_weeks - 1:
        return "峰值周：测试极限或完成最大容量"
    elif week == total_weeks:
        return "减载周：容量减少40-50%，让身体恢复"
    elif week % 4 == 0:
        return "月度检查：评估力量进步，调整训练负荷"
    return ""


def format_workout_for_display(workout: Dict) -> str:
    """Format a workout for display."""
    lines = [
        f"训练日 {workout['day_number']} - {workout['focus']}",
        "=" * 40,
        ""
    ]
    
    for i, ex in enumerate(workout.get("exercises", []), 1):
        lines.append(f"{i}. {ex['name']}")
        lines.append(f"   {ex['sets']}组 × {ex['reps']}次")
        lines.append(f"   休息: {ex['rest_seconds']}秒")
        lines.append(f"   提示：{ex['tips']}")
        lines.append("")
    
    lines.append(f"预计时间: {workout.get('estimated_duration', 60)}分钟")
    return "\n".join(lines)


# ============================================================================
# CLI INTERFACE
# ============================================================================

if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description='Generate fitness program')
    parser.add_argument('--goal', default='hypertrophy', 
                       choices=['hypertrophy', 'strength', 'fat_loss', 'endurance'],
                       help='Training goal')
    parser.add_argument('--weeks', type=int, default=12,
                       help='Program duration in weeks')
    parser.add_argument('--split', default='upper_lower',
                       choices=['full_body', 'upper_lower', 'ppl', 'bro_split'],
                       help='Training split type')
    parser.add_argument('--days', type=int, default=4,
                       help='Days per week')
    parser.add_argument('--output', help='Output JSON file path')

    args = parser.parse_args()

    # Load profile
    profile = get_profile()
    if not profile.get("initialized"):
        print("Error: Fitness profile not initialized. Please set up profile first.")
        sys.exit(1)

    # Create program data
    program_data = {
        "name": f"{args.weeks}周{args.goal}计划",
        "goal": args.goal,
        "duration_weeks": args.weeks,
        "split_type": args.split,
        "days_per_week": args.days
    }

    # Generate program
    program = generate_program(profile, program_data)

    # Save to database
    program_id = add_program(program)
    program["id"] = program_id

    # Output
    if args.output:
        with open(args.output, 'w') as f:
            json.dump(program, f, indent=2)
        print(f"Program generated: {args.output}")
        print(f"  Program ID: {program_id}")
    else:
        print(json.dumps(program, indent=2))
