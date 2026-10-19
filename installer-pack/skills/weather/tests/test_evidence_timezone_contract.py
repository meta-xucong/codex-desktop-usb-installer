from pathlib import Path
import unittest

SKILL = Path(__file__).resolve().parents[1] / "SKILL.md"

class WeatherEvidenceAndTimeContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = SKILL.read_text(encoding="utf-8")
        cls.compact = " ".join(cls.text.split())

    def test_resolves_ambiguous_place_and_echoes_provider_location(self):
        self.assertIn("地点缺失或同名地点无法消歧时，先询问", self.compact)
        self.assertIn("不能自行猜测", self.compact)
        self.assertIn("回显来源解析出的城市、行政区/国家", self.compact)
        self.assertIn("来源地点与用户指定不一致", self.compact)

    def test_query_observation_and_forecast_times_include_location_timezone(self):
        self.assertIn("带时区的查询时间", self.compact)
        self.assertIn("观测时间或预报有效时段", self.compact)
        self.assertIn("每个可用时间都要带地点时区或明确的 IANA 时区", self.compact)
        self.assertIn("不把查询者时区当作地点时区", self.compact)

    def test_unavailable_or_invalid_data_fails_closed(self):
        for phrase in ("空响应", "超时", "非成功 HTTP", "无效结构/JSON", "地点不匹配", "缺少必要时间字段", "数据明显过期"):
            with self.subTest(phrase=phrase):
                self.assertIn(phrase, self.compact)
        self.assertIn("按“当前数据不可核验”处理", self.compact)
        self.assertIn("不包装成实时天气", self.compact)

    def test_compact_format_cannot_claim_missing_metadata(self):
        self.assertIn("简写响应如 `format=3` 仅当实际响应字段足以支持所写结论时才可引用", self.compact)
        self.assertIn("没有地点或时间元数据时，不用于当前实况/预报报告", self.compact)

    def test_network_fallback_stays_conditional_and_does_not_guess(self):
        self.assertIn("当前执行环境明确允许外网请求时", self.compact)
        self.assertIn("所有回退都失败时，说明无法实时核验，不根据季节或常识猜天气", self.compact)

    def test_real_weather_and_theory_only_requests_remain_separated(self):
        self.assertIn("纯气象原理解释，或用户明确要求不查实时天气时，不调用天气服务", self.compact)

if __name__ == "__main__":
    unittest.main()
