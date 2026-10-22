#!/usr/bin/env python
"""
Fitness Coach Database Manager

Manages user fitness profiles, training programs, and workout logs.
"""

import json
import os
from pathlib import Path
from datetime import datetime, timedelta
from typing import Dict, Any, List, Optional

DB_DIR = Path(os.environ.get("CODEX_DATA_DIR", Path.cwd() / "codex-data")) / "fitness_coach"
PROFILE_FILE = DB_DIR / "profile.json"
PROGRAMS_FILE = DB_DIR / "programs.json"
WORKOUTS_FILE = DB_DIR / "workouts.json"


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
            "experience_level": "",  # beginner, intermediate, advanced
            "days_per_week": 3,
            "session_duration_minutes": 60,
            "goals": [],  # hypertrophy, strength, fat_loss, endurance, mobility
            "equipment": [],  # none, dumbbells, barbell, gym, etc.
            "limitations": [],
            "avoid_exercises": []
        }
        PROFILE_FILE.write_text(json.dumps(default_profile, indent=2))

    if not PROGRAMS_FILE.exists():
        default_programs = {
            "active_programs": [],
            "completed_programs": [],
            "program_templates": []
        }
        PROGRAMS_FILE.write_text(json.dumps(default_programs, indent=2))

    if not WORKOUTS_FILE.exists():
        default_workouts = {
            "workouts": []
        }
        WORKOUTS_FILE.write_text(json.dumps(default_workouts, indent=2))


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
    """Check if fitness profile is initialized."""
    profile = load_json(PROFILE_FILE)
    return profile.get("initialized", False)


def get_profile() -> Dict[str, Any]:
    """Get user fitness profile."""
    return load_json(PROFILE_FILE)


def save_profile(profile: Dict[str, Any]) -> None:
    """Save user fitness profile."""
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


def add_goal(goal: str) -> None:
    """Add a fitness goal."""
    profile = load_json(PROFILE_FILE)
    if "goals" not in profile:
        profile["goals"] = []
    if goal not in profile["goals"]:
        profile["goals"].append(goal)
    save_json(PROFILE_FILE, profile)


def add_equipment(equipment: str) -> None:
    """Add available equipment."""
    profile = load_json(PROFILE_FILE)
    if "equipment" not in profile:
        profile["equipment"] = []
    if equipment not in profile["equipment"]:
        profile["equipment"].append(equipment)
    save_json(PROFILE_FILE, profile)


def add_limitation(limitation: str) -> None:
    """Add a physical limitation."""
    profile = load_json(PROFILE_FILE)
    if "limitations" not in profile:
        profile["limitations"] = []
    if limitation not in profile["limitations"]:
        profile["limitations"].append(limitation)
    save_json(PROFILE_FILE, profile)


# ============================================================================
# PROGRAM MANAGEMENT
# ============================================================================

def get_programs(status: str = "all") -> Dict[str, List[Dict]]:
    """
    Get programs by status.
    
    Args:
        status: "active", "completed", "templates", or "all"
    """
    programs = load_json(PROGRAMS_FILE)
    
    if status == "all":
        return programs
    elif status == "active":
        return {"active_programs": programs.get("active_programs", [])}
    elif status == "completed":
        return {"completed_programs": programs.get("completed_programs", [])}
    elif status == "templates":
        return {"program_templates": programs.get("program_templates", [])}
    return {}


def add_program(program: Dict[str, Any], status: str = "active") -> str:
    """
    Add a new training program.
    
    Args:
        program: Program data
        status: "active", "completed", or "template"
    
    Returns:
        Program ID
    """
    programs = load_json(PROGRAMS_FILE)
    
    program_id = str(int(datetime.now().timestamp()))
    program["id"] = program_id
    program["created_at"] = datetime.now().isoformat()
    program["status"] = status
    
    if status == "active":
        # Deactivate other active programs
        for p in programs.get("active_programs", []):
            p["status"] = "paused"
        programs["active_programs"].append(program)
    elif status == "completed":
        programs["completed_programs"].append(program)
    elif status == "template":
        programs["program_templates"].append(program)
    
    save_json(PROGRAMS_FILE, programs)
    return program_id


