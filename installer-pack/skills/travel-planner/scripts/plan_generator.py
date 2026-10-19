#!/usr/bin/env python
"""
Travel Plan Generator

Generates detailed travel plans including itinerary, budget, and checklists.
"""

import sys
import re
import math
from collections.abc import Mapping
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent))

from travel_db import get_preferences, add_trip, PREFERENCES_FILE, TRIPS_FILE
import json
from datetime import datetime, timedelta
from typing import Dict, Any, List


def _reject_database_output_alias(output: Path, database_paths: list[Path]) -> None:
    """Prevent an exported plan from replacing persistent travel data."""
    for database_path in database_paths:
        if output.resolve() == database_path.resolve():
            raise ValueError(f"output must differ from travel database file: {database_path.name}")
        if output.exists() and database_path.exists() and output.samefile(database_path):
            raise ValueError(f"output must differ from travel database file: {database_path.name}")


def generate_daily_itinerary(destination: str, num_days: int,
                            interests: List[str], pace: str = "moderate") -> List[Dict[str, Any]]:
    """
    Generate a suggested daily itinerary structure.

    This is a template that should be filled with actual attractions and activities
    based on web research or user input.
    """
    itinerary = []

    activities_per_day = {
        "relaxed": 2,
        "moderate": 3,
        "packed": 4
    }

    num_activities = activities_per_day.get(pace, 3)

    for day in range(1, num_days + 1):
        day_plan = {
            "day": day,
            "date": "",  # To be filled with actual dates
            "morning": {
                "time": "9:00 AM - 12:00 PM",
                "activity": f"Activity {day}A (to be customized)",
                "type": "sightseeing",
                "duration": "3 hours",
                "notes": "Based on user interests"
            },
            "afternoon": {
                "time": "2:00 PM - 5:00 PM",
                "activity": f"Activity {day}B (to be customized)",
                "type": "experience",
                "duration": "3 hours",
                "notes": ""
            },
            "evening": {
                "time": "7:00 PM - 10:00 PM",
                "activity": f"Activity {day}C (to be customized)",
                "type": "dining",
                "duration": "2-3 hours",
                "notes": ""
            },
            "meals": {
                "breakfast": "Hotel/Local cafe",
                "lunch": "Near afternoon activity",
                "dinner": "Local restaurant recommendation"
            },
            "accommodation": "Hotel/Accommodation name"
        }

        if pace == "relaxed":
            day_plan.pop("evening")  # Fewer activities for relaxed pace

        itinerary.append(day_plan)

    return itinerary


def calculate_budget_breakdown(total_budget: float, num_days: int,
                               accommodation_level: str = "mid-range") -> Dict[str, Any]:
    """
    Generate budget breakdown by category.
    """
    if isinstance(total_budget, bool) or not isinstance(total_budget, (int, float)):
        raise ValueError("total_budget must be a finite non-negative number")
    try:
        finite_budget = math.isfinite(total_budget)
    except OverflowError:
        finite_budget = False
    if not finite_budget or total_budget < 0:
        raise ValueError("total_budget must be a finite non-negative number")
    if isinstance(num_days, bool) or not isinstance(num_days, int) or num_days < 1:
        raise ValueError("num_days must be a positive integer")

    # Default percentage allocations
    allocations = {
        "budget": {
            "accommodation": 0.40,
            "food": 0.25,
            "activities": 0.20,
            "transportation": 0.10,
            "miscellaneous": 0.05
        },
        "mid-range": {
            "accommodation": 0.35,
            "food": 0.25,
            "activities": 0.25,
            "transportation": 0.10,
            "miscellaneous": 0.05
        },
        "luxury": {
            "accommodation": 0.45,
            "food": 0.20,
            "activities": 0.20,
            "transportation": 0.10,
            "miscellaneous": 0.05
        }
    }

    allocation = allocations.get(accommodation_level, allocations["mid-range"])

    breakdown = {}
    for category, percentage in allocation.items():
        amount = total_budget * percentage
        per_day = amount / num_days if num_days > 0 else 0
        breakdown[category] = {
            "total": round(amount, 2),
            "per_day": round(per_day, 2),
            "percentage": percentage * 100
        }

    return {
        "total_budget": total_budget,
        "duration_days": num_days,
        "breakdown": breakdown,
        "daily_average": round(total_budget / num_days, 2) if num_days > 0 else 0
    }


