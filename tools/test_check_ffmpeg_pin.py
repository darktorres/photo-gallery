"""Tests for tools/check_ffmpeg_pin.py: the pin must be a month-end build, not yet expired, and the static build's commit."""

import datetime
import unittest

from check_ffmpeg_pin import check, expiry

COMMIT = "7adfbe40dcde598f93a6963d2f7b22cd75202ccc"
STATIC = f'FFMPEG_COMMIT = "{COMMIT}"  # the commit\n'
MONTH_END = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-31-12-59/ffmpeg-n9.0.2-39-g7adfbe40dc-win64-gpl-shared-9.0.zip"
DAILY = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-11-19-12-39/ffmpeg-n9.0.2-58-g8c7ef49002-win64-gpl-shared-9.0.zip"
TODAY = datetime.date(2026, 11, 19)


class Pin(unittest.TestCase):
    def test_a_month_end_build_at_the_static_commit_passes(self):
        self.assertEqual(check(MONTH_END, STATIC, TODAY), ([], []))

    def test_a_daily_build_is_refused(self):
        errors, _ = check(DAILY, STATIC, TODAY)
        self.assertEqual(len(errors), 2, errors)  # not month-end, and a commit other than the static build's
        self.assertIn("last day of its month", errors[0])

    def test_a_commit_other_than_the_static_build_is_refused(self):
        other = MONTH_END.replace("g7adfbe40dc", "g8c7ef49002")
        errors, _ = check(other, STATIC, TODAY)
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("FFMPEG_COMMIT", errors[0])

    def test_a_pin_at_its_expiry_is_refused(self):
        errors, _ = check(MONTH_END, STATIC, datetime.date(2028, 10, 31))
        self.assertTrue(any("expired" in e for e in errors), errors)

    def test_a_pin_within_90_days_of_its_expiry_warns(self):
        errors, warnings = check(MONTH_END, STATIC, datetime.date(2028, 10, 1))
        self.assertEqual(errors, [])
        self.assertEqual(len(warnings), 1)

    def test_a_pin_far_from_its_expiry_is_quiet(self):
        self.assertEqual(check(MONTH_END, STATIC, datetime.date(2028, 1, 1)), ([], []))

    def test_a_url_with_no_tag_is_refused(self):
        errors, _ = check("https://example.invalid/ffmpeg.zip", STATIC, TODAY)
        self.assertEqual(len(errors), 1)

    def test_a_static_build_with_no_commit_is_refused(self):
        errors, _ = check(MONTH_END, "FFMPEG_COMMIT = None\n", TODAY)
        self.assertTrue(any("no commit" in e for e in errors), errors)


class Expiry(unittest.TestCase):
    def test_two_years_on(self):
        self.assertEqual(expiry(datetime.date(2026, 10, 31)), datetime.date(2028, 10, 31))

    def test_a_leap_day_expires_on_the_28th(self):
        self.assertEqual(expiry(datetime.date(2024, 2, 29)), datetime.date(2026, 2, 28))


if __name__ == "__main__":
    unittest.main()
