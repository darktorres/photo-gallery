"""Tests of the CI helpers (tools/ci): the GITHUB_ENV and GITHUB_PATH lines, the eval pins, and the pins: the Justfile is the
one place a version is written, so CI's workflow holds no pinned literal and every consumer agrees with the Justfile."""

import hashlib
import os
import re
import shutil
import sys
import tarfile
import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest import mock

from build_static_ffmpeg import MSYS_PACKAGES, WINDOWS_SDK
from ci.eval_pins import PACKAGES, REQUIREMENTS, pin
from ci.github_files import env_line, path_line, pin_values
from install_exiftool import install
from pgtools import DEFAULT_BUILD_TOOLS, DEFAULT_MSYS, MSVC_TOOLSET, REPO, just_value

WORKFLOW = (REPO / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
PINNED = {
    "qt_version": just_value("qt_version"),
    "just_version": just_value("just_version"),
    "cargo_deny_version": just_value("cargo_deny_version"),
    "cargo_machete_version": just_value("cargo_machete_version"),
    "cargo_modules_version": just_value("cargo_modules_version"),
    "aqtinstall_commit": just_value("aqtinstall_commit"),
    "exiftool_version": just_value("exiftool_version"),
    "exiftool_source_sha256": just_value("exiftool_source_sha256"),
    "ffmpeg_sha256": just_value("ffmpeg_sha256"),
}


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


class EvalPins(unittest.TestCase):
    def test_finds_each_pin_among_the_other_lines(self):
        text = "torch==2.8.0\nnumpy==2.5.5\npillow==13.0.0\n"
        self.assertEqual(pin(text, "numpy"), "numpy==2.5.5")
        self.assertEqual(pin(text, "pillow"), "pillow==13.0.0")

    def test_no_pin_is_none(self):
        self.assertIsNone(pin("pillow==13.0.0\n", "numpy"))

    def test_another_package_whose_name_starts_with_it_is_not_it(self):
        self.assertIsNone(pin("numpy-typing==1.0\n", "numpy"))

    def test_the_name_is_case_blind(self):
        self.assertEqual(pin("Pillow==13.0.0\n", "pillow"), "Pillow==13.0.0")

    def test_the_eval_requirements_pin_every_package_ci_installs(self):
        text = REQUIREMENTS.read_text(encoding="utf-8")
        for package in PACKAGES:
            self.assertIsNotNone(pin(text, package), package)

    def test_the_shots_requirements_agree_with_the_eval_pins(self):
        eval_text = REQUIREMENTS.read_text(encoding="utf-8")
        shots = (REPO / "tools" / "shots" / "requirements.txt").read_text(encoding="utf-8")
        for package in PACKAGES:
            self.assertEqual(pin(shots, package), pin(eval_text, package), package)


class JustValue(unittest.TestCase):
    def justfile(self, text):
        """Patches the repository folder to one holding a Justfile with TEXT."""
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        Path(folder.name, "Justfile").write_text(text, encoding="utf-8")
        patch = mock.patch("pgtools.REPO", Path(folder.name))
        patch.start()
        self.addCleanup(patch.stop)

    def test_reads_the_literal_line(self):
        self.justfile('qt_version := "6.12.0"  # the kit\n')
        self.assertEqual(just_value("qt_version"), "6.12.0")

    def test_a_longer_name_is_not_the_one(self):
        self.justfile('qt_version_extra := "1"\nqt_version := "6.12.0"\n')
        self.assertEqual(just_value("qt_version"), "6.12.0")

    def test_another_right_hand_side_is_refused(self):
        self.justfile('qt := env("QT")\n')
        with self.assertRaises(SystemExit):
            just_value("qt")

    def test_a_second_assignment_is_refused(self):
        self.justfile('qt := "6.12.0"\nqt := "6.13.0"\n')
        with self.assertRaises(SystemExit):
            just_value("qt")


class Pins(unittest.TestCase):
    def test_the_justfile_pins_are_what_their_names_say(self):
        self.assertRegex(PINNED["qt_version"], r"^\d+\.\d+\.\d+$")
        self.assertRegex(PINNED["aqtinstall_commit"], r"^[0-9a-f]{40}$")

    def test_the_windows_kit_is_under_the_drive_root(self):
        with mock.patch.object(sys, "platform", "win32"):
            values = dict(pin_values())
        self.assertEqual(values["PG_QT_DIR"], f"C:\\Qt\\{PINNED['qt_version']}\\msvc2022_64")
        self.assertEqual(values["QT_INSTALL_ROOT"], "C:\\")
        self.assertNotIn("QMAKE", values)

    def test_the_ffmpeg_checksum_is_exported_as_the_cache_key(self):
        # The FFmpeg cache's key is this value (ci.yml), so a new pin is a new cache.
        self.assertRegex(PINNED["ffmpeg_sha256"], r"^[0-9a-f]{64}$")
        self.assertEqual(dict(pin_values())["FFMPEG_SHA256"], PINNED["ffmpeg_sha256"])

    def test_the_linux_kit_is_under_the_runner_temp_folder(self):
        qt = PINNED["qt_version"]
        runner = "/work/_runner"
        with mock.patch.object(sys, "platform", "linux"), mock.patch.dict(os.environ, {"RUNNER_TEMP": runner}):
            values = dict(pin_values())
        kit = f"{runner}/Qt/{qt}/gcc_64"
        self.assertEqual(values["PG_QT_DIR"], kit)
        self.assertEqual(values["QMAKE"], f"{kit}/bin/qmake")
        self.assertEqual(values["LD_LIBRARY_PATH"], f"{kit}/lib")
        self.assertEqual(values["EXIFTOOL_SOURCE_SHA256"], PINNED["exiftool_source_sha256"])
        self.assertEqual(values["EXIFTOOL_SOURCE_VERSION"], just_value("exiftool_source_version"))
        self.assertTrue(values["EXIFTOOL_SOURCE_URL"].startswith("https://"))
        self.assertEqual(
            values["TAIKI_TOOLS"],
            f"just@{PINNED['just_version']},cargo-deny@{PINNED['cargo_deny_version']},"
            f"cargo-machete@{PINNED['cargo_machete_version']},cargo-modules@{PINNED['cargo_modules_version']}",
        )


@unittest.skipIf(sys.platform == "win32", "the fake ExifTool is a shell script")
class InstallExifTool(unittest.TestCase):
    """The install script on a local archive (no network): the checksum is refused before unpacking, and the version the
    program reports is checked."""

    def archive(self, version: str) -> tuple[str, str]:
        """A release-shaped archive (Image-ExifTool-<version>/exiftool and lib/) and its URL and SHA-256."""
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        source = Path(folder.name, f"Image-ExifTool-{version}")
        (source / "lib" / "Image").mkdir(parents=True)
        (source / "lib" / "Image" / "ExifTool.pm").write_text("1;\n", encoding="utf-8")
        program = source / "exiftool"
        program.write_text(f"#!/bin/sh\necho {version}\n", encoding="utf-8")
        program.chmod(0o755)
        archive = Path(folder.name, "release.tar.gz")
        with tarfile.open(archive, "w:gz") as tar:
            tar.add(source, arcname=source.name)
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        return archive.as_uri(), digest

    def test_installs_the_program_and_its_lib_when_the_checksum_and_version_match(self):
        url, digest = self.archive("13.59")
        prefix = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, prefix, True)
        install(url, digest, "13.59", prefix)
        self.assertTrue((prefix / "lib" / "Image" / "ExifTool.pm").is_file())
        self.assertTrue(os.access(prefix / "exiftool", os.X_OK))

    def test_a_checksum_that_does_not_match_is_refused_before_anything_is_installed(self):
        url, _ = self.archive("13.59")
        prefix = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, prefix, True)
        with self.assertRaises(SystemExit):
            install(url, "0" * 64, "13.59", prefix)
        self.assertFalse((prefix / "exiftool").exists())

    def test_a_program_of_another_version_is_refused(self):
        url, digest = self.archive("13.58")
        prefix = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, prefix, True)
        with self.assertRaises(SystemExit):
            install(url, digest, "13.59", prefix)