def generate_packing_checklist(destination_climate: str, duration_days: int,
                               trip_activities: List[str]) -> Dict[str, List[str]]:
    """
    Generate packing checklist based on destination and activities.
    """
    checklist = {
        "essentials": [
            "Passport",
            "Visa (if required)",
            "Travel insurance documents",
            "Flight tickets/boarding passes",
            "Hotel confirmations",
            "Credit/debit cards",
            "Local currency",
            "Phone and charger",
            "Adapter/converter (if needed)",
            "Medications (prescription and basic)",
            "Copies of important documents"
        ],
        "clothing": [],
        "toiletries": [
            "Toothbrush and toothpaste",
            "Shampoo and soap",
            "Deodorant",
            "Sunscreen",
            "Any personal care items"
        ],
        "technology": [
            "Phone charger",
            "Power bank",
            "Camera (if bringing)",
            "Headphones",
            "Laptop/tablet (if needed)"
        ],
        "activities": []
    }

    if not isinstance(destination_climate, str) or not destination_climate.strip():
        raise ValueError("destination_climate must be a non-empty supported climate label")
    climate = destination_climate.strip().casefold()

    # Classify explicitly so unfamiliar labels never silently receive moderate
    # packing. English substring matching preserves the existing free-text labels.
    if any(label in climate for label in ("tropical", "warm", "热带", "温暖", "炎热")):
        climate_category = "warm"
    elif any(label in climate for label in ("cold", "winter", "寒冷", "冬季")):
        climate_category = "cold"
    elif any(label in climate for label in ("temperate", "mild", "moderate", "温和", "温带")):
        climate_category = "moderate"
    else:
        raise ValueError(f"Unsupported destination_climate label: {destination_climate!r}")

    # Add climate-appropriate clothing
    if climate_category == "warm":
        checklist["clothing"].extend([
            "Lightweight, breathable clothes",
            "Shorts and t-shirts",
            "Sundress/summer clothes",
            "Swimsuit",
            "Sun hat",
            "Sunglasses",
            "Sandals/flip-flops"
        ])
    elif climate_category == "cold":
        checklist["clothing"].extend([
            "Warm jacket/coat",
            "Sweaters/hoodies",
            "Long pants",
            "Thermal underwear",
            "Warm socks",
            "Gloves and scarf",
            "Winter boots"
        ])
    else:  # Moderate/temperate
        checklist["clothing"].extend([
            "Mix of light and warm layers",
            "T-shirts and long-sleeve shirts",
            "Pants and shorts",
            "Light jacket",
            "Comfortable walking shoes",
            "Sneakers"
        ])

    # Add activity-specific items
    activity_items = {
        "hiking": ["Hiking boots", "Backpack", "Water bottle", "Trail snacks"],
        "beach": ["Swimsuit", "Beach towel", "Snorkel gear", "Waterproof bag"],
        "formal": ["Dress clothes", "Dress shoes", "Nice accessories"],
        "adventure": ["Athletic wear", "Action camera", "First aid kit"],
        "business": ["Business attire", "Laptop", "Business cards", "Portfolio"]
    }

    for activity in trip_activities:
        activity_lower = activity.lower()
        for key, items in activity_items.items():
            if key in activity_lower:
                checklist["activities"].extend(items)

    # Remove duplicates
    for category in checklist:
        checklist[category] = list(set(checklist[category]))

    return checklist


