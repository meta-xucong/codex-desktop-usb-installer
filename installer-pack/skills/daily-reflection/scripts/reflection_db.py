#!/usr/bin/env python
"""
Daily Reflection Database Manager

Manages user reflection profiles, daily reflections, habit tracking, and goals.
"""

import json
import os
import re
from pathlib import Path
from datetime import datetime, timedelta
from typing import Dict, Any, List, Optional, Tuple
from collections import Counter, defaultdict

DB_DIR = Path(os.environ.get("CODEX_DATA_DIR", Path.cwd() / "codex-data")) / "daily_reflection"
PROFILE_FILE = DB_DIR / "profile.json"
REFLECTIONS_FILE = DB_DIR / "reflections.json"
HABITS_FILE = DB_DIR / "habits.json"
GOALS_FILE = DB_DIR / "goals.json"
READING_FILE = DB_DIR / "reading.json"
WELLNESS_FILE = DB_DIR / "wellness.json"


def ensure_db_files() -> None:
    """Ensure all database files exist."""
    DB_DIR.mkdir(parents=True, exist_ok=True)

    if not PROFILE_FILE.exists():
        default_profile = {
            "initialized": False,
            "created_at": datetime.now().isoformat(),
            "preferred_time": "21:00",
            "reflection_style": "structured",  # structured/free
            "reminder_style": "gentle",  # none/gentle/strict
            "timezone": "Asia/Shanghai"
        }
        PROFILE_FILE.write_text(json.dumps(default_profile, indent=2))

    if not REFLECTIONS_FILE.exists():
        default_reflections = {
            "reflections": []
        }
        REFLECTIONS_FILE.write_text(json.dumps(default_reflections, indent=2))

    if not HABITS_FILE.exists():
        default_habits = {
            "habits": [],
            "habit_logs": []
        }
        HABITS_FILE.write_text(json.dumps(default_habits, indent=2))

    if not GOALS_FILE.exists():
        default_goals = {
            "goals": []
        }
        GOALS_FILE.write_text(json.dumps(default_goals, indent=2))

    if not READING_FILE.exists():
        default_reading = {
            "reading_plans": [],
            "reading_logs": []
        }
        READING_FILE.write_text(json.dumps(default_reading, indent=2))

    if not WELLNESS_FILE.exists():
        default_wellness = {
            "wellness_entries": []
        }
        WELLNESS_FILE.write_text(json.dumps(default_wellness, indent=2))


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
    """Check if reflection profile is initialized."""
    profile = load_json(PROFILE_FILE)
    return profile.get("initialized", False)


def get_profile() -> Dict[str, Any]:
    """Get user reflection profile."""
    return load_json(PROFILE_FILE)


def save_profile(profile: Dict[str, Any]) -> None:
    """Save user reflection profile."""
    data = load_json(PROFILE_FILE)
    data.update(profile)
    data["initialized"] = True
    data["last_updated"] = datetime.now().isoformat()
    save_json(PROFILE_FILE, data)


# ============================================================================
# REFLECTION LOGGING
# ============================================================================

def log_reflection(reflection: Dict[str, Any]) -> str:
    """Log a daily reflection."""
    reflections = load_json(REFLECTIONS_FILE)
    
    reflection_id = str(int(datetime.now().timestamp()))
    reflection["reflection_id"] = reflection_id
    reflection["logged_at"] = datetime.now().isoformat()
    
    if "reflections" not in reflections:
        reflections["reflections"] = []
    
    # Check if reflection for this date already exists
    date = reflection.get("date")
    existing_idx = None
    for i, r in enumerate(reflections["reflections"]):
        if r.get("date") == date:
            existing_idx = i
            break
    
    if existing_idx is not None:
        # Update existing
        reflections["reflections"][existing_idx] = reflection
    else:
        # Add new
        reflections["reflections"].append(reflection)
    
    save_json(REFLECTIONS_FILE, reflections)
    
    # Also update habit logs if habits are included
    if "habits" in reflection:
        for habit in reflection["habits"]:
            log_habit(
                habit_id=habit.get("habit_id"),
                date=date,
                completed=habit.get("completed", False),
                note=habit.get("note", "")
            )
    
    return reflection_id