def get_program_by_id(program_id: str) -> Optional[Dict[str, Any]]:
    """Get a specific program by ID."""
    programs = load_json(PROGRAMS_FILE)
    
    for key in ["active_programs", "completed_programs", "program_templates"]:
        for program in programs.get(key, []):
            if program.get("id") == program_id:
                return program
    return None


def update_program(program_id: str, updates: Dict[str, Any]) -> bool:
    """Update a program."""
    programs = load_json(PROGRAMS_FILE)
    
    for key in ["active_programs", "completed_programs", "program_templates"]:
        for program in programs.get(key, []):
            if program.get("id") == program_id:
                program.update(updates)
                program["updated_at"] = datetime.now().isoformat()
                save_json(PROGRAMS_FILE, programs)
                return True
    return False


def complete_program(program_id: str) -> bool:
    """Mark an active program as completed."""
    programs = load_json(PROGRAMS_FILE)
    
    for i, program in enumerate(programs.get("active_programs", [])):
        if program.get("id") == program_id:
            program["completed_at"] = datetime.now().isoformat()
            program["status"] = "completed"
            programs["active_programs"].pop(i)
            programs["completed_programs"].append(program)
            save_json(PROGRAMS_FILE, programs)
            return True
    return False


def delete_program(program_id: str) -> bool:
    """Delete a program."""
    programs = load_json(PROGRAMS_FILE)
    
    for key in ["active_programs", "completed_programs", "program_templates"]:
        for i, program in enumerate(programs.get(key, [])):
            if program.get("id") == program_id:
                programs[key].pop(i)
                save_json(PROGRAMS_FILE, programs)
                return True
    return False


def substitute_exercise(program_id: str, old_exercise: str, new_exercise: str, 
                        reason: str = "") -> bool:
    """Substitute an exercise in a program."""
    program = get_program_by_id(program_id)
    if not program:
        return False
    
    # Update in workout days
    for day in program.get("workout_days", []):
        for exercise in day.get("exercises", []):
            if exercise.get("name") == old_exercise:
                exercise["name"] = new_exercise
                exercise["substituted_from"] = old_exercise
                exercise["substitution_reason"] = reason
                exercise["substituted_at"] = datetime.now().isoformat()
    
    return update_program(program_id, {"workout_days": program.get("workout_days", [])})


# ============================================================================
# WORKOUT LOGGING
# ============================================================================

def log_workout(workout: Dict[str, Any]) -> str:
    """
    Log a completed workout.
    
    Returns:
        Workout log ID
    """
    workouts = load_json(WORKOUTS_FILE)
    
    log_id = str(int(datetime.now().timestamp()))
    workout["log_id"] = log_id
    workout["logged_at"] = datetime.now().isoformat()
    
    if "workouts" not in workouts:
        workouts["workouts"] = []
    
    workouts["workouts"].append(workout)
    save_json(WORKOUTS_FILE, workouts)
    return log_id


def get_workouts(program_id: str = None, limit: int = None) -> List[Dict[str, Any]]:
    """Get workout logs, optionally filtered by program."""
    workouts = load_json(WORKOUTS_FILE)
    all_workouts = workouts.get("workouts", [])
    
    if program_id:
        all_workouts = [w for w in all_workouts if w.get("program_id") == program_id]
    
    # Sort by date, newest first
    all_workouts.sort(key=lambda x: x.get("logged_at", ""), reverse=True)
    
    if limit:
        return all_workouts[:limit]
    return all_workouts


def get_workout_by_id(log_id: str) -> Optional[Dict[str, Any]]:
    """Get a specific workout log."""
    workouts = load_json(WORKOUTS_FILE)
    for workout in workouts.get("workouts", []):
        if workout.get("log_id") == log_id:
            return workout
    return None