def generate_pre_trip_checklist(destination_country: str, departure_date: str) -> List[Dict[str, Any]]:
    """
    Generate pre-trip preparation checklist with timeline.
    """
    departure = _parse_departure_date(departure_date)

    today = datetime.now()
    days_until = (departure - today).days

    checklist = []

    # 3 months before (90 days)
    if days_until >= 90:
        checklist.append({
            "timeline": "3 months before",
            "tasks": [
                "Research destination and create wish list",
                "Check passport expiration (needs 6+ months validity)",
                "Research visa requirements",
                "Set up travel alerts for flights",
                "Start saving/budgeting for trip"
            ]
        })

    # 2 months before (60 days)
    if days_until >= 60:
        checklist.append({
            "timeline": "2 months before",
            "tasks": [
                "Book flights",
                "Book accommodation",
                "Apply for visa if needed",
                "Purchase travel insurance",
                "Check vaccination requirements",
                "Research local customs and etiquette"
            ]
        })

    # 1 month before (30 days)
    if days_until >= 30:
        checklist.append({
            "timeline": "1 month before",
            "tasks": [
                "Book major activities and tours",
                "Notify bank of travel dates",
                "Set up international phone plan",
                "Make restaurant reservations",
                "Check weather forecasts",
                "Start gathering packing items"
            ]
        })

    # 2 weeks before
    if days_until >= 14:
        checklist.append({
            "timeline": "2 weeks before",
            "tasks": [
                "Confirm all reservations",
                "Print important documents",
                "Exchange some currency",
                "Refill prescriptions",
                "Arrange pet/plant care",
                "Hold mail delivery"
            ]
        })

    # 1 week before
    if days_until >= 7:
        checklist.append({
            "timeline": "1 week before",
            "tasks": [
                "Check in for flights",
                "Download offline maps",
                "Pack luggage",
                "Charge all devices",
                "Clean out refrigerator",
                "Set up home security"
            ]
        })

    # Day before
    checklist.append({
        "timeline": "Day before departure",
        "tasks": [
            "Re-check flight times",
            "Prepare carry-on essentials",
            "Take out trash",
            "Check weather at destination",
            "Get good rest",
            "Set multiple alarms"
        ]
    })

    return checklist


def _parse_departure_date(value: Any) -> datetime:
    """Parse the contract's calendar-date-only YYYY-MM-DD value."""
    if not isinstance(value, str) or not value.strip():
        raise ValueError("departure_date must be a non-empty YYYY-MM-DD date")
    normalized = value.strip()
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", normalized):
        raise ValueError("departure_date must use YYYY-MM-DD format")
    try:
        return datetime.strptime(normalized, "%Y-%m-%d")
    except (TypeError, ValueError, OverflowError) as exc:
        raise ValueError("departure_date must be a valid calendar date in YYYY-MM-DD format") from exc


def _is_usable_text(value: Any) -> bool:
    """Reject empty and common placeholder values for activity/alternative fields."""
    if not isinstance(value, str):
        return False
    text = str(value).strip().lower()
    if not text:
        return False
    if re.search(r"\b(?:tbd|todo|unknown|n/?a|placeholder|undetermined)\b", text):
        return False
    phrases = (
        "to be customized", "to be determined", "not decided", "not specified",
        "待定", "待补充", "待填写", "未确定", "占位",
    )
    return not any(marker in text for marker in phrases)


def _alternative_fields(alternative: Dict[str, Any]) -> Dict[str, Any]:
    def first_usable(*keys: str) -> Any:
        for key in keys:
            value = alternative.get(key)
            if _is_usable_text(value):
                return value
        return None

    return {
        "替代活动": first_usable("activity", "alternative_activity"),
        "时间安排": first_usable("time", "schedule"),
        "交通方式": first_usable("transport", "transportation"),
        "预算影响": first_usable("budget_impact"),
    }


def _exact_day_number(value: Any):
    """Return an exact integer day number; reject booleans and fractional values."""
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, str) and re.fullmatch(r"[+-]?\d+", value.strip()):
        try:
            return int(value.strip())
        except ValueError:
            return None
    return None


