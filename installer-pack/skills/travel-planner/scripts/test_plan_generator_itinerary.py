import copy
import unittest
from unittest.mock import patch

import plan_generator


def trip(itinerary=None):
    data = {
        "id": "synthetic-s045",
        "destination": {"city": "Test City", "country": "Test Country"},
        "departure_date": "2030-06-01",
        "return_date": "2030-06-04",
        "duration_days": 3,
        "budget": {"total": 3000},
        "climate": "moderate",
        "activities": [],
    }
    if itinerary is not None:
        data["itinerary"] = itinerary
    return data


class ItineraryAssessmentTests(unittest.TestCase):
    def make_plan(self, data):
        with patch.object(plan_generator, "get_preferences", return_value={
            "interests": [], "pace_preference": "moderate", "budget_level": "mid-range"
        }):
            return plan_generator.generate_trip_plan(data)

    def test_generated_synthetic_itinerary_is_a_skeleton(self):
        result = self.make_plan(trip())
        assessment = result["itinerary_assessment"]
        self.assertTrue(assessment["needs_research"])
        self.assertEqual(assessment["status"], "template_skeleton_or_incomplete")
        self.assertEqual(assessment["completion_label"], "模板骨架/信息不完整")
        self.assertEqual(len(result["itinerary"]), 3)
        self.assertEqual(len(assessment["missing_requirements"]), 3)
        for day in assessment["missing_requirements"]:
            self.assertIn("备选信息：替代活动、时间安排、交通方式、预算影响", day["missing_requirements"])
        self.assertEqual(result["budget"]["total_budget"], 3000)

    def test_empty_caller_itinerary_lists_every_expected_day_as_missing(self):
        result = self.make_plan(trip([]))
        assessment = result["itinerary_assessment"]
        self.assertTrue(assessment["needs_research"])
        self.assertEqual([entry["day"] for entry in assessment["missing_requirements"]], [1, 2, 3])
        for entry in assessment["missing_requirements"]:
            self.assertEqual(len(entry["missing_requirements"]), 2)

    def test_short_caller_itinerary_lists_unprovided_days(self):
        schedule = [{
            "day": 1,
            "morning": {"activity": "Synthetic named activity"},
            "alternatives": [{"activity": "Synthetic option", "time": "10:00", "transport": "Walk", "budget_impact": "$0"}],
        }]
        result = self.make_plan(trip(schedule))
        assessment = result["itinerary_assessment"]
        self.assertTrue(assessment["needs_research"])
        self.assertEqual(result["itinerary"], schedule)
        self.assertEqual([entry["day"] for entry in assessment["missing_requirements"]], [2, 3])

    def test_complete_caller_itinerary_is_preserved_but_unverified(self):
        schedule = [{
            "day": day,
            "date": f"2030-06-0{day}",
            "morning": {"time": "09:00", "activity": f"Synthetic museum {day}"},
            "alternatives": [{
                "activity": f"Synthetic indoor option {day}",
                "time": "14:00-16:00",
                "transport": "Synthetic local bus",
                "budget_impact": "Synthetic +$5",
            }],
        } for day in range(1, 4)]
        original = copy.deepcopy(schedule)
        result = self.make_plan(trip(schedule))
        assessment = result["itinerary_assessment"]
        self.assertEqual(result["itinerary"], original)
        self.assertEqual(assessment["status"], "alternatives_complete_unverified")
        self.assertEqual(assessment["completion_label"], "备选信息完整（未核验）")
        self.assertFalse(assessment["needs_research"])
        self.assertEqual(assessment["verification_status"], "caller_provided_unverified")
        self.assertIn("未独立核验", assessment["warnings"][0])

    def test_incomplete_alternative_lists_missing_fields(self):
        schedule = [{
            "day": 1,
            "morning": {"activity": "Synthetic named activity"},
            "alternatives": [{"activity": "Synthetic replacement", "transport": "Walk"}],
        }]
        assessment = self.make_plan(trip(schedule))["itinerary_assessment"]
        self.assertTrue(assessment["needs_research"])
        self.assertEqual(assessment["missing_requirements"][0], {
            "day": 1,
            "missing_requirements": ["备选信息：替代活动、时间安排、交通方式、预算影响"],
        })
        self.assertEqual([entry["day"] for entry in assessment["missing_requirements"]], [1, 2, 3])

    def test_extra_caller_day_is_retained_but_requires_research(self):
        schedule = [{"day": day, "morning": {"activity": f"Synthetic activity {day}"},
                     "alternatives": [{"activity": "Synthetic option", "time": "10:00", "transport": "Walk", "budget_impact": "$0"}]}
                    for day in range(1, 5)]
        result = self.make_plan(trip(schedule))
        self.assertEqual(result["itinerary"], schedule)
        self.assertTrue(result["itinerary_assessment"]["needs_research"])
        self.assertIn("超出预期 3 天", result["itinerary_assessment"]["warnings"][-1])

    def test_duplicate_day_numbers_cannot_be_marked_complete(self):
        schedule = [{"day": day, "morning": {"activity": f"Synthetic activity {day}"},
                     "alternatives": [{"activity": "Synthetic option", "time": "10:00", "transport": "Walk", "budget_impact": "$0"}]}
                    for day in (1, 1, 2, 3)]
        result = self.make_plan(trip(schedule))
        assessment = result["itinerary_assessment"]
        self.assertEqual(result["itinerary"], schedule)
        self.assertTrue(assessment["needs_research"])
        self.assertNotEqual(assessment["completion_label"], "备选信息完整（未核验）")
        self.assertTrue(any("重复日编号" in warning for warning in assessment["warnings"]))

    def test_fractional_and_boolean_day_numbers_cannot_be_marked_complete(self):
        complete_day = lambda day: {
            "day": day,
            "morning": {"activity": "Synthetic named activity"},
            "alternatives": [{"activity": "Synthetic option", "time": "10:00",
                              "transport": "Walk", "budget_impact": "$0"}],
        }
        for invalid_day in (1.5, True):
            with self.subTest(invalid_day=invalid_day):
                assessment = self.make_plan(trip([
                    complete_day(invalid_day), complete_day(2), complete_day(3)
                ]))["itinerary_assessment"]
                self.assertTrue(assessment["needs_research"])
                self.assertNotEqual(assessment["completion_label"], "备选信息完整（未核验）")
                self.assertTrue(any("日编号无效" in warning for warning in assessment["warnings"]))

    def test_non_dictionary_entry_is_reported_instead_of_raising(self):
        result = self.make_plan(trip([None]))
        assessment = result["itinerary_assessment"]
        self.assertTrue(assessment["needs_research"])
        self.assertTrue(any("不是有效对象" in warning for warning in assessment["warnings"]))
        self.assertIn("有效的日程对象", assessment["missing_requirements"][0]["missing_requirements"])

    def test_placeholder_activity_and_alternative_fields_do_not_count(self):
        schedule = [{
            "day": 1,
            "morning": {"activity": "TBD"},
            "alternatives": [{
                "activity": None,
                "time": "to be determined",
                "transport": "unknown",
                "budget_impact": "N/A",
            }],
        }]
        result = self.make_plan(trip(schedule))
        assessment = result["itinerary_assessment"]
        self.assertTrue(assessment["needs_research"])
        self.assertEqual(assessment["missing_requirements"][0]["missing_requirements"], [
            "具名日程（至少一项）",
            "备选信息：替代活动、时间安排、交通方式、预算影响",
        ])

    def test_valid_trip_wide_mapped_fallback_covers_each_day(self):
        schedule = [{"day": day, "morning": {"activity": f"Synthetic activity {day}"}}
                    for day in range(1, 4)]
        data = trip(schedule)
        data["trip_wide_alternatives"] = [{
            "days": [1, 2, 3],
            "activity": "Synthetic indoor alternative",
            "time": "14:00-16:00",
            "transport": "Synthetic local bus",
            "budget_impact": "Synthetic +$5",
        }]
        result = self.make_plan(data)
        self.assertEqual(result["itinerary"], schedule)
        self.assertEqual(result["trip_wide_alternatives"], data["trip_wide_alternatives"])
        self.assertEqual(result["trip_wide_alternatives_verification"], "caller_provided_unverified")
        self.assertEqual(result["itinerary_assessment"]["completion_label"], "备选信息完整（未核验）")
        self.assertFalse(result["itinerary_assessment"]["needs_research"])

    def test_trip_wide_fallback_missing_day_mapping_is_incomplete(self):
        schedule = [{"day": day, "morning": {"activity": f"Synthetic activity {day}"}}
                    for day in range(1, 4)]
        data = trip(schedule)
        data["trip_wide_alternatives"] = [{
            "days": [1, 2],
            "activity": "Synthetic indoor alternative",
            "time": "14:00-16:00",
            "transport": "Synthetic local bus",
            "budget_impact": "Synthetic +$5",
        }]
        assessment = self.make_plan(data)["itinerary_assessment"]
        self.assertTrue(assessment["needs_research"])
        self.assertEqual(assessment["missing_requirements"], [{
            "day": 3,
            "missing_requirements": ["备选信息：替代活动、时间安排、交通方式、预算影响"],
        }])

    def test_fractional_trip_wide_day_mapping_does_not_cover_an_expected_day(self):
        schedule = [{"day": day, "morning": {"activity": f"Synthetic activity {day}"}}
                    for day in range(1, 4)]
        data = trip(schedule)
        data["trip_wide_alternatives"] = [{
            "days": [1.5, 2, 3],
            "activity": "Synthetic indoor alternative",
            "time": "14:00-16:00",
            "transport": "Synthetic local bus",
            "budget_impact": "Synthetic +$5",
        }]
        assessment = self.make_plan(data)["itinerary_assessment"]
        self.assertTrue(assessment["needs_research"])
        self.assertEqual(assessment["missing_requirements"], [{
            "day": 1,
            "missing_requirements": ["备选信息：替代活动、时间安排、交通方式、预算影响"],
        }])


if __name__ == "__main__":
    unittest.main()
