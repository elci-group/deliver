import unittest

from calculator import parse_percent


class ParsePercentTests(unittest.TestCase):
    def test_accepts_whitespace_and_percent_sign(self):
        self.assertEqual(parse_percent(" 12.5% "), 0.125)

    def test_accepts_plain_number(self):
        self.assertEqual(parse_percent("25"), 0.25)

    def test_rejects_invalid_values(self):
        for value in ("", "abc", "10%%", "-1%", "101%"):
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    parse_percent(value)


if __name__ == "__main__":
    unittest.main()
