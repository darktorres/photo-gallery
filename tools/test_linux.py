"""Tests of tools/linux.py's check of the host folders it mounts against Docker Desktop's shared folders."""

import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import linux


class MountSources(unittest.TestCase):
    def test_only_drive_paths_are_host_folders(self):
        mounts = ["-v", "D:/GitHub/photo-gallery:/work", "-v", "pg-cargo-target:/cargo-target", "-v", "C:/x:/y:ro"]
        self.assertEqual(linux.mount_sources(mounts), ["D:/GitHub/photo-gallery", "C:/x"])


@unittest.skipUnless(sys.platform == "win32", "Docker Desktop's shared folders are a Windows host's")
class Unshared(unittest.TestCase):
    shared = (r"D:\GitHub\photo-gallery", r"D:\PhotoGallery\bench-data")

    def test_a_folder_under_a_shared_one_is_shared_whatever_the_case_or_separator(self):
        self.assertEqual(linux.unshared(["d:/github/photo-gallery/target/x", "D:/GitHub/photo-gallery"], self.shared), [])

    def test_a_folder_outside_them_or_with_a_shared_prefix_only_is_not(self):
        sources = ["D:/PhotoGallery/cache/ann", "D:/GitHub/photo-gallery-old"]
        self.assertEqual(linux.unshared(sources, self.shared), sources)

    def test_the_settings_file_gives_the_list_and_its_absence_checks_nothing(self):
        with tempfile.TemporaryDirectory() as appdata, mock.patch.dict(os.environ, {"APPDATA": appdata}):
            self.assertIsNone(linux.docker_desktop_shared())
            (Path(appdata) / "Docker").mkdir()
            settings = Path(appdata) / "Docker" / "settings-store.json"
            settings.write_text(json.dumps({"FilesharingDirectories": list(self.shared)}), encoding="utf-8")
            self.assertEqual(linux.docker_desktop_shared(), list(self.shared))
            with self.assertRaises(SystemExit):
                linux.shared_or_fail(["-v", "D:/PhotoGallery/cache/ann:/ann/0"])
            linux.shared_or_fail(["-v", "D:/GitHub/photo-gallery:/work"])


if __name__ == "__main__":
    unittest.main()