def get_reflections(start_date: str = None, end_date: str = None, 
                   limit: int = None) -> List[Dict[str, Any]]:
    """Get reflections, optionally filtered by date range."""
    reflections = load_json(REFLECTIONS_FILE)
    all_reflections = reflections.get("reflections", [])
    
    if start_date:
        all_reflections = [r for r in all_reflections if r.get("date", "") >= start_date]
    if end_date:
        all_reflections = [r for r in all_reflections if r.get("date", "") <= end_date]
    
    # Sort by date, newest first
    all_reflections.sort(key=lambda x: x.get("date", ""), reverse=True)
    
    if limit:
        return all_reflections[:limit]
    return all_reflections


def get_reflection_by_date(date: str) -> Optional[Dict[str, Any]]:
    """Get reflection for a specific date."""
    reflections = get_reflections()
    for r in reflections:
        if r.get("date") == date:
            return r
    return None


def get_today_reflection() -> Optional[Dict[str, Any]]:
    """Get today's reflection."""
    today = datetime.now().strftime("%Y-%m-%d")
    return get_reflection_by_date(today)


# ============================================================================
# HABIT TRACKING
# ============================================================================

def add_habit(habit: Dict[str, Any]) -> str:
    """Add a new habit to track."""
    habits = load_json(HABITS_FILE)
    
    habit_id = f"h{int(datetime.now().timestamp())}"
    habit["habit_id"] = habit_id
    habit["created_at"] = datetime.now().isoformat()
    
    if "habits" not in habits:
        habits["habits"] = []
    
    habits["habits"].append(habit)
    save_json(HABITS_FILE, habits)
    return habit_id


def get_habits() -> List[Dict[str, Any]]:
    """Get all habits."""
    habits = load_json(HABITS_FILE)
    return habits.get("habits", [])


def log_habit(habit_id: str, date: str, completed: bool, note: str = "") -> str:
    """Log a habit completion."""
    habits = load_json(HABITS_FILE)
    
    log_entry = {
        "log_id": str(int(datetime.now().timestamp())),
        "habit_id": habit_id,
        "date": date,
        "completed": completed,
        "note": note,
        "logged_at": datetime.now().isoformat()
    }
    
    if "habit_logs" not in habits:
        habits["habit_logs"] = []
    
    # Check if log for this habit/date already exists
    existing_idx = None
    for i, log in enumerate(habits["habit_logs"]):
        if log.get("habit_id") == habit_id and log.get("date") == date:
            existing_idx = i
            break
    
    if existing_idx is not None:
        habits["habit_logs"][existing_idx] = log_entry
    else:
        habits["habit_logs"].append(log_entry)
    
    save_json(HABITS_FILE, habits)
    return log_entry["log_id"]


def get_habit_logs(habit_id: str = None, days: int = 30) -> List[Dict[str, Any]]:
    """Get habit logs, optionally filtered by habit and date range."""
    habits = load_json(HABITS_FILE)
    all_logs = habits.get("habit_logs", [])
    
    if habit_id:
        all_logs = [l for l in all_logs if l.get("habit_id") == habit_id]
    
    # Filter by date range
    cutoff_date = (datetime.now() - timedelta(days=days)).strftime("%Y-%m-%d")
    all_logs = [l for l in all_logs if l.get("date", "") >= cutoff_date]
    
    # Sort by date
    all_logs.sort(key=lambda x: x.get("date", ""))
    return all_logs