class WorkflowPins(unittest.TestCase):
    def test_each_job_runs_cargo_deny_as_its_own_step(self):
        # `just lint` leaves cargo-deny out (its advisory fetch is a network call); each job runs `just deny` in its own step.
        self.assertEqual(WORKFLOW.count("run: |\n          for attempt in 1 2 3; do\n            if just deny"), 2)

    def test_the_workflow_holds_no_pinned_value(self):
        for name, value in PINNED.items():
            self.assertNotIn(value, WORKFLOW, f"{name} is written in ci.yml: read it from the pins step")

    def test_each_job_runs_the_pins_step_before_its_first_use(self):
        body = WORKFLOW.split("\njobs:\n", 1)[1]
        parts = re.split(r"^  ([\w-]+):\n", body, flags=re.MULTILINE)[1:]
        jobs = dict(zip(parts[0::2], parts[1::2], strict=True))
        self.assertEqual(sorted(jobs), ["linux", "windows"])
        for job, text in jobs.items():
            pins = text.find("github_files.py pins")
            self.assertNotEqual(pins, -1, job)
            uses = [i for i in (text.find(use) for use in ("env.QT_", "env.TAIKI_TOOLS", "env.AQTINSTALL_COMMIT")) if i != -1]
            self.assertLess(pins, min(uses), job)
            if "runs-on: windows" in text:
                self.assertLess(text.find("actions/setup-python"), pins, job)