def get_last_workout(program_id: str = None) -> Optional[Dict[str, Any]]:
    """Get the most recent workout."""
    workouts = get_workouts(program_id, limit=1)
    return workouts[0] if workouts else None


# ============================================================================
# PROGRESS TRACKING
# ============================================================================

def get_progress_summary(program_id: str = None) -> Dict[str, Any]:
    """Get progress summary."""
    workouts = get_workouts(program_id)
    profile = get_profile()
    
    if not workouts:
        return {
            "total_workouts": 0,
            "total_volume": 0,
            "average_workout_duration": 0,
            "consistency_score": 0
        }
    
    total_volume = 0
    total_duration = 0
    exercise_progress = {}
    
    for workout in workouts:
        total_duration += workout.get("duration_minutes", 0)
        
        for exercise in workout.get("exercises", []):
            name = exercise.get("name", "Unknown")
            sets = exercise.get("sets", [])
            
            # Calculate volume for this exercise
            exercise_volume = sum(s.get("weight", 0) * s.get("reps", 0) for s in sets)
            total_volume += exercise_volume
            
            # Track progress per exercise
            if name not in exercise_progress:
                exercise_progress[name] = []
            
            if sets:
                max_weight = max(s.get("weight", 0) for s in sets)
                exercise_progress[name].append({
                    "date": workout.get("logged_at"),
                    "max_weight": max_weight,
                    "total_volume": exercise_volume,
                    "sets": len(sets)
                })
    
    # Calculate consistency (workouts per week)
    if len(workouts) >= 2:
        first_date = datetime.fromisoformat(workouts[-1].get("logged_at", datetime.now().isoformat()))
        last_date = datetime.fromisoformat(workouts[0].get("logged_at", datetime.now().isoformat()))
        weeks = max(1, (last_date - first_date).days / 7)
        consistency = len(workouts) / weeks
    else:
        consistency = len(workouts)
    
    return {
        "total_workouts": len(workouts),
        "total_volume": total_volume,
        "average_workout_duration": total_duration / len(workouts) if workouts else 0,
        "consistency_score": round(consistency, 2),
        "exercise_progress": exercise_progress,
        "recent_workouts": workouts[:5]
    }


def get_exercise_history(exercise_name: str, program_id: str = None) -> List[Dict[str, Any]]:
    """Get history for a specific exercise."""
    workouts = get_workouts(program_id)
    history = []
    
    for workout in workouts:
        for exercise in workout.get("exercises", []):
            if exercise.get("name") == exercise_name:
                sets = exercise.get("sets", [])
                if sets:
                    history.append({
                        "date": workout.get("logged_at"),
                        "max_weight": max(s.get("weight", 0) for s in sets),
                        "max_reps": max(s.get("reps", 0) for s in sets),
                        "total_sets": len(sets),
                        "volume": sum(s.get("weight", 0) * s.get("reps", 0) for s in sets)
                    })
    
    return sorted(history, key=lambda x: x["date"])


def get_personal_records() -> Dict[str, Dict[str, Any]]:
    """Get personal records for each exercise."""
    workouts = get_workouts()
    records = {}
    
    for workout in workouts:
        for exercise in workout.get("exercises", []):
            name = exercise.get("name", "Unknown")
            sets = exercise.get("sets", [])
            
            if not sets:
                continue
            
            max_weight_set = max(sets, key=lambda s: s.get("weight", 0))
            
            if name not in records or max_weight_set.get("weight", 0) > records[name].get("weight", 0):
                records[name] = {
                    "weight": max_weight_set.get("weight", 0),
                    "reps": max_weight_set.get("reps", 0),
                    "date": workout.get("logged_at"),
                    "estimated_1rm": calculate_1rm(max_weight_set.get("weight", 0), max_weight_set.get("reps", 0))
                }
    
    return records