def get_habit_stats(habit_id: str, days: int = 30) -> Dict[str, Any]:
    """Get statistics for a specific habit."""
    logs = get_habit_logs(habit_id, days)
    habit = None
    
    for h in get_habits():
        if h.get("habit_id") == habit_id:
            habit = h
            break
    
    if not logs:
        return {
            "habit_id": habit_id,
            "habit_name": habit.get("name", "Unknown") if habit else "Unknown",
            "period_days": days,
            "completion_rate": 0,
            "current_streak": 0,
            "longest_streak": 0,
            "total_completions": 0
        }
    
    # Calculate completion rate
    completed_count = sum(1 for l in logs if l.get("completed"))
    completion_rate = completed_count / len(logs) * 100 if logs else 0
    
    # Calculate streaks
    dates = sorted([l.get("date") for l in logs if l.get("completed")], reverse=True)
    current_streak = 0
    longest_streak = 0
    
    if dates:
        # Current streak
        today = datetime.now().strftime("%Y-%m-%d")
        yesterday = (datetime.now() - timedelta(days=1)).strftime("%Y-%m-%d")
        
        if dates[0] in [today, yesterday]:
            current_streak = 1
            for i in range(1, len(dates)):
                expected_date = (datetime.strptime(dates[i-1], "%Y-%m-%d") - timedelta(days=1)).strftime("%Y-%m-%d")
                if dates[i] == expected_date:
                    current_streak += 1
                else:
                    break
        
        # Longest streak
        temp_streak = 1
        for i in range(1, len(dates)):
            prev_date = datetime.strptime(dates[i-1], "%Y-%m-%d")
            curr_date = datetime.strptime(dates[i], "%Y-%m-%d")
            if (prev_date - curr_date).days == 1:
                temp_streak += 1
            else:
                longest_streak = max(longest_streak, temp_streak)
                temp_streak = 1
        longest_streak = max(longest_streak, temp_streak)
    
    return {
        "habit_id": habit_id,
        "habit_name": habit.get("name", "Unknown") if habit else "Unknown",
        "period_days": days,
        "completion_rate": round(completion_rate, 1),
        "current_streak": current_streak,
        "longest_streak": longest_streak,
        "total_completions": completed_count,
        "recent_logs": logs[-7:]
    }


def get_all_habits_status(days: int = 7) -> List[Dict[str, Any]]:
    """Get status for all habits."""
    habits = get_habits()
    result = []
    
    for habit in habits:
        habit_id = habit.get("habit_id")
        stats = get_habit_stats(habit_id, days)
        result.append({**habit, **stats})
    
    return result


# ============================================================================
# GOAL MANAGEMENT
# ============================================================================

def add_goal(goal: Dict[str, Any]) -> str:
    """Add a new goal."""
    goals = load_json(GOALS_FILE)
    
    goal_id = f"g{int(datetime.now().timestamp())}"
    goal["goal_id"] = goal_id
    goal["created_at"] = datetime.now().isoformat()
    goal["status"] = goal.get("status", "active")
    goal["progress_percentage"] = goal.get("progress_percentage", 0)
    
    if "goals" not in goals:
        goals["goals"] = []
    
    goals["goals"].append(goal)
    save_json(GOALS_FILE, goals)
    return goal_id


def get_goals(status: str = "all") -> List[Dict[str, Any]]:
    """Get goals, optionally filtered by status."""
    goals = load_json(GOALS_FILE)
    all_goals = goals.get("goals", [])
    
    if status != "all":
        all_goals = [g for g in all_goals if g.get("status") == status]
    
    return all_goals


def update_goal_progress(goal_id: str, progress_text: str, 
                        percentage: int = None) -> bool:
    """Update goal progress."""
    goals = load_json(GOALS_FILE)
    
    for goal in goals.get("goals", []):
        if goal.get("goal_id") == goal_id:
            goal["progress_text"] = progress_text
            if percentage is not None:
                goal["progress_percentage"] = min(100, max(0, percentage))
            goal["last_updated"] = datetime.now().isoformat()
            save_json(GOALS_FILE, goals)
            return True
    return False


def complete_goal(goal_id: str) -> bool:
    """Mark a goal as completed."""
    goals = load_json(GOALS_FILE)
    
    for goal in goals.get("goals", []):
        if goal.get("goal_id") == goal_id:
            goal["status"] = "completed"
            goal["progress_percentage"] = 100
            goal["completed_at"] = datetime.now().isoformat()
            save_json(GOALS_FILE, goals)
            return True
    return False


# ============================================================================
# WEEKLY & MONTHLY REVIEWS
# ============================================================================

