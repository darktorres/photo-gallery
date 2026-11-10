"""Tests of pgtools.py's `.env` reading (stdlib unittest: `just test-tools`)."""

import unittest

from pgtools import _ENV_LINE, env_value


def read(line: str):
    """The (key, value) a `.env` line sets, or None."""
    match = _ENV_LINE.match(line)
    return (match.group(1), env_value(match.group(2))) if match else None


class EnvFile(unittest.TestCase):
    def test_a_quoted_value_loses_its_quotes(self):
        self.assertEqual(read('LLVM_DIR="C:/Program Files/LLVM"'), ("LLVM_DIR", "C:/Program Files/LLVM"))
        self.assertEqual(read("PG_ROOT='D:/Photo Gallery'"), ("PG_ROOT", "D:/Photo Gallery"))

    def test_a_bare_value_is_kept_as_it_is(self):
        self.assertEqual(read("PG_ROOT=D:/PhotoGallery"), ("PG_ROOT", "D:/PhotoGallery"))
        self.assertEqual(read("EMPTY="), ("EMPTY", ""))

    def test_unmatched_or_inner_quotes_are_kept(self):
        self.assertEqual(env_value('"open'), '"open')
        self.assertEqual(env_value('a "b" c'), 'a "b" c')
        self.assertEqual(env_value("\"mixed'"), "\"mixed'")

    def test_a_comment_line_sets_nothing(self):
        self.assertIsNone(read("# QT_DIR=C:/Qt"))


if __name__ == "__main__":
    unittest.main()
