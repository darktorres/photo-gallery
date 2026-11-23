"""Tests of fork-updates.py: the fork crates each lock file names, the merge across lock files, the release check (a 404 is
not a read failure, each crate is fetched once), and the report. No network: the fetch is a fake, and urlopen is mocked."""

import email.message
import importlib.util
import os
import tempfile
import unittest
import urllib.error
from pathlib import Path
from unittest import mock

SCRIPT = Path(__file__).resolve().parent / "fork-updates.py"
spec = importlib.util.spec_from_file_location("fork_updates", SCRIPT)
if spec is None or spec.loader is None:
    raise ImportError(f"cannot load {SCRIPT}")
fork_updates = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fork_updates)

FORK = "git+https://github.com/darktorres/fast_image_resize?rev=0ea70891"
REGISTRY = "git+registry+https://github.com/rust-lang/crates.io-index"
# One fork crate in the root lock, and one that crates.io never published (parajpeg).
ROWS = [
    ("fast_image_resize", "6.1.0", "darktorres/fast_image_resize", ["Cargo.lock"]),
    ("parajpeg", "0.1.0", "darktorres/parajpeg", ["Cargo.lock"]),
]


def lock_text(*packages):
    """A Cargo.lock with the given (name, version, source or None) packages."""
    blocks = []
    for name, version, source in packages:
        block = f'[[package]]\nname = "{name}"\nversion = "{version}"\n'
        if source:
            block += f'source = "{source}"\n'
        blocks.append(block)
    return "\n".join(blocks)


class ForkedCrates(unittest.TestCase):
    def test_only_the_fork_sources_are_listed(self):
        with tempfile.TemporaryDirectory() as folder:
            lock = Path(folder) / "Cargo.lock"
            lock.write_text(lock_text(("serde", "1.0.0", REGISTRY), ("fast_image_resize", "6.1.0", FORK)), encoding="utf-8")
            self.assertEqual(
                fork_updates.forked_crates(lock),
                [("fast_image_resize", "6.1.0", "darktorres/fast_image_resize")],
            )


class Collect(unittest.TestCase):
    def test_a_relative_lock_path_is_named_from_the_repository_root(self):
        # `--lock fuzz/Cargo.lock` is relative to the working directory; it must still be named as the repository has it.
        with tempfile.TemporaryDirectory() as folder:
            lock = Path(folder) / "fuzz" / "Cargo.lock"
            lock.parent.mkdir(parents=True)
            lock.write_text(lock_text(("fast_image_resize", "6.2.0", FORK)), encoding="utf-8")
            here = Path.cwd()
            try:
                os.chdir(folder)
                with mock.patch.object(fork_updates, "ROOT", Path(folder).resolve()):
                    rows = fork_updates.collect([Path("fuzz/Cargo.lock")])
            finally:
                os.chdir(here)
            self.assertEqual(rows[0][3], ["fuzz/Cargo.lock"])

    def test_a_crate_in_two_lock_files_is_one_row_naming_both(self):
        with tempfile.TemporaryDirectory() as folder:
            root_lock = Path(folder) / "root" / "Cargo.lock"
            fuzz_lock = Path(folder) / "fuzz" / "Cargo.lock"
            for path in (root_lock, fuzz_lock):
                path.parent.mkdir(parents=True)
                path.write_text(lock_text(("fast_image_resize", "6.1.0", FORK)), encoding="utf-8")
            # collect names the lock files relative to the repository root, so use the real root's paths.
            with mock.patch.object(fork_updates, "ROOT", Path(folder)):
                rows = fork_updates.collect([root_lock, fuzz_lock])
            self.assertEqual(len(rows), 1)
            crate, locked, repository, locks = rows[0]
            self.assertEqual((crate, locked, repository), ("fast_image_resize", "6.1.0", "darktorres/fast_image_resize"))
            self.assertEqual(locks, ["root/Cargo.lock", "fuzz/Cargo.lock"])

    def test_a_crate_at_two_versions_is_two_rows(self):
        with tempfile.TemporaryDirectory() as folder:
            lock = Path(folder) / "Cargo.lock"
            lock.write_text(lock_text(("png", "0.18.1", FORK), ("png", "0.16.0", FORK)), encoding="utf-8")
            with mock.patch.object(fork_updates, "ROOT", Path(folder)):
                rows = fork_updates.collect([lock])
            self.assertEqual([row[1] for row in rows], ["0.16.0", "0.18.1"])