def generate_weekly_review(week_start: str = None) -> Dict[str, Any]:
    """Generate a weekly review."""
    if week_start is None:
        # Find the most recent Monday
        today = datetime.now()
        monday = today - timedelta(days=today.weekday())
        week_start = monday.strftime("%Y-%m-%d")
    
    week_end = (datetime.strptime(week_start, "%Y-%m-%d") + timedelta(days=6)).strftime("%Y-%m-%d")
    
    # Get reflections for the week
    reflections = get_reflections(week_start, week_end)
    
    # Calculate stats
    total_days = 7
    reflection_days = len(reflections)
    
    moods = [r.get("mood") for r in reflections if r.get("mood")]
    mood_counts = defaultdict(int)
    for mood in moods:
        mood_counts[mood] += 1
    dominant_mood = max(mood_counts, key=mood_counts.get) if mood_counts else "neutral"
    
    energy_levels = [r.get("energy_level") for r in reflections if r.get("energy_level")]
    avg_energy = sum(energy_levels) / len(energy_levels) if energy_levels else 0
    
    # Get habit stats for the week
    habits_status = get_all_habits_status(days=7)
    
    # Get goal progress updates
    goals = get_goals("active")
    
    return {
        "week_start": week_start,
        "week_end": week_end,
        "reflection_completion": {
            "completed": reflection_days,
            "total": total_days,
            "percentage": round(reflection_days / total_days * 100, 1)
        },
        "mood_summary": {
            "dominant": dominant_mood,
            "distribution": dict(mood_counts)
        },
        "energy_average": round(avg_energy, 1),
        "habits_summary": habits_status,
        "goals_status": goals,
        "daily_highlights": [
            {
                "date": r.get("date"),
                "wins": r.get("wins", [])[:2],
                "mood": r.get("mood")
            }
            for r in reflections[:7]
        ]
    }


def generate_monthly_review(year: int = None, month: int = None) -> Dict[str, Any]:
    """Generate a monthly review."""
    if year is None:
        year = datetime.now().year
    if month is None:
        month = datetime.now().month
    
    # Calculate date range
    start_date = f"{year}-{month:02d}-01"
    if month == 12:
        end_date = f"{year + 1}-01-01"
    else:
        end_date = f"{year}-{month + 1:02d}-01"
    
    # Get reflections
    reflections = get_reflections(start_date, end_date)
    
    # Calculate days in month
    if month == 12:
        next_month = datetime(year + 1, 1, 1)
    else:
        next_month = datetime(year, month + 1, 1)
    days_in_month = (next_month - datetime(year, month, 1)).days
    
    return {
        "year": year,
        "month": month,
        "reflection_stats": {
            "completed": len(reflections),
            "total_days": days_in_month,
            "percentage": round(len(reflections) / days_in_month * 100, 1)
        },
        "habits_stats": get_all_habits_status(days=days_in_month),
        "goals_progress": get_goals("active"),
        "monthly_themes": extract_themes(reflections)
    }


def extract_themes(reflections: List[Dict]) -> List[str]:
    """Extract up to five recurring, human-entered phrases deterministically."""
    phrases: List[str] = []
    for reflection in reflections:
        for field in ("wins", "learnings", "gratitude"):
            values = reflection.get(field, [])
            if isinstance(values, str):
                values = [values]
            if isinstance(values, list):
                phrases.extend(str(value) for value in values)
        free_notes = reflection.get("free_notes", "")
        if isinstance(free_notes, str):
            phrases.extend(re.split(r"[。！？!?；;\n]+", free_notes))

    normalized = []
    for phrase in phrases:
        value = re.sub(r"\s+", " ", phrase).strip(" \t\r\n，,。.；;：:!?！？-_*")
        if len(value) >= 2:
            normalized.append(value[:80])
    counts = Counter(normalized)
    return [phrase for phrase, _ in sorted(counts.items(), key=lambda item: (-item[1], item[0]))[:5]]


# ============================================================================
# STATISTICS & INSIGHTS
# ============================================================================

def get_reflection_stats(days: int = 30) -> Dict[str, Any]:
    """Get reflection statistics."""
    cutoff_date = (datetime.now() - timedelta(days=days)).strftime("%Y-%m-%d")
    reflections = get_reflections(start_date=cutoff_date)
    
    # Mood distribution
    moods = [r.get("mood") for r in reflections if r.get("mood")]
    mood_dist = defaultdict(int)
    for mood in moods:
        mood_dist[mood] += 1
    
    # Energy trend
    energy_by_week = defaultdict(list)
    for r in reflections:
        if r.get("energy_level"):
            week = datetime.strptime(r.get("date"), "%Y-%m-%d").isocalendar()[1]
            energy_by_week[week].append(r.get("energy_level"))
    
    energy_trend = {
        week: round(sum(levels) / len(levels), 1)
        for week, levels in energy_by_week.items()
    }
    
    return {
        "period_days": days,
        "total_reflections": len(reflections),
        "mood_distribution": dict(mood_dist),
        "energy_trend": energy_trend,
        "average_energy": round(sum(r.get("energy_level", 0) for r in reflections) / len(reflections), 1) if reflections else 0
    }


