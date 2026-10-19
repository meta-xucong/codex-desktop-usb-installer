"""Offline regressions for explicit packing climate classification."""

from __future__ import annotations

import unittest

import plan_generator


class PackingClimateClassificationTests(unittest.TestCase):
    def _clothing(self, climate):
        return set(plan_generator.generate_packing_checklist(climate, 5, [])['clothing'])

    def test_english_and_chinese_cold_labels_use_cold_weather_items(self):
        cold_items = {
            "Warm jacket/coat", "Thermal underwear", "Warm socks",
            "Gloves and scarf", "Winter boots",
        }
        for label in ("cold", "winter", "寒冷", "冬季", "寒冷地区", "冬季气候"):
            with self.subTest(label=label):
                self.assertTrue(cold_items.issubset(self._clothing(label)))

    def test_warm_tropical_and_moderate_labels_keep_their_categories(self):
        warm_items = {"Lightweight, breathable clothes", "Swimsuit", "Sandals/flip-flops"}
        for label in ("warm", "warm Mediterranean", "tropical", "热带", "温暖"):
            with self.subTest(label=label):
                clothing = self._clothing(label)
                self.assertTrue(warm_items.issubset(clothing))
                self.assertNotIn("Warm jacket/coat", clothing)

        moderate_items = {"Mix of light and warm layers", "Light jacket", "Comfortable walking shoes"}
        for label in ("moderate", "temperate", "mild", "温和", "温带"):
            with self.subTest(label=label):
                clothing = self._clothing(label)
                self.assertTrue(moderate_items.issubset(clothing))
                self.assertNotIn("Warm jacket/coat", clothing)

    def test_empty_nonstring_and_unknown_labels_fail_explicitly(self):
        for label in ("", "   ", None, 7, "中性气候", "monsoon"):
            with self.subTest(label=label):
                with self.assertRaisesRegex(ValueError, "destination_climate|Unsupported"):
                    plan_generator.generate_packing_checklist(label, 5, [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
