"""The version registry's conformance (crates/model/src/versions.rs): every `*VERSION` constant in the workspace is
registered there, and every registered one is read by its owner.

A regex scan of the Rust sources, not a parser: a version constant is a `const NAME_VERSION` line at the start of a line
(with an optional `pub` or `pub(crate)`), and an owner reads a registry constant as `versions::NAME`. Run with
`just test-tools`.
"""

import re
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REGISTRY_FILE = REPO / "crates" / "model" / "src" / "versions.rs"

# A `*VERSION` constant declaration on one line.
CONST = re.compile(r"^\s*(?:pub(?:\(\w+\))?\s+)?const\s+(\w*VERSION\w*)\s*:")
# A registry constant, as its owner reads it.
READ = r"versions::{name}\b"


def registry_names(text):
    """The constants `versions.rs` declares: every `pub const NAME_VERSION`-style item."""
    return [m.group(1) for line in text.splitlines() if (m := CONST.match(line))]


def version_constants(text):
    """The `*VERSION` constants declared on the lines of `text`."""
    return [m.group(1) for line in text.splitlines() if (m := CONST.match(line))]


def sources():
    """Every Rust source file of the workspace's crates (their `src` folders)."""
    return sorted(REPO.glob("crates/*/src/**/*.rs"))


class VersionRegistry(unittest.TestCase):
    def setUp(self):
        self.registry = registry_names(REGISTRY_FILE.read_text(encoding="utf-8"))
        self.files = {path: path.read_text(encoding="utf-8", errors="replace") for path in sources()}

    def test_the_scan_finds_a_version_constant_and_skips_the_rest(self):
        self.assertEqual(version_constants("pub const IMPORT_VERSION: i64 = 3;"), ["IMPORT_VERSION"])
        self.assertEqual(version_constants("    pub(crate) const LAYOUT_VERSION: i64 = 1;"), ["LAYOUT_VERSION"])
        self.assertEqual(version_constants("const X_VERSION: i64 = 1;"), ["X_VERSION"])
        self.assertEqual(version_constants("pub const MAX: usize = 25;"), [])
        self.assertEqual(version_constants("let version = 2;"), [])
        self.assertEqual(version_constants('assert_eq!(x, "const A_VERSION: i64");'), [])

    def test_the_registry_declares_the_known_versions(self):
        self.assertTrue(self.registry, "no version constants found in crates/model/src/versions.rs")
        for name in ("IMPORT_VERSION", "PAIRING_VERSION", "CACHE_VERSION", "CATALOG_SCHEMA_VERSION"):
            self.assertIn(name, self.registry)

    def test_every_version_constant_in_the_workspace_is_registered(self):
        unregistered = [
            f"{path.relative_to(REPO)}: {name}"
            for path, text in self.files.items()
            if path != REGISTRY_FILE
            for name in version_constants(text)
            if name not in self.registry
        ]
        self.assertEqual(unregistered, [], "add these to crates/model/src/versions.rs")

    def test_each_registry_version_is_read_by_its_owner(self):
        unread = []
        for name in self.registry:
            pattern = re.compile(READ.format(name=name))
            if not any(pattern.search(text) for path, text in self.files.items() if path != REGISTRY_FILE):
                unread.append(name)
        self.assertEqual(unread, [], "no owner reads these registry versions (versions::NAME)")


if __name__ == "__main__":
    unittest.main()