# ============================================================================
# UTILITY FUNCTIONS
# ============================================================================

def export_all() -> Dict[str, Any]:
    """Export all reflection data."""
    return {
        "profile": get_profile(),
        "reflections": get_reflections(),
        "habits": get_habits(),
        "habit_logs": get_habit_logs(days=365),
        "goals": get_goals("all"),
        "stats": get_reflection_stats(30),
        "exported_at": datetime.now().isoformat()
    }


def reset_all() -> None:
    """Reset all data (use with caution)."""
    for file_path in [PROFILE_FILE, REFLECTIONS_FILE, HABITS_FILE, GOALS_FILE, READING_FILE, WELLNESS_FILE]:
        if file_path.exists():
            file_path.unlink()
    ensure_db_files()


# ============================================================================
# READING PLAN MANAGEMENT
# ============================================================================

def add_reading_plan(plan: Dict[str, Any]) -> str:
    """Add a new reading plan."""
    reading = load_json(READING_FILE)
    
    plan_id = f"rp{int(datetime.now().timestamp())}"
    plan["plan_id"] = plan_id
    plan["created_at"] = datetime.now().isoformat()
    
    if "reading_plans" not in reading:
        reading["reading_plans"] = []
    
    reading["reading_plans"].append(plan)
    save_json(READING_FILE, reading)
    return plan_id


def get_reading_plans() -> List[Dict[str, Any]]:
    """Get all reading plans."""
    reading = load_json(READING_FILE)
    return reading.get("reading_plans", [])


def log_reading_progress(progress: Dict[str, Any]) -> str:
    """Log reading progress for a book."""
    reading = load_json(READING_FILE)
    
    log_id = str(int(datetime.now().timestamp()))
    progress["log_id"] = log_id
    progress["logged_at"] = datetime.now().isoformat()
    
    if "reading_logs" not in reading:
        reading["reading_logs"] = []
    
    reading["reading_logs"].append(progress)
    save_json(READING_FILE, reading)
    return log_id


def get_reading_stats(days: int = 30) -> Dict[str, Any]:
    """Get reading statistics."""
    reading = load_json(READING_FILE)
    logs = reading.get("reading_logs", [])
    plans = reading.get("reading_plans", [])
    
    # Filter logs by date
    cutoff_date = (datetime.now() - timedelta(days=days)).strftime("%Y-%m-%d")
    recent_logs = [l for l in logs if l.get("date", "") >= cutoff_date]
    
    # Calculate stats
    total_pages = sum(l.get("pages_read", 0) for l in recent_logs)
    reading_days = len(set(l.get("date") for l in recent_logs))
    
    # Calculate streak
    dates = sorted(set(l.get("date") for l in logs), reverse=True)
    streak = 0
    if dates:
        today = datetime.now().strftime("%Y-%m-%d")
        yesterday = (datetime.now() - timedelta(days=1)).strftime("%Y-%m-%d")
        if dates[0] in [today, yesterday]:
            streak = 1
            for i in range(1, len(dates)):
                expected = (datetime.strptime(dates[i-1], "%Y-%m-%d") - timedelta(days=1)).strftime("%Y-%m-%d")
                if dates[i] == expected:
                    streak += 1
                else:
                    break
    
    # Count completed books
    completed_books = sum(1 for p in plans for b in p.get("books", []) if b.get("status") == "completed")
    
    return {
        "period_days": days,
        "total_pages": total_pages,
        "reading_days": reading_days,
        "average_pages_per_day": round(total_pages / days, 1),
        "current_streak": streak,
        "completed_books": completed_books,
        "active_plans": len(plans)
    }


# ============================================================================
# MENTAL WELLNESS MANAGEMENT
# ============================================================================

def log_mental_wellness(entry: Dict[str, Any]) -> str:
    """Log a mental wellness entry."""
    wellness = load_json(WELLNESS_FILE)
    
    entry_id = str(int(datetime.now().timestamp()))
    entry["entry_id"] = entry_id
    entry["logged_at"] = datetime.now().isoformat()
    
    if "wellness_entries" not in wellness:
        wellness["wellness_entries"] = []
    
    wellness["wellness_entries"].append(entry)
    save_json(WELLNESS_FILE, wellness)
    return entry_id


