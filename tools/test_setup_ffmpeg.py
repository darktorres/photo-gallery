"""Tests for tools/setup_ffmpeg.py: the checks that decide what an FFmpeg archive may unpack."""

import tempfile
import unittest
import urllib.error
from email.message import Message
from pathlib import Path
from unittest import mock

from setup_ffmpeg import HEADER, PIN_FILE, download, installed_pin, is_installed, sha256_of, top_folder, unsafe_name

PIN = "1d9823c35c7a7e2f54d1438cf61030d6efabe3278de01c191309d1e828bb74df"
OTHER = "08f4d6542510d35710631e1475e72f1a2e731700f35a701c84d7479893a7aa96"


def build_at(folder: Path, pin: str | None) -> Path:
    """A folder holding a build's header, recording `pin` when it is not None."""
    (folder / HEADER).parent.mkdir(parents=True, exist_ok=True)
    (folder / HEADER).write_text("/* header */", encoding="utf-8")
    if pin is not None:
        (folder / PIN_FILE).write_text(pin + "\n", encoding="utf-8")
    return folder


class PinRecord(unittest.TestCase):
    def test_a_build_at_the_pin_is_installed(self):
        with tempfile.TemporaryDirectory() as folder:
            self.assertTrue(is_installed(build_at(Path(folder), PIN), PIN))

    def test_a_build_at_another_pin_is_not_installed(self):
        with tempfile.TemporaryDirectory() as folder:
            self.assertFalse(is_installed(build_at(Path(folder), OTHER), PIN))

    def test_a_build_without_a_record_is_not_installed(self):
        with tempfile.TemporaryDirectory() as folder:
            dest = build_at(Path(folder), None)
            self.assertIsNone(installed_pin(dest))
            self.assertFalse(is_installed(dest, PIN))

    def test_no_build_is_not_installed(self):
        with tempfile.TemporaryDirectory() as folder:
            self.assertFalse(is_installed(Path(folder) / "ffmpeg", PIN))


class NotFound(unittest.TestCase):
    def test_a_404_names_the_pin_and_the_month_end_rule(self):
        error = urllib.error.HTTPError("https://example.invalid/x.zip", 404, "Not Found", Message(), None)
        with (
            tempfile.TemporaryDirectory() as folder,
            mock.patch("setup_ffmpeg.urllib.request.urlopen", side_effect=error),
            self.assertRaises(SystemExit) as caught,
        ):
            download("https://example.invalid/x.zip", Path(folder) / "x.zip")
        message = str(caught.exception.code)
        self.assertIn("(404)", message)
        self.assertIn("month-end", message)

    def test_another_http_error_is_raised_as_it_is(self):
        error = urllib.error.HTTPError("https://example.invalid/x.zip", 500, "Server Error", Message(), None)
        with (
            tempfile.TemporaryDirectory() as folder,
            mock.patch("setup_ffmpeg.urllib.request.urlopen", side_effect=error),
            self.assertRaises(urllib.error.HTTPError),
        ):
            download("https://example.invalid/x.zip", Path(folder) / "x.zip")


class ArchiveChecks(unittest.TestCase):
    def test_an_entry_that_leaves_the_folder_is_unsafe(self):
        for name in ["../x", "a/../../x", "/etc/x", "C:/x", "a\\..\\x"]:
            self.assertTrue(unsafe_name(name), name)

    def test_entries_inside_the_folder_are_safe(self):
        for name in ["ffmpeg/", "ffmpeg/include/libavcodec/avcodec.h", "ffmpeg/bin/avcodec-62.dll"]:
            self.assertFalse(unsafe_name(name), name)

    def test_an_archive_with_one_top_folder_gives_it(self):
        self.assertEqual(top_folder(["ffmpeg/", "ffmpeg/include/a.h", "ffmpeg/bin/a.dll"]), "ffmpeg")

    def test_an_archive_with_two_top_folders_is_refused(self):
        with self.assertRaises(ValueError):
            top_folder(["a/x", "b/y"])

    def test_the_sha256_is_the_hex_digest_of_the_file(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "abc.bin"
            path.write_bytes(b"abc")
            self.assertEqual(sha256_of(path), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")


if __name__ == "__main__":
    unittest.main()
