"""Tests for tools/setup_ffmpeg.py: the checks that decide what an FFmpeg archive may unpack."""

import tempfile
import unittest
from pathlib import Path

from setup_ffmpeg import sha256_of, top_folder, unsafe_name


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
