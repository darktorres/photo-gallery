"""Which reference a screenshot comparison runs against (compare_shots.resolve_reference: stdlib unittest, `just test-tools`).

Each test builds a small git history in a temporary repository, records references in a temporary folder, and points the
script at both.
"""

import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import compare_shots


def git(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), "-c", "user.name=test", "-c", "user.email=test@example.com", *args],
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout.strip()


class ResolveReference(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        root = Path(self._tmp.name)
        self.repo = root / "repo"
        self.repo.mkdir()
        self.refs = root / "reference"
        git(self.repo, "init", "-q")
        self.c1 = self.commit("one")
        self.c2 = self.commit("two")
        self.c3 = self.commit("three")
        git(self.repo, "checkout", "-q", "-b", "side", self.c1)
        self.side = self.commit("side")
        git(self.repo, "checkout", "-q", "--detach", self.c3)
        patches = [mock.patch.object(compare_shots, "REPO", self.repo), mock.patch.object(compare_shots, "REF_ROOT", self.refs)]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)
        self.addCleanup(self._tmp.cleanup)

    def commit(self, name: str) -> str:
        (self.repo / name).write_text(name, encoding="utf-8")
        git(self.repo, "add", name)
        git(self.repo, "commit", "-q", "-m", name)
        return git(self.repo, "rev-parse", "HEAD")

    def record(self, sha: str) -> Path:
        folder = self.refs / sha[:10]
        folder.mkdir(parents=True, exist_ok=True)
        (folder / "manifest.json").write_text("{}", encoding="utf-8")
        return folder

    def test_the_newest_reference_in_the_history_is_used(self):
        self.record(self.c1)
        self.record(self.c2)
        self.assertEqual(compare_shots.resolve_reference(""), self.refs / self.c2[:10])

    def test_a_reference_on_another_branch_is_not_used(self):
        self.record(self.c1)
        self.record(self.side)
        self.assertEqual(compare_shots.resolve_reference(""), self.refs / self.c1[:10])

    def test_no_reference_in_the_history_exits(self):
        self.record(self.side)
        with self.assertRaises(SystemExit):
            compare_shots.resolve_reference("")

    def test_nothing_recorded_exits(self):
        with self.assertRaises(SystemExit):
            compare_shots.resolve_reference("")

    def test_base_names_the_reference_even_when_a_newer_one_exists(self):
        self.record(self.c1)
        self.record(self.c2)
        self.assertEqual(compare_shots.resolve_reference(self.c1), self.refs / self.c1[:10])

    def test_base_without_a_reference_exits(self):
        self.record(self.c1)
        with self.assertRaises(SystemExit):
            compare_shots.resolve_reference(self.c2)


if __name__ == "__main__":
    unittest.main()
