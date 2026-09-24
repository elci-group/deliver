import unittest
from datetime import datetime

from cron import next_occurrence


def at(text: str) -> datetime:
    return datetime.strptime(text, "%Y-%m-%d %H:%M")


class CronNextOccurrenceTests(unittest.TestCase):
    def test_step_within_hour(self):
        self.assertEqual(next_occurrence("*/15 * * * *", at("2026-09-24 12:07")),
                         at("2026-09-24 12:15"))

    def test_start_never_returned_even_when_it_matches(self):
        self.assertEqual(next_occurrence("30 9 * * *", at("2026-09-24 09:30")),
                         at("2026-09-25 09:30"))

    def test_hour_range_with_step(self):
        self.assertEqual(next_occurrence("30 8-17/3 * * *", at("2026-09-24 12:31")),
                         at("2026-09-24 14:30"))

    def test_mixed_list_and_range(self):
        self.assertEqual(next_occurrence("30 8-11,17 * * *", at("2026-09-24 12:31")),
                         at("2026-09-24 17:30"))

    def test_first_of_month(self):
        self.assertEqual(next_occurrence("0 0 1 * *", at("2026-09-24 00:00")),
                         at("2026-10-01 00:00"))

    def test_day_31_skips_short_months(self):
        self.assertEqual(next_occurrence("0 0 31 * *", at("2026-09-24 00:00")),
                         at("2026-10-31 00:00"))

    def test_feb_29_only_in_leap_years(self):
        self.assertEqual(next_occurrence("0 0 29 2 *", at("2026-09-24 00:00")),
                         at("2028-02-29 00:00"))

    def test_day_of_week_monday(self):
        self.assertEqual(next_occurrence("0 0 * * 1", at("2026-09-24 00:00")),
                         at("2026-09-28 00:00"))

    def test_day_of_week_sunday_is_zero(self):
        self.assertEqual(next_occurrence("0 12 * * 0", at("2026-09-24 00:00")),
                         at("2026-09-27 12:00"))

    def test_dom_and_dow_restricted_matches_either(self):
        # 1st-of-month OR Monday: from Thursday 2026-09-24 the next match is
        # Monday 2026-09-28, not 2026-10-01.
        self.assertEqual(next_occurrence("0 0 1 * 1", at("2026-09-24 00:00")),
                         at("2026-09-28 00:00"))

    def test_dom_and_dow_either_rule_picks_closer_friday(self):
        # 13th-of-month OR Friday: next Friday 2026-09-25 beats Oct 13.
        self.assertEqual(next_occurrence("0 0 13 * 5", at("2026-09-24 00:00")),
                         at("2026-09-25 00:00"))

    def test_lists_in_day_fields(self):
        self.assertEqual(next_occurrence("0 6,18 1,15 * *", at("2026-09-24 00:00")),
                         at("2026-10-01 06:00"))

    def test_last_minute_of_year(self):
        self.assertEqual(next_occurrence("59 23 31 12 *", at("2026-09-24 00:00")),
                         at("2026-12-31 23:59"))

    def test_wraps_year_boundary(self):
        self.assertEqual(next_occurrence("*/15 * * * *", at("2026-12-31 23:52")),
                         at("2027-01-01 00:00"))

    def test_invalid_expressions_raise_value_error(self):
        for expr in ("60 * * * *", "* * * *", "* * * * * *", "*/0 * * * *",
                     "a * * * *", "5-1 * * * *", "30 5/2 * * *", "0 0 * * 7"):
            with self.subTest(expr=expr):
                with self.assertRaises(ValueError):
                    next_occurrence(expr, at("2026-09-24 00:00"))

    def test_impossible_dates_raise_value_error(self):
        for expr in ("0 0 31 2 *", "0 0 30 2 *"):
            with self.subTest(expr=expr):
                with self.assertRaises(ValueError):
                    next_occurrence(expr, at("2026-09-24 00:00"))


if __name__ == "__main__":
    unittest.main()
