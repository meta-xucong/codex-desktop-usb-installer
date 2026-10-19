import unittest

from polish_text import ArticlePolisher


class ArticlePolisherTests(unittest.TestCase):
    def test_reports_character_count_for_chinese_and_english(self) -> None:
        text = "AI 焦虑：故事 10%。"
        result = ArticlePolisher().polish(text)

        self.assertEqual(result["character_count"], len(text))
        self.assertNotIn("word_count", result)

    def test_character_count_includes_newlines(self) -> None:
        text = "第一段。\n\nSecond paragraph."
        result = ArticlePolisher().polish(text)

        self.assertEqual(result["character_count"], len(text))
        self.assertEqual(result["paragraph_count"], 2)


if __name__ == "__main__":
    unittest.main()
