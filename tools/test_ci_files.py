"""Tests of the CI helpers (tools/ci): the GITHUB_ENV and GITHUB_PATH lines, and the Pillow pin."""

import unittest
from pathlib import Path

from ci.github_files import env_line, path_line
from ci.pillow_pin import pin


class EnvLine(unittest.TestCase):
    def test_writes_name_and_value(self):
        self.assertEqual(env_line("PG_DESKTOP_CONTENT_KEY", "abc123"), "PG_DESKTOP_CONTENT_KEY=abc123\n")

    def test_refuses_a_line_break_in_the_value(self):
        with self.assertRaises(SystemExit):
            env_line("X", "a\nY=b")

    def test_refuses_a_name_that_is_not_one(self):
        with self.assertRaises(SystemExit):
            env_line("X=Y", "v")


class PathLine(unittest.TestCase):
    def test_a_relative_directory_is_taken_from_the_workspace(self):
        self.assertEqual(path_line(".deps/ffmpeg/bin", Path("/work/repo")), str(Path("/work/repo/.deps/ffmpeg/bin")) + "\n")

    def test_an_absolute_directory_is_kept(self):
        absolute = str(Path("/opt/tools").resolve())
        self.assertEqual(path_line(absolute, Path("/work/repo")), absolute + "\n")


class PillowPin(unittest.TestCase):
    def test_finds_the_pin_among_the_other_lines(self):
        text = "numpy==2.5.4\npillow==13.0.0\ntorch==2.8.0\n"
        self.assertEqual(pin(text), "pillow==13.0.0")

    def test_no_pin_is_none(self):
        self.assertIsNone(pin("numpy==2.5.4\n"))

    def test_the_name_is_case_blind(self):
        self.assertEqual(pin("Pillow==13.0.0\n"), "Pillow==13.0.0")


if __name__ == "__main__":
    unittest.main()