def assess_itinerary(itinerary: List[Dict[str, Any]], expected_days: int = None,
                     trip_wide_alternatives: Any = None) -> Dict[str, Any]:
    """Describe itinerary completeness without validating caller-supplied facts."""
    missing_by_day = []
    warnings = []
    covered_days = set()
    day_counts = {}
    mapped_fallback_days = set()
    fallback_entries = trip_wide_alternatives
    if isinstance(fallback_entries, dict):
        fallback_entries = [fallback_entries]
    for fallback in fallback_entries if isinstance(fallback_entries, list) else []:
        if not isinstance(fallback, dict) or not all(_is_usable_text(value) for value in _alternative_fields(fallback).values()):
            continue
        mapped_days = fallback.get("days", [])
        if not isinstance(mapped_days, list):
            continue
        for mapped_day in mapped_days:
            day_number = _exact_day_number(mapped_day)
            if day_number is not None:
                mapped_fallback_days.add(day_number)
    for index, day in enumerate(itinerary, start=1):
        if not isinstance(day, dict):
            missing_by_day.append({
                "day": index,
                "missing_requirements": ["有效的日程对象", "具名日程（至少一项）", "备选信息：替代活动、时间安排、交通方式、预算影响"],
            })
            warnings.append(f"第 {index} 个行程条目不是有效对象，无法评估。")
            continue
        raw_day_number = day.get("day", index)
        day_number = _exact_day_number(raw_day_number)
        if day_number is None:
            day_number = index
            warnings.append(f"第 {index} 个行程条目日编号无效（{raw_day_number!r}），按缺项处理。")
        covered_days.add(day_number)
        day_counts[day_number] = day_counts.get(day_number, 0) + 1
        requirements = []

        # Accept named activities in the conventional time slots or an activities list.
        named = []
        activities = day.get("activities", [])
        if isinstance(activities, list):
            named.extend(item.get("activity") for item in activities if isinstance(item, dict))
            named.extend(item for item in activities if isinstance(item, str))
        for slot in ("morning", "afternoon", "evening"):
            item = day.get(slot)
            if isinstance(item, dict):
                named.append(item.get("activity"))
        if not any(_is_usable_text(value) for value in named):
            requirements.append("具名日程（至少一项）")

        alternatives = day.get("alternatives", [])
        if isinstance(alternatives, dict):
            alternatives = [alternatives]
        valid_alternative = False
        for alternative in alternatives if isinstance(alternatives, list) else []:
            if not isinstance(alternative, dict):
                continue
            fields = _alternative_fields(alternative)
            if all(value is not None for value in fields.values()):
                valid_alternative = True
                break
        if not valid_alternative and day_number not in mapped_fallback_days:
            requirements.append("备选信息：替代活动、时间安排、交通方式、预算影响")
        if requirements:
            missing_by_day.append({"day": day_number, "missing_requirements": requirements})

    if expected_days is not None:
        duplicate_days = sorted(day for day, count in day_counts.items() if count > 1)
        if duplicate_days:
            warnings.append(f"行程包含重复日编号：{duplicate_days}；请确保每个预期日仅有一条日程。")
        for day_number in range(1, max(0, expected_days) + 1):
            if day_number not in covered_days:
                missing_by_day.append({
                    "day": day_number,
                    "missing_requirements": ["具名日程（至少一项）", "备选信息：替代活动、时间安排、交通方式、预算影响"],
                })
        extra_days = sorted(day for day in covered_days if day < 1 or day > expected_days)
        if extra_days:
            warnings.append(f"行程包含超出预期 {expected_days} 天的条目：{extra_days}；请核对 duration_days。")
    complete = bool(itinerary) and not missing_by_day and not warnings
    if not complete:
        warnings.insert(0, "当前行程为模板骨架或信息不完整；请补充缺项并研究核验地点、路线、时间、交通和预算。")
    else:
        warnings.append("地点、路线、时间、交通和预算数据均未独立核验；出行前请核对官方及实时信息。")
    return {
        "status": "alternatives_complete_unverified" if complete else "template_skeleton_or_incomplete",
        "completion_label": "备选信息完整（未核验）" if complete else "模板骨架/信息不完整",
        "needs_research": not complete,
        "verification_status": "caller_provided_unverified",
        "missing_requirements": missing_by_day,
        "warnings": warnings,
    }


