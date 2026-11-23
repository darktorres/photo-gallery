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

from ci.eval_pins import PACKAGES, REQUIREMENTS, pin
from ci.github_files import env_line, path_line, pin_values
from install_exiftool import install
from pgtools import REPO, function_body, just_value

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
    "rustup_init_sha256": just_value("rustup_init_sha256"),
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

    def test_the_container_checks_rustup_init_against_the_justfile_pin(self):
        # The container's rustup step verifies the installer with the same SHA-256 the Justfile pins (and ci_setup.py reads).
        self.assertRegex(PINNED["rustup_init_sha256"], r"^[0-9a-f]{64}$")
        dockerfile = (REPO / "docker" / "Dockerfile").read_text(encoding="utf-8")
        self.assertIn(f"RUSTUP_INIT_SHA256={PINNED['rustup_init_sha256']}", dockerfile)
        self.assertIn("sha256sum -c", dockerfile)

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
    """The FFmpeg libraries the static build links (build_static_ffmpeg.py's link check) are the ones the two build scripts
    name: crates/video/build.rs opens them, crates/desktop/build.rs lists them. The lists are literals, so each is read
    from its text; a change to one list that the other misses fails here."""

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
        app = (REPO / "crates" / "desktop" / "build.rs").read_text(encoding="utf-8")
        app_list = re.search(r"STATIC_FFMPEG_LIBS: &\[&str\] = &\[([^\]]*)\]", app)
        assert app_list is not None, "crates/desktop/build.rs has no STATIC_FFMPEG_LIBS list"
        self.assertEqual(self.quoted(app_list.group(1)), expected, "crates/desktop/build.rs STATIC_FFMPEG_LIBS")
        video = (REPO / "crates" / "video" / "build.rs").read_text(encoding="utf-8")
        video_list = re.search(r"for lib in \[([^\]]*)\]", video)
        assert video_list is not None, "crates/video/build.rs has no `for lib in [...]` loop"
        self.assertEqual(self.quoted(video_list.group(1)), expected, "crates/video/build.rs the library loop")


ENV_REGISTRY = REPO / "crates" / "platform" / "src" / "env.rs"
ENV_EXAMPLE = REPO / ".env.example"
ENV_SOURCES = {".rs", ".cpp", ".h", ".hpp"}
PG_NAME = re.compile(r"\bPG_[A-Z][A-Z0-9_]*\b")
REGISTERED_SPEC = re.compile(r'Spec \{\s*name: "(PG_[A-Z0-9_]+)",\s*kind: Kind::\w+,\s*reader: Reader::(\w+),')
DOCUMENTED = re.compile(r"^#?\s*(PG_[A-Z][A-Z0-9_]*)=", re.MULTILINE)


def _family(reason, *names):
    """Each of `names` with the one `reason` that explains the family."""
    return dict.fromkeys(names, reason)


# The names the sources use that are not the app's variables, each exact and with its reason. A name is registered in
# crates/platform/src/env.rs (VARS), documented in .env.example, or one of these: a new `PG_PROBE_X` fails until it is
# listed here, and a listed name that no source uses any more fails the rot test below.
TEST_INPUTS = {
    **_family(
        "a child process's handshake keys (the catalog and dbfile holders)",
        "PG_HOLDER_PATH",
        "PG_HOLDER_MODE",
        "PG_HOLDER_READY",
        "PG_HOLDER_RELEASE",
    ),
    **_family(
        "the thumbnail cache's child-process keys",
        "PG_THUMB_HOLDER",
        "PG_THUMB_CATALOG",
        "PG_THUMB_READY",
        "PG_THUMB_RELEASE",
    ),
    **_family(
        "pg-platform's child-process keys, and a stdout marker of its test helper (PG_TESTING_HELD is not a variable)",
        "PG_PLATFORM_TESTING_CHILD_PREFIX",
        "PG_PLATFORM_SPAWN_HELPER",
        "PG_TESTING_HELD",
    ),
    **_family(
        "the examples' probe inputs, not the app's",
        "PG_PROBE_BETWEEN",
        "PG_PROBE_AGAIN",
        "PG_PROBE_WRITE",
        "PG_PROBE_WORKERS",
        "PG_PROBE_WIDTHS",
        "PG_PROBE_SLICE",
        "PG_PROBE_LOAD_PARTS",
        "PG_PROBE_KEEP_OWED",
        "PG_PROBE_GUARD",
        "PG_PROBE_FEW",
        "PG_PROBE_CACHE_KIB",
        "PG_PROBE_BLOCK",
        "PG_PROBE_MEMSTAT",
    ),
    **_family("the join test's dump inputs", "PG_JOIN_THREADS", "PG_JOIN_OUT", "PG_JOIN_DUMP"),
    **_family("a benchmark's output label", "PG_LABEL"),
    **_family(
        "a harness input, not a setting: the QML test's resource folder, read also by tools/qmltest.py and main.cpp",
        "PG_QML_RESOURCES",
    ),
    **_family(
        "the input a test or a rehearsal reads",
        "PG_TEST_CATALOG",
        "PG_REHEARSAL_CATALOG",
        "PG_JPEG_COMPARE_DIR",
    ),
}