class ReleaseCheck(unittest.TestCase):
    def test_a_newer_release_is_listed_as_newer(self):
        rows = fork_updates.check(ROWS[:1], lambda _crate: "6.2.0")
        self.assertEqual(rows[0]["status"], "NEWER")

    def test_the_locked_release_is_current_when_nothing_is_newer(self):
        rows = fork_updates.check(ROWS[:1], lambda _crate: "6.1.0")
        self.assertEqual(rows[0]["status"], "current")

    def test_a_crate_crates_io_never_published_is_not_a_read_failure(self):
        def fetch(crate):
            raise fork_updates.NotPublishedError(crate)

        rows = fork_updates.check(ROWS[1:], fetch)
        self.assertEqual(rows[0]["status"], "NOT PUBLISHED")

    def test_a_failed_read_is_unreadable(self):
        def fetch(_crate):
            raise OSError("no network")

        rows = fork_updates.check(ROWS[:1], fetch)
        self.assertTrue(rows[0]["status"].startswith("UNREADABLE"))

    def test_each_crate_is_fetched_once_however_many_rows_name_it(self):
        calls = []

        def fetch(crate):
            calls.append(crate)
            return "6.1.0"

        rows_in = [
            ("png", "0.18.1", "darktorres/image-png", ["Cargo.lock"]),
            ("png", "0.18.1", "darktorres/image-png", ["fuzz/Cargo.lock"]),
        ]
        with mock.patch.object(fork_updates.time, "sleep"):
            fork_updates.check(rows_in, fetch)
        self.assertEqual(calls, ["png"])


class NewestRelease(unittest.TestCase):
    def test_a_404_from_crates_io_is_not_published(self):
        error = urllib.error.HTTPError("https://crates.io", 404, "Not Found", email.message.Message(), None)
        with (
            mock.patch.object(fork_updates.urllib.request, "urlopen", side_effect=error),
            self.assertRaises(fork_updates.NotPublishedError),
        ):
            fork_updates.newest_release("parajpeg")

    def test_any_other_http_error_stays_an_error(self):
        error = urllib.error.HTTPError("https://crates.io", 500, "Server Error", email.message.Message(), None)
        with (
            mock.patch.object(fork_updates.urllib.request, "urlopen", side_effect=error),
            self.assertRaises(urllib.error.HTTPError),
        ):
            fork_updates.newest_release("png")


class VersionsAndReport(unittest.TestCase):
    def test_versions_compare_by_their_numbers(self):
        self.assertEqual(fork_updates.version_key("0.41.1"), (0, 41, 1))
        self.assertEqual(fork_updates.version_key("1.2"), (1, 2, 0))
        self.assertEqual(fork_updates.status_of("0.41.0", "0.41.1"), "NEWER")
        self.assertEqual(fork_updates.status_of("0.41.1", "0.41.0"), "current")

    def test_the_report_explains_a_crate_that_is_not_published(self):
        rows = [
            {
                "crate": "parajpeg",
                "repository": "darktorres/parajpeg",
                "locked": "0.1.0",
                "locks": ["Cargo.lock"],
                "newest": "-",
                "status": "NOT PUBLISHED",
            }
        ]
        text = fork_updates.report(rows)
        self.assertIn("NOT PUBLISHED", text)
        self.assertIn("Not on crates.io", text)
        self.assertIn("`parajpeg`", text)


if __name__ == "__main__":
    unittest.main()