def get_wellness_entries(days: int = 30) -> List[Dict[str, Any]]:
    """Get recent wellness entries."""
    wellness = load_json(WELLNESS_FILE)
    entries = wellness.get("wellness_entries", [])
    
    cutoff_date = (datetime.now() - timedelta(days=days)).strftime("%Y-%m-%d")
    recent_entries = [e for e in entries if e.get("date", "") >= cutoff_date]
    
    return sorted(recent_entries, key=lambda x: x.get("date", ""), reverse=True)


def get_wellness_stats(days: int = 30) -> Dict[str, Any]:
    """Get mental wellness statistics."""
    entries = get_wellness_entries(days)
    
    if not entries:
        return {"period_days": days, "entries_count": 0}
    
    # Mood distribution
    moods = [e.get("mood") for e in entries if e.get("mood")]
    mood_counts = defaultdict(int)
    for mood in moods:
        mood_counts[mood] += 1
    
    # Coping strategies effectiveness
    effectiveness = [e.get("effectiveness", 0) for e in entries if e.get("effectiveness")]
    avg_effectiveness = sum(effectiveness) / len(effectiveness) if effectiveness else 0
    
    # Common triggers
    all_triggers = []
    for e in entries:
        all_triggers.extend(e.get("triggers", []))
    trigger_counts = defaultdict(int)
    for trigger in all_triggers:
        trigger_counts[trigger] += 1
    
    return {
        "period_days": days,
        "entries_count": len(entries),
        "mood_distribution": dict(mood_counts),
        "avg_coping_effectiveness": round(avg_effectiveness, 1),
        "common_triggers": dict(sorted(trigger_counts.items(), key=lambda x: x[1], reverse=True)[:5]),
        "recent_entries": entries[:5]
    }


# ============================================================================
# CLI INTERFACE
# ============================================================================

if __name__ == "__main__":
    import sys

    if len(sys.argv) < 2:
        print("Daily Reflection Database Manager")
        print("\nUsage:")
        print("  python reflection_db.py is_initialized")
        print("  python reflection_db.py today")
        print("  python reflection_db.py weekly [YYYY-MM-DD]")
        print("  python reflection_db.py monthly [YYYY-MM]")
        print("  python reflection_db.py habits")
        print("  python reflection_db.py goals")
        print("  python reflection_db.py reading_stats [days]")
        print("  python reflection_db.py wellness_stats [days]")
        print("  python reflection_db.py stats [days]")
        print("  python reflection_db.py export")
        print("  python reflection_db.py reset")
        sys.exit(1)

    command = sys.argv[1]

    if command == "is_initialized":
        print("true" if is_initialized() else "false")
    elif command == "today":
        reflection = get_today_reflection()
        if reflection:
            print(json.dumps(reflection, indent=2))
        else:
            print("No reflection logged for today yet.")
    elif command == "weekly":
        week_start = sys.argv[2] if len(sys.argv) > 2 else None
        print(json.dumps(generate_weekly_review(week_start), indent=2))
    elif command == "monthly":
        if len(sys.argv) > 2:
            year, month = map(int, sys.argv[2].split("-"))
            print(json.dumps(generate_monthly_review(year, month), indent=2))
        else:
            print(json.dumps(generate_monthly_review(), indent=2))
    elif command == "habits":
        print(json.dumps(get_all_habits_status(days=7), indent=2))
    elif command == "goals":
        print(json.dumps(get_goals("active"), indent=2))
    elif command == "reading_stats":
        days = int(sys.argv[2]) if len(sys.argv) > 2 else 30
        print(json.dumps(get_reading_stats(days), indent=2))
    elif command == "wellness_stats":
        days = int(sys.argv[2]) if len(sys.argv) > 2 else 30
        print(json.dumps(get_wellness_stats(days), indent=2))
    elif command == "stats":
        days = int(sys.argv[2]) if len(sys.argv) > 2 else 30
        print(json.dumps(get_reflection_stats(days), indent=2))
    elif command == "export":
        print(json.dumps(export_all(), indent=2))
    elif command == "reset":
        confirm = input("Are you sure you want to reset all reflection data? (yes/no): ")
        if confirm.lower() == "yes":
            reset_all()
            print("All reflection data has been reset.")
        else:
            print("Reset cancelled.")
    else:
        print(f"Unknown command: {command}")
        sys.exit(1)