def env_sources():
    """{name: [file, ...]} for every PG_* name in the sources under crates/."""
    found = {}
    for path in sorted((REPO / "crates").rglob("*")):
        if path.suffix not in ENV_SOURCES or not path.is_file():
            continue
        text = path.read_text(encoding="utf-8")
        for name in set(PG_NAME.findall(text)):
            found.setdefault(name, []).append(path.relative_to(REPO).as_posix())
    return found


class EnvironmentRegistry(unittest.TestCase):
    def setUp(self):
        self.registry = dict(REGISTERED_SPEC.findall(ENV_REGISTRY.read_text(encoding="utf-8")))
        self.documented = set(DOCUMENTED.findall(ENV_EXAMPLE.read_text(encoding="utf-8")))

    def test_the_registry_is_read_from_the_source(self):
        self.assertIn("PG_HOME", self.registry)
        self.assertIn("PG_FORCE_AV1", self.registry)

    def test_every_name_the_sources_use_is_registered_documented_or_a_test_input(self):
        unknown = {}
        for name, files in env_sources().items():
            if name in self.registry or name in self.documented:
                continue
            if name in TEST_INPUTS:
                continue
            unknown[name] = sorted(set(files))
        self.assertEqual(unknown, {}, "add to VARS in crates/platform/src/env.rs, or to .env.example, or to TEST_INPUTS")

    def test_every_test_input_is_still_used_by_a_source(self):
        # An exact list rots the way a pattern cannot: a name whose last use is gone must leave the list.
        sources = env_sources()
        unused = sorted(name for name in TEST_INPUTS if name not in sources)
        self.assertEqual(unused, [], "these TEST_INPUTS names no source uses any more: remove them")

    def test_every_name_a_build_script_uses_is_registered_for_the_build(self):
        # boundaries.py exempts a build script from the environment rule by its path, so the names it uses are checked
        # here instead. Its `//` comments name variables too (desktop/build.rs, video/build.rs), so they are dropped first.
        unregistered = {}
        for path in sorted((REPO / "crates").glob("*/build.rs")):
            code = re.sub(r"//[^\n]*", "", path.read_text(encoding="utf-8"))
            for name in sorted(set(PG_NAME.findall(code))):
                if self.registry.get(name) != "Build":
                    unregistered.setdefault(name, []).append(path.relative_to(REPO).as_posix())
        self.assertEqual(unregistered, {}, "register these with Reader::Build in crates/platform/src/env.rs (VARS)")

    def test_every_variable_the_app_or_native_code_reads_is_documented(self):
        missing = [name for name, reader in self.registry.items() if reader in ("App", "Native") and name not in self.documented]
        self.assertEqual(missing, [], "document these in .env.example (a commented PG_NAME=value line with its meaning)")

    def test_the_cpp_flag_spellings_are_the_rust_spellings(self):
        rust = set(re.findall(r'"([^"]+)"', function_body(ENV_REGISTRY.read_text(encoding="utf-8"), "pub fn parse(")))
        cpp = set(
            re.findall(
                r'u"([^"]+)"',
                function_body(
                    (REPO / "crates" / "desktop" / "cpp" / "gpu_device.cpp").read_text(encoding="utf-8"),
                    "bool env_flag(",
                ),
            )
        )
        self.assertEqual(cpp, rust, "the C++ desktop reads its flags with the same spellings as pg_platform::env::parse")


