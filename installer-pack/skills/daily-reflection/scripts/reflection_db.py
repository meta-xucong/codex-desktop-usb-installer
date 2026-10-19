#!/usr/bin/env python
"""
Daily Reflection Database Manager

Manages user reflection profiles, daily reflections, habit tracking, and goals.
"""

import copy
import json
import os
import re
import tempfile
import uuid
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


def load_json(file_path: Path, ensure: bool = True) -> Dict[str, Any]:
    """Load JSON from file."""
    if ensure:
        ensure_db_files()
    try:
        with open(file_path, 'r') as f:
            return json.load(f)
    except json.JSONDecodeError as exc:
        raise ValueError(
            f"Daily reflection data file '{file_path.name}' contains invalid JSON; refusing to continue."
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


def _save_json_pair(
    first_path: Path,
    first_data: Dict[str, Any],
    second_path: Path,
    second_data: Dict[str, Any],
) -> None:
    """Stage two JSON files and restore the first if the second replace fails."""
    first_content = json.dumps(first_data, indent=2)
    second_content = json.dumps(second_data, indent=2)
    first_path.parent.mkdir(parents=True, exist_ok=True)
    try:
        original_first = first_path.read_bytes()
    except FileNotFoundError:
        original_first = None
    first_temp = second_temp = rollback_temp = None
    first_replaced = False
    try:
        for path, content in ((first_path, first_content), (second_path, second_content)):
            with tempfile.NamedTemporaryFile(
                mode="w", encoding="utf-8", dir=path.parent,
                prefix=f".{path.name}.", suffix=".tmp", delete=False,
            ) as handle:
                staged = Path(handle.name)
                if path == first_path:
                    first_temp = staged
                else:
                    second_temp = staged
                handle.write(content)
                handle.flush()
                os.fsync(handle.fileno())

        os.replace(first_temp, first_path)
        first_temp = None
        first_replaced = True
        os.replace(second_temp, second_path)
        second_temp = None
    except Exception as exc:
        if first_replaced:
            try:
                with tempfile.NamedTemporaryFile(
                    mode="wb", dir=first_path.parent,
                    prefix=f".{first_path.name}.", suffix=".rollback.tmp", delete=False,
                ) as handle:
                    rollback_temp = Path(handle.name)
                    if original_first is not None:
                        handle.write(original_first)
                        handle.flush()
                        os.fsync(handle.fileno())
                if original_first is None:
                    first_path.unlink(missing_ok=True)
                else:
                    os.replace(rollback_temp, first_path)
                    rollback_temp = None
            except Exception as rollback_exc:
                raise RuntimeError(
                    f"Could not commit both data files and rollback '{first_path.name}': {rollback_exc}"
                ) from exc
        raise
    finally:
        for temporary_path in (first_temp, second_temp, rollback_temp):
            if temporary_path is not None:
                temporary_path.unlink(missing_ok=True)


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
    reflections = load_json(REFLECTIONS_FILE, ensure=False)
    includes_habits = "habits" in reflection
    habits = load_json(HABITS_FILE, ensure=False) if includes_habits else None
    if not isinstance(reflections.get("reflections", []), list):
        raise ValueError("Daily reflection data field 'reflections' must be a list.")
    if any(not isinstance(item, dict) for item in reflections.get("reflections", [])):
        raise ValueError("Each stored reflection must be an object.")
    if includes_habits:
        habits.setdefault("habits", [])
        habits.setdefault("habit_logs", [])
        if not isinstance(habits.get("habits", []), list):
            raise ValueError("Daily reflection data field 'habits' must be a list.")
        if not isinstance(habits.get("habit_logs", []), list):
            raise ValueError("Daily reflection data field 'habit_logs' must be a list.")
        if any(not isinstance(item, dict) for item in habits.get("habit_logs", [])):
            raise ValueError("Each stored habit log must be an object.")
        if not isinstance(reflection["habits"], list):
            raise ValueError("Reflection field 'habits' must be a list.")
        for index, habit in enumerate(reflection["habits"]):
            if not isinstance(habit, dict) or not habit.get("habit_id"):
                raise ValueError(f"Reflection habit {index} must be an object with a habit_id.")
            if type(habit.get("completed", False)) is not bool:
                raise ValueError(f"Reflection habit {index} completed must be a bool.")

    caller_reflection = reflection
    reflection = copy.deepcopy(reflection)
    reflection_id = uuid.uuid4().hex
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
        reflection_id = reflections["reflections"][existing_idx].get("reflection_id", reflection_id)
        reflection["reflection_id"] = reflection_id
        reflections["reflections"][existing_idx] = reflection
    else:
        # Add new
        reflections["reflections"].append(reflection)
    
    if includes_habits:
        _parse_canonical_habit_date(date, "date")
        for habit in reflection["habits"]:
            log_entry = {
                "log_id": uuid.uuid4().hex,
                "habit_id": habit["habit_id"],
                "date": date,
                "completed": habit.get("completed", False),
                "note": habit.get("note", ""),
                "logged_at": datetime.now().isoformat(),
            }
            existing_idx = next((
                i for i, item in enumerate(habits["habit_logs"])
                if item.get("habit_id") == log_entry["habit_id"] and item.get("date") == date
            ), None)
            if existing_idx is None:
                habits["habit_logs"].append(log_entry)
            else:
                log_entry["log_id"] = habits["habit_logs"][existing_idx].get("log_id", log_entry["log_id"])
                habits["habit_logs"][existing_idx] = log_entry
        _save_json_pair(HABITS_FILE, habits, REFLECTIONS_FILE, reflections)
        # Preserve the established input mutation behavior only after a successful save.
        caller_reflection.clear()
        caller_reflection.update(copy.deepcopy(reflection))
    else:
        save_json(REFLECTIONS_FILE, reflections)
        caller_reflection.clear()
        caller_reflection.update(copy.deepcopy(reflection))
    
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
    
    habit_id = uuid.uuid4().hex
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
    if type(completed) is not bool:
        raise ValueError("completed must be a bool.")
    _parse_canonical_habit_date(date, "date")
    habits = load_json(HABITS_FILE)
    
    log_entry = {
        "log_id": uuid.uuid4().hex,
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
        log_entry["log_id"] = habits["habit_logs"][existing_idx].get("log_id", log_entry["log_id"])
        habits["habit_logs"][existing_idx] = log_entry
    else:
        habits["habit_logs"].append(log_entry)
    
    save_json(HABITS_FILE, habits)
    return log_entry["log_id"]


def _parse_canonical_habit_date(value: str, label: str) -> datetime:
    if not isinstance(value, str):
        raise ValueError(f"{label} must use YYYY-MM-DD format.")
    try:
        parsed = datetime.strptime(value, "%Y-%m-%d")
    except ValueError as exc:
        raise ValueError(f"{label} must use YYYY-MM-DD format.") from exc
    if parsed.strftime("%Y-%m-%d") != value:
        raise ValueError(f"{label} must use zero-padded YYYY-MM-DD format.")
    return parsed


def get_habit_logs(
    habit_id: str = None,
    days: int = 30,
    start_date: str = None,
    end_date: str = None,
) -> List[Dict[str, Any]]:
    """Get habit logs, optionally filtered by habit and date range."""
    habits = load_json(HABITS_FILE)
    all_logs = habits.get("habit_logs", [])
    
    if habit_id:
        all_logs = [l for l in all_logs if l.get("habit_id") == habit_id]

    for index, log in enumerate(all_logs):
        if not isinstance(log, dict):
            raise ValueError(f"Habit log {index} must be an object.")
        _parse_canonical_habit_date(log.get("date"), f"habit log {index} date")

    # Explicit bounds make historical reports independent of today's date.
    if start_date is not None:
        _parse_canonical_habit_date(start_date, "start_date")
    if end_date is not None:
        _parse_canonical_habit_date(end_date, "end_date")
    if start_date is not None and end_date is not None and start_date > end_date:
        raise ValueError("start_date must not be after end_date.")
    if start_date is None and end_date is None:
        cutoff_date = (datetime.now() - timedelta(days=days)).strftime("%Y-%m-%d")
        all_logs = [l for l in all_logs if l.get("date", "") >= cutoff_date]
    else:
        if start_date is not None:
            all_logs = [l for l in all_logs if l.get("date", "") >= start_date]
        if end_date is not None:
            all_logs = [l for l in all_logs if l.get("date", "") <= end_date]
    
    # Sort by date
    all_logs.sort(key=lambda x: x.get("date", ""))
    return all_logs


def get_habit_stats(
    habit_id: str,
    days: int = 30,
    start_date: str = None,
    end_date: str = None,
) -> Dict[str, Any]:
    """Get statistics for a specific habit."""
    if (start_date is None) != (end_date is None):
        raise ValueError("start_date and end_date must be provided together.")
    logs = get_habit_logs(habit_id, days, start_date, end_date)
    period_days = days
    if start_date is not None:
        period_days = (datetime.strptime(end_date, "%Y-%m-%d") - datetime.strptime(start_date, "%Y-%m-%d")).days + 1
    habit = None
    
    for h in get_habits():
        if h.get("habit_id") == habit_id:
            habit = h
            break

    # Current streak is measured as of the report endpoint for an explicit range.
    streak_logs = logs
    reference_date = datetime.now().date()
    if end_date is not None:
        streak_logs = get_habit_logs(habit_id, start_date=None, end_date=end_date)
        reference_date = datetime.strptime(end_date, "%Y-%m-%d").date()
    streak_dates = sorted(
        [l.get("date") for l in streak_logs if l.get("completed")], reverse=True
    )
    current_streak = 0
    if streak_dates:
        today = reference_date.strftime("%Y-%m-%d")
        yesterday = (reference_date - timedelta(days=1)).strftime("%Y-%m-%d")
        if streak_dates[0] in [today, yesterday]:
            current_streak = 1
            for i in range(1, len(streak_dates)):
                expected_date = (datetime.strptime(streak_dates[i-1], "%Y-%m-%d") - timedelta(days=1)).strftime("%Y-%m-%d")
                if streak_dates[i] == expected_date:
                    current_streak += 1
                else:
                    break
    
    if not logs:
        return {
            "habit_id": habit_id,
            "habit_name": habit.get("name", "Unknown") if habit else "Unknown",
            "period_days": period_days,
            "completion_rate": 0,
            "current_streak": current_streak,
            "longest_streak": 0,
            "total_completions": 0
        }
    
    # Calculate completion rate
    completed_count = sum(1 for l in logs if l.get("completed"))
    completion_rate = completed_count / len(logs) * 100 if logs else 0
    
    dates = sorted([l.get("date") for l in logs if l.get("completed")], reverse=True)
    longest_streak = 0

    if dates:
        # Longest streak is scoped to the requested reporting period.
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
        "period_days": period_days,
        "completion_rate": round(completion_rate, 1),
        "current_streak": current_streak,
        "longest_streak": longest_streak,
        "total_completions": completed_count,
        "recent_logs": logs[-7:]
    }


def get_all_habits_status(
    days: int = 7,
    start_date: str = None,
    end_date: str = None,
) -> List[Dict[str, Any]]:
    """Get status for all habits."""
    habits = get_habits()
    result = []
    
    for habit in habits:
        habit_id = habit.get("habit_id")
        stats = get_habit_stats(habit_id, days, start_date, end_date)
        result.append({**habit, **stats})
    
    return result


# ============================================================================
# GOAL MANAGEMENT
# ============================================================================

def add_goal(goal: Dict[str, Any]) -> str:
    """Add a new goal."""
    goals = load_json(GOALS_FILE)
    
    goal_id = uuid.uuid4().hex
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
    
    week_start_date = _parse_canonical_habit_date(week_start, "week_start")
    week_start = week_start_date.strftime("%Y-%m-%d")
    week_end = (week_start_date + timedelta(days=6)).strftime("%Y-%m-%d")
    
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
    habits_status = get_all_habits_status(
        days=7, start_date=week_start, end_date=week_end
    )
    
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
    
    plan_id = uuid.uuid4().hex
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
    
    log_id = uuid.uuid4().hex
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
    
    entry_id = uuid.uuid4().hex
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
        try:
            print(json.dumps(generate_weekly_review(week_start), indent=2))
        except ValueError as exc:
            print(f"Error: {exc}", file=sys.stderr)
            sys.exit(2)
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
