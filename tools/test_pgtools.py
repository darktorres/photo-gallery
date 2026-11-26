"""Tests of pgtools.py's `.env` reading (stdlib unittest: `just test-tools`)."""

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import pgtools
from pgtools import _ENV_LINE, env_value, load_env_file


def read(line: str):
    """The (key, value) a `.env` line sets, or None."""
    match = _ENV_LINE.match(line)
    return (match.group(1), env_value(match.group(2))) if match else None


class EnvFile(unittest.TestCase):
    def test_a_quoted_value_loses_its_quotes(self):
        self.assertEqual(read('PG_LLVM_DIR="C:/Program Files/LLVM"'), ("PG_LLVM_DIR", "C:/Program Files/LLVM"))
        self.assertEqual(read("PG_ROOT='D:/Photo Gallery'"), ("PG_ROOT", "D:/Photo Gallery"))

    def test_a_bare_value_is_kept_as_it_is(self):
        self.assertEqual(read("PG_ROOT=D:/PhotoGallery"), ("PG_ROOT", "D:/PhotoGallery"))
        self.assertEqual(read("EMPTY="), ("EMPTY", ""))

    def test_unmatched_or_inner_quotes_are_kept(self):
        self.assertEqual(env_value('"open'), '"open')
        self.assertEqual(env_value('a "b" c'), 'a "b" c')
        self.assertEqual(env_value("\"mixed'"), "\"mixed'")

    def test_a_comment_line_sets_nothing(self):
        self.assertIsNone(read("# PG_QT_DIR=C:/Qt"))


class LoadEnvFile(unittest.TestCase):
    """The repository's `.env` is read into the environment, except in the container (PG_CONTAINER, docker/Dockerfile)."""

    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        (Path(self.folder.name) / ".env").write_text("TEST_PG_KEY=from-file\n", encoding="utf-8")
        self.environ = mock.patch.dict(os.environ)
        self.environ.start()
        self.addCleanup(self.environ.stop)
        self.addCleanup(self.folder.cleanup)
        os.environ.pop("TEST_PG_KEY", None)
        os.environ.pop("PG_CONTAINER", None)
        self.repo = mock.patch.object(pgtools, "REPO", Path(self.folder.name))
        self.repo.start()
        self.addCleanup(self.repo.stop)

    def test_a_host_reads_the_file(self):
        load_env_file()
        self.assertEqual(os.environ.get("TEST_PG_KEY"), "from-file")

    def test_a_container_reads_no_file(self):
        os.environ["PG_CONTAINER"] = "1"
        load_env_file()
        self.assertNotIn("TEST_PG_KEY", os.environ)


class FunctionBody(unittest.TestCase):
    SOURCE = "fn before() {\n    1\n}\n\npub fn process(name: &str) -> u8 {\n    let x = 2;\n    x\n}\n\nfn after() {}\n"

    def test_the_body_runs_from_the_header_to_the_closing_brace(self):
        self.assertEqual(
            pgtools.function_body(self.SOURCE, "pub fn process("),
            "pub fn process(name: &str) -> u8 {\n    let x = 2;\n    x",
        )

    def test_a_drifted_header_raises(self):
        with self.assertRaises(ValueError):
            pgtools.function_body(self.SOURCE, "pub(crate) fn process(")

    def test_a_body_without_a_closing_brace_raises(self):
        with self.assertRaises(ValueError):
            pgtools.function_body("pub fn process() {\n    1\n", "pub fn process(")


class CatalogNameKey(unittest.TestCase):
    """The tools' `path_name_key` is the catalog's: the cases `pg_model::name`'s pinned test pins
    (`the_upcase_key_of_text_is_pinned`), read from the same generated table."""

    def test_windows_keys_are_the_pinned_ones(self):
        key = pgtools.catalog_name_key("windows")
        text = lambda s: key(s.encode("utf-8"))  # noqa: E731
        # Alpha, sigma and the dotless i by their code points: they look like Latin letters.
        alpha_sigma, dotless_i = chr(0x03B1) + chr(0x03C3), chr(0x0131)
        self.assertEqual(text("Plain.jpg"), b"PLAIN.JPG")
        self.assertEqual(text("Σίσυφος"), "ΣΊΣΥΦΟς".encode())
        self.assertEqual(text(alpha_sigma), text("ΑΣ"))
        self.assertNotEqual(text("ας"), text("ΑΣ"))
        self.assertEqual(text("İstanbul"), "İSTANBUL".encode())
        self.assertEqual(text(dotless_i + "i"), (dotless_i + "I").encode())
        self.assertEqual(text("ſµ"), "ſµ".encode())
        georgian_an = chr(0x10D0)
        self.assertEqual(text(georgian_an), georgian_an.encode(), "Georgian an: its Mtavruli capital is Unicode 11's")
        self.assertEqual(text("straße"), "STRAßE".encode())
        self.assertEqual(text("\U00010428"), "\U00010428".encode(), "supplementary: kept")
        self.assertEqual(text("a\x01b\x02c"), b"A\x01B\x02C")
        self.assertEqual(text("😀 x"), "😀 X".encode())
        self.assertEqual(key(b"a\xed\xa0\x80"), b"A\xed\xa0\x80", "an encoded lone surrogate is kept")
        self.assertEqual(key(b"a\xff"), b"A\xff", "a stray byte is kept")
        self.assertIsNone(key(None))

    def test_posix_keys_are_the_exact_bytes(self):
        key = pgtools.catalog_name_key("posix")
        self.assertEqual(key(b"Plain.jpg"), b"Plain.jpg")
        self.assertEqual(key("ας"), "ας".encode())


if __name__ == "__main__":
    unittest.main()