# The binaries that keep anyhow for good. Each is the top of its stack: a failure is reported to the user as a message and
# an exit code, with context added on the way up, so a typed error there would only be matched to choose a message.
# (`desktop` also matches one typed error by downcast, `pg_app::Error`, and says so where it does.) Every library crate is
# listed in anyhow-allow.txt until its migration removes it (Phase 7). `gallery` has no anyhow dependency and is not here.
ANYHOW_PERMANENT = {"cli", "desktop"}
ANYHOW_ALLOW = REPO / "tools" / "anyhow-allow.txt"


def depends_on_anyhow(manifest):
    """Whether the crate's manifest names anyhow in any dependency table: normal, dev, build, or a target-specific one."""
    tables = [manifest, *manifest.get("target", {}).values()]
    kinds = ("dependencies", "dev-dependencies", "build-dependencies")
    return any("anyhow" in table.get(kind, {}) for table in tables for kind in kinds)


def permanent_without_anyhow(permanent, depending):
    """The permanent crates that no longer depend on anyhow: a stale entry in ANYHOW_PERMANENT."""
    return set(permanent) - set(depending)


class AnyhowAllowlist(unittest.TestCase):
    def crates_depending(self):
        found = set()
        for manifest_path in sorted((REPO / "crates").glob("*/Cargo.toml")):
            manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
            if depends_on_anyhow(manifest):
                found.add(manifest_path.parent.name)
        return found

    def listed(self):
        lines = ANYHOW_ALLOW.read_text(encoding="utf-8").splitlines()
        return {line.strip() for line in lines if line.strip() and not line.lstrip().startswith("#")}

    def test_every_crate_that_uses_anyhow_is_listed_or_permanent(self):
        unlisted = self.crates_depending() - self.listed() - ANYHOW_PERMANENT
        self.assertEqual(
            unlisted, set(), "these crates depend on anyhow: move them to typed errors, or list them in anyhow-allow.txt"
        )

    def test_every_listed_crate_still_depends_on_anyhow(self):
        stale = self.listed() - self.crates_depending()
        self.assertEqual(stale, set(), "remove these from anyhow-allow.txt: they no longer depend on anyhow")

    def test_the_permanent_crates_are_crates(self):
        present = {path.parent.name for path in (REPO / "crates").glob("*/Cargo.toml")}
        self.assertLessEqual(ANYHOW_PERMANENT, present)

    def test_every_permanent_crate_still_depends_on_anyhow(self):
        self.assertEqual(
            permanent_without_anyhow(ANYHOW_PERMANENT, self.crates_depending()),
            set(),
            "these permanent crates no longer depend on anyhow: take them out of ANYHOW_PERMANENT",
        )

    def test_a_permanent_crate_that_drops_anyhow_is_reported(self):
        self.assertEqual(permanent_without_anyhow({"cli", "desktop"}, {"cli"}), {"desktop"})
        self.assertEqual(permanent_without_anyhow({"cli"}, {"cli", "desktop"}), set())

    def test_a_manifest_check_sees_each_table_kind(self):
        self.assertTrue(depends_on_anyhow({"dependencies": {"anyhow": {"workspace": True}}}))
        self.assertTrue(depends_on_anyhow({"dev-dependencies": {"anyhow": {"workspace": True}}}))
        self.assertTrue(depends_on_anyhow({"build-dependencies": {"anyhow": "1"}}))
        self.assertTrue(depends_on_anyhow({"target": {"cfg(unix)": {"dependencies": {"anyhow": "1"}}}}))
        self.assertFalse(depends_on_anyhow({"dependencies": {"thiserror": "2"}}))