def generate_trip_plan(trip_data: Dict[str, Any]) -> Dict[str, Any]:
    """
    Generate complete trip plan with all components.
    """
    if not isinstance(trip_data, Mapping):
        raise ValueError("trip_data must be a mapping")
    destination = trip_data.get("destination", {})
    if not isinstance(destination, Mapping):
        raise ValueError("destination must be a mapping")
    budget_data = trip_data.get("budget", {})
    if not isinstance(budget_data, Mapping):
        raise ValueError("budget must be a mapping")
    duration = trip_data.get("duration_days", 7)
    if isinstance(duration, bool) or not isinstance(duration, int) or duration < 1:
        raise ValueError("duration_days must be a positive integer")
    budget = budget_data.get("total", 0)
    departure_date = trip_data.get("departure_date")
    _parse_departure_date(departure_date)

    # Get user preferences
    prefs = get_preferences()
    interests = prefs.get("interests", [])
    pace = prefs.get("pace_preference", "moderate")
    accommodation_level = prefs.get("budget_level", "mid-range")

    # Generate components
    # Caller-supplied itinerary is preserved verbatim. The fallback is explicitly a
    # template and must not be read as a researched recommendation.
    itinerary = trip_data.get("itinerary")
    if not isinstance(itinerary, list):
        itinerary = generate_daily_itinerary(
            destination.get("city", ""), duration, interests, pace
        )
    trip_wide_alternatives = trip_data.get("trip_wide_alternatives")
    plan = {
        "trip_id": trip_data.get("id", ""),
        "destination": destination,
        "dates": {
            "departure": trip_data.get("departure_date", ""),
            "return": trip_data.get("return_date", ""),
            "duration_days": duration
        },
        "itinerary": itinerary,
        # Keep mapped alternatives in the generated plan so downstream consumers
        # can actually use the data that contributed to the completeness status.
        "trip_wide_alternatives": trip_wide_alternatives,
        "trip_wide_alternatives_verification": "caller_provided_unverified",
        "itinerary_assessment": assess_itinerary(
            itinerary,
            expected_days=duration,
            trip_wide_alternatives=trip_wide_alternatives,
        ),
        "budget": calculate_budget_breakdown(budget, duration, accommodation_level),
        "packing_checklist": generate_packing_checklist(
            trip_data.get("climate", "moderate"),
            duration,
            trip_data.get("activities", [])
        ),
        "pre_trip_checklist": generate_pre_trip_checklist(
            destination.get("country", ""),
            departure_date
        ),
        "generated_at": datetime.now().isoformat()
    }

    return plan


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description='Generate travel plan')
    parser.add_argument('--trip-id', help='Trip ID from database')
    parser.add_argument('--output', help='Output JSON file path')

    args = parser.parse_args()

    if not args.trip_id:
        print("Error: --trip-id required")
        sys.exit(1)

    from travel_db import get_trip_by_id

    try:
        trip = get_trip_by_id(args.trip_id)
    except (TypeError, ValueError) as exc:
        print(f"Error: invalid travel database: {exc}", file=sys.stderr)
        sys.exit(1)
    if not trip:
        print(f"Error: Trip {args.trip_id} not found")
        sys.exit(1)

    try:
        plan = generate_trip_plan(trip)
    except (TypeError, ValueError) as exc:
        print(f"Error: invalid trip data: {exc}", file=sys.stderr)
        sys.exit(1)

    if args.output:
        output_path = Path(args.output).expanduser().resolve()
        try:
            _reject_database_output_alias(output_path, [TRIPS_FILE, PREFERENCES_FILE])
        except ValueError as exc:
            print(f"Error: {exc}", file=sys.stderr)
            sys.exit(1)
        with output_path.open('w') as f:
            json.dump(plan, f, indent=2)
        print(f"Travel plan generated: {output_path}")
    else:
        print(json.dumps(plan, indent=2))