class ClippyFiles(unittest.TestCase):
    def test_the_crates_repeat_the_root_test_allowances_and_bans(self):
        # Clippy reads a crate's own clippy.toml instead of the root's, and has no include, so the model and client files
        # repeat the root's settings that apply everywhere. This keeps the repeats in step with the root.
        root = tomllib.loads((REPO / "clippy.toml").read_text(encoding="utf-8"))
        root_types = {entry["path"] for entry in root["disallowed-types"]}
        for crate in ("model", "client"):
            with self.subTest(crate=crate):
                own = tomllib.loads((REPO / "crates" / crate / "clippy.toml").read_text(encoding="utf-8"))
                for key in (
                    "allow-panic-in-tests",
                    "allow-print-in-tests",
                    "allow-unwrap-in-tests",
                    "allow-expect-in-tests",
                    "allow-indexing-slicing-in-tests",
                ):
                    self.assertEqual(own.get(key), root.get(key), key)
                own_types = {entry["path"] for entry in own["disallowed-types"]}
                self.assertLessEqual(root_types, own_types, "every root hash-collection ban is in the crate's list")
                own_methods = {entry["path"] for entry in own["disallowed-methods"]}
                self.assertIn("std::env::temp_dir", own_methods, "the root temp_dir ban is in the crate's list")


class Consumers(unittest.TestCase):
    def test_the_powershell_copies_agree_on_the_msvc_toolset(self):
        for script in ("build-jpeg-turbo.ps1", "build-static-ffmpeg.ps1"):
            text = (REPO / "tools" / script).read_text(encoding="utf-8")
            self.assertIn(f"'{MSVC_TOOLSET}'", text, script)

    def test_the_powershell_copies_agree_on_the_static_build_folders_and_pins(self):
        ffmpeg = (REPO / "tools" / "build-static-ffmpeg.ps1").read_text(encoding="utf-8")
        jpeg = (REPO / "tools" / "build-jpeg-turbo.ps1").read_text(encoding="utf-8")
        for text in (ffmpeg, jpeg):
            self.assertIn(f"'{DEFAULT_BUILD_TOOLS}'", text)
        self.assertIn(f"'{DEFAULT_MSYS}'", ffmpeg)
        self.assertIn(f"'{WINDOWS_SDK}'", ffmpeg)
        for name, version in MSYS_PACKAGES.items():
            self.assertIn(f"{name} = '{version}'", ffmpeg)

    def test_the_workflow_python_is_the_justfile_pin(self):
        # setup-python runs before the pins step, so the workflow holds the literal; this asserts it is the pinned one.
        self.assertIn(f'python-version: "{just_value("python_version")}"', WORKFLOW)

    def test_the_static_qt_source_is_the_pinned_version(self):
        text = (REPO / "tools" / "build_static_qt.py").read_text(encoding="utf-8")
        self.assertEqual(re.search(r'^QT_VERSION = "([^"]+)"', text, re.MULTILINE).group(1), PINNED["qt_version"])

    def test_the_qml_host_and_the_env_example_name_the_pinned_kit(self):
        minor = ".".join(PINNED["qt_version"].split(".")[:2])
        cmake = (REPO / "crates" / "desktop" / "tests" / "qml_host" / "CMakeLists.txt").read_text(encoding="utf-8")
        self.assertIn(f"find_package(Qt6 {minor} ", cmake)
        example = (REPO / ".env.example").read_text(encoding="utf-8")
        self.assertIn(f"Qt/{PINNED['qt_version']}/msvc2022_64", example)


class StaticFfmpegLibraries(unittest.TestCase):
    """The FFmpeg libraries the static build links (build_static_ffmpeg.py's link check) are the ones the two Qt patches
    name: pg-video.patch opens them, app-build.patch lists them. The patches' lists are literals, so each is read from its
    text; a change to one list that the others miss fails here."""

    @staticmethod
    def quoted(text: str) -> list[str]:
        return sorted(name.removesuffix(".lib") for name in re.findall(r'"([^"]+)"', text))

    def test_the_patches_name_the_libraries_the_link_check_names(self):
        build = (REPO / "tools" / "build_static_ffmpeg.py").read_text(encoding="utf-8")
        link = re.search(r"libraries = \[([^\]]*)\]", build)
        assert link is not None, "build_static_ffmpeg.py has no `libraries = [...]` list"
        expected = self.quoted(
            link.group(1)
        )  # the literal names; SYSTEM_LIBS (*SYSTEM_LIBS) are not FFmpeg's and are not quoted here
        self.assertEqual(len(expected), 5)
        app = (REPO / "tools" / "static-qt" / "app-build.patch").read_text(encoding="utf-8")
        app_list = re.search(r"STATIC_FFMPEG_LIBS: &\[&str\] = &\[([^\]]*)\]", app)
        assert app_list is not None, "app-build.patch has no STATIC_FFMPEG_LIBS list"
        self.assertEqual(self.quoted(app_list.group(1)), expected, "app-build.patch STATIC_FFMPEG_LIBS")
        video = (REPO / "tools" / "static-qt" / "pg-video.patch").read_text(encoding="utf-8")
        video_list = re.search(r"for lib in \[([^\]]*)\]", video)
        assert video_list is not None, "pg-video.patch has no `for lib in [...]` loop"
        self.assertEqual(self.quoted(video_list.group(1)), expected, "pg-video.patch the library loop")


if __name__ == "__main__":
    unittest.main()