# The crates whose code holds unsafe: each unsafe item says why with `#[expect(unsafe_code, reason)]`, and every other
# crate root forbids it (CU-005). `desktop` is the cxx-qt bridge crate, `platform` the system-call wrappers.
UNSAFE_CRATES = {"dedup", "desktop", "platform", "trace", "video"}
FORBID = "#![forbid(unsafe_code)]"


class UnsafePolicy(unittest.TestCase):
    def roots(self):
        found = {}
        for directory in sorted((REPO / "crates").glob("*/")):
            for name in ("src/lib.rs", "src/main.rs"):
                root = directory / name
                if root.exists():
                    found[directory.name] = root.read_text(encoding="utf-8")
                    break
        return found

    def test_every_crate_without_unsafe_forbids_it(self):
        missing = {name for name, text in self.roots().items() if name not in UNSAFE_CRATES and FORBID not in text}
        self.assertEqual(missing, set(), "these crate roots must say #![forbid(unsafe_code)] (CU-005)")

    def test_the_unsafe_crates_do_not_forbid_it(self):
        forbidding = {name for name, text in self.roots().items() if name in UNSAFE_CRATES and FORBID in text}
        self.assertEqual(forbidding, set(), "these crates hold unsafe code, so they cannot forbid it")

    def test_the_workspace_denies_unsafe_code(self):
        manifest = tomllib.loads((REPO / "Cargo.toml").read_text(encoding="utf-8"))
        self.assertEqual(manifest["workspace"]["lints"]["rust"].get("unsafe_code"), "deny")


class AnalysisScaleRule(unittest.TestCase):
    """The analyze scale rule (the long side and the short side a decode keeps) is written three times: the app's
    DECODE_MIN_* constants, the eval decoder's copy (its own workspace cannot import the app's crates) and the Python
    eval's ANALYSIS_DRAFT_SIDES. The three must agree."""

    def test_the_three_copies_agree(self):
        analyze = (REPO / "crates" / "analysis" / "src" / "analyze.rs").read_text(encoding="utf-8")
        decoder = (REPO / "eval" / "jpeg-decoders" / "src" / "lib.rs").read_text(encoding="utf-8")
        common = (REPO / "eval" / "common.py").read_text(encoding="utf-8")
        app = (
            int(_const(analyze, "DECODE_MIN_LONG")),
            int(_const(analyze, "DECODE_MIN_SHORT")),
        )
        eval_decoder = (
            int(_const(decoder, "ANALYZE_MIN_LONG")),
            int(_const(decoder, "ANALYZE_MIN_SHORT")),
        )
        match = re.search(r"^ANALYSIS_DRAFT_SIDES = \((\d+), (\d+)\)$", common, re.MULTILINE)
        if match is None:
            self.fail("eval/common.py has no ANALYSIS_DRAFT_SIDES pair")
        python = (int(match.group(1)), int(match.group(2)))
        self.assertEqual(eval_decoder, app, "eval/jpeg-decoders disagrees with the app's analyze minimum")
        self.assertEqual(python, app, "eval/common.py disagrees with the app's analyze minimum")


class VerifyCommonIsTorchFree(unittest.TestCase):
    """The helpers a tool shares with the verifier load no torch: the modules they import from the eval folder do not
    import torch or kornia either, so a tool that needs only these helpers does not pay for them."""

    def test_no_module_it_imports_names_torch_or_kornia(self):
        eval_dir = REPO / "eval"
        for name in ("verify_common.py", "verify_types.py", "common.py"):
            text = (eval_dir / name).read_text(encoding="utf-8")
            for line in text.splitlines():
                stripped = line.strip()
                for heavy in ("torch", "kornia"):
                    self.assertFalse(
                        stripped.startswith((f"import {heavy}", f"from {heavy}")),
                        f"{name} imports {heavy}: {stripped}",
                    )


def _const(text: str, name: str) -> str:
    """The value of a `const NAME: usize = N;` declaration in Rust `text`."""
    match = re.search(rf"^(?:pub )?const {name}: usize = (\d+);", text, re.MULTILINE)
    if match is None:
        raise AssertionError(f"no const {name}")
    return match.group(1)


if __name__ == "__main__":
    unittest.main()
