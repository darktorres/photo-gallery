"""The compiler graph's cache (tools/modgraph/compiler_graph.py): a crate's graph is reused only while its key holds."""

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "modgraph"))

import compiler_graph as cg


class SourceKey(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.folder = Path(self.tmp.name) / "pg-demo"
        (self.folder / "src").mkdir(parents=True)
        (self.folder / "src" / "lib.rs").write_text("pub mod a;\n", encoding="utf-8")
        (self.folder / "src" / "a.rs").write_text("pub fn f() {}\n", encoding="utf-8")

    def tearDown(self):
        self.tmp.cleanup()

    def key(self, tools="cm 0.28.0|rustc|windows"):
        return cg.source_key("pg-demo", self.folder, tools)

    def test_the_same_files_give_the_same_key(self):
        self.assertEqual(self.key(), self.key())

    def test_an_edited_file_changes_the_key(self):
        before = self.key()
        (self.folder / "src" / "a.rs").write_text("pub fn g() {}\n", encoding="utf-8")
        self.assertNotEqual(before, self.key())

    def test_a_new_module_changes_the_key(self):
        before = self.key()
        (self.folder / "src" / "b.rs").write_text("\n", encoding="utf-8")
        self.assertNotEqual(before, self.key())

    def test_the_tools_and_the_target_change_the_key(self):
        self.assertNotEqual(self.key("cm 0.28.0|rustc|windows"), self.key("cm 0.28.0|rustc|linux"))
        self.assertNotEqual(self.key("cm 0.28.0|rustc|windows"), self.key("cm 0.29.0|rustc|windows"))

    def test_the_target_folder_is_not_part_of_the_key(self):
        before = self.key()
        (self.folder / "target").mkdir()
        (self.folder / "target" / "junk.rs").write_text("x\n", encoding="utf-8")
        self.assertEqual(before, self.key())


class CachedDot(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.out = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def test_a_stored_graph_is_reused_only_for_its_key(self):
        (self.out / "pg-demo.dot").write_text("digraph {}", encoding="utf-8")
        (self.out / "pg-demo.key").write_text("k1", encoding="utf-8")
        self.assertEqual(cg.cached_dot(self.out, "pg-demo", "k1"), "digraph {}")
        self.assertIsNone(cg.cached_dot(self.out, "pg-demo", "k2"))

    def test_no_graph_or_no_key_means_it_is_made_again(self):
        self.assertIsNone(cg.cached_dot(self.out, "pg-demo", "k1"))
        (self.out / "pg-demo.dot").write_text("digraph {}", encoding="utf-8")
        self.assertIsNone(cg.cached_dot(self.out, "pg-demo", "k1"))


if __name__ == "__main__":
    unittest.main()