def calculate_1rm(weight: float, reps: int) -> float:
    """Calculate estimated 1-rep max using Epley formula."""
    if reps == 1:
        return weight
    if reps == 0 or weight == 0:
        return 0
    return weight * (1 + reps / 30)


# ============================================================================
# STATISTICS & INSIGHTS
# ============================================================================

def get_fitness_stats() -> Dict[str, Any]:
    """Get overall fitness statistics."""
    profile = get_profile()
    programs = load_json(PROGRAMS_FILE)
    workouts = get_workouts()
    
    active_programs = [p for p in programs.get("active_programs", [])]
    completed_programs = programs.get("completed_programs", [])
    
    # Calculate streak
    streak = 0
    if workouts:
        dates = sorted(set([
            datetime.fromisoformat(w.get("logged_at", "")).date()
            for w in workouts
        ]), reverse=True)
        
        if dates:
            today = datetime.now().date()
            check_date = today
            
            for date in dates:
                if date == check_date or date == check_date - timedelta(days=1):
                    if date == today or date == today - timedelta(days=1):
                        streak += 1
                    check_date = date
                else:
                    break
    
    return {
        "profile_initialized": profile.get("initialized", False),
        "experience_level": profile.get("experience_level", ""),
        "goals": profile.get("goals", []),
        "active_programs": len(active_programs),
        "completed_programs": len(completed_programs),
        "total_workouts_logged": len(workouts),
        "current_streak": streak,
        "personal_records_count": len(get_personal_records())
    }


# ============================================================================
# UTILITY FUNCTIONS
# ============================================================================

def export_all() -> Dict[str, Any]:
    """Export all fitness data."""
    return {
        "profile": get_profile(),
        "programs": get_programs("all"),
        "workouts": get_workouts(),
        "stats": get_fitness_stats(),
        "personal_records": get_personal_records(),
        "exported_at": datetime.now().isoformat()
    }


def reset_all() -> None:
    """Reset all data (use with caution)."""
    for file_path in [PROFILE_FILE, PROGRAMS_FILE, WORKOUTS_FILE]:
        if file_path.exists():
            file_path.unlink()
    ensure_db_files()


# ============================================================================
# CLI INTERFACE
# ============================================================================

if __name__ == "__main__":
    import sys

    if len(sys.argv) < 2:
        print("Fitness Coach Database Manager")
        print("\nUsage:")
        print("  python fitness_db.py is_initialized")
        print("  python fitness_db.py get_profile")
        print("  python fitness_db.py get_programs [active|completed|templates|all]")
        print("  python fitness_db.py progress [program_id]")
        print("  python fitness_db.py stats")
        print("  python fitness_db.py prs")
        print("  python fitness_db.py export")
        print("  python fitness_db.py reset")
        sys.exit(1)

    command = sys.argv[1]

    if command == "is_initialized":
        print("true" if is_initialized() else "false")
    elif command == "get_profile":
        print(json.dumps(get_profile(), indent=2))
    elif command == "get_programs":
        status = sys.argv[2] if len(sys.argv) > 2 else "all"
        print(json.dumps(get_programs(status), indent=2))
    elif command == "progress":
        program_id = sys.argv[2] if len(sys.argv) > 2 else None
        print(json.dumps(get_progress_summary(program_id), indent=2))
    elif command == "stats":
        print(json.dumps(get_fitness_stats(), indent=2))
    elif command == "prs":
        print(json.dumps(get_personal_records(), indent=2))
    elif command == "export":
        print(json.dumps(export_all(), indent=2))
    elif command == "reset":
        confirm = input("Are you sure you want to reset all fitness data? (yes/no): ")
        if confirm.lower() == "yes":
            reset_all()
            print("All fitness data has been reset.")
        else:
            print("Reset cancelled.")
    else:
        print(f"Unknown command: {command}")
        sys.exit(1)
