"""The narrowed cargo-deny policies (fuzz/deny.toml, eval/*/deny.toml) against the root's deny.toml: the shared sections
are the same, and every entry a narrowed policy has is one the root has (or, for the eval harnesses' own wrappers, a
crate the root names), so a policy can drop an entry but cannot add one the root does not state."""

import tomllib
import unittest

from pgtools import REPO

ROOT = REPO / "deny.toml"
NARROWED = [
    REPO / "fuzz" / "deny.toml",
    REPO / "eval" / "jpeg-decoders" / "deny.toml",
    REPO / "eval" / "png-decoders" / "deny.toml",
]

# The keys every narrowed policy repeats from the root, with the values that must match.
SHARED = {
    "graph": None,
    "advisories": None,
    "licenses.private": None,
    "sources.unknown-registry": None,
    "sources.unknown-git": None,
    "bans.multiple-versions": None,
    "bans.wildcards": None,
    "bans.allow-wildcard-paths": None,
}


def load(path):
    with path.open("rb") as f:
        return tomllib.load(f)


def get(table, dotted):
    """The value at a dotted key; the whole table for a bare section name."""
    if dotted == "graph":
        return table.get("graph")
    node = table
    for part in dotted.split("."):
        node = node.get(part) if isinstance(node, dict) else None
    return node


class SharedSections(unittest.TestCase):
    def setUp(self):
        self.root = load(ROOT)

    def test_each_narrowed_policy_repeats_the_root_shared_sections(self):
        for path in NARROWED:
            narrowed = load(path)
            for key in SHARED:
                with self.subTest(file=str(path.relative_to(REPO)), key=key):
                    self.assertEqual(get(narrowed, key), get(self.root, key), f"{key} differs from the root")

    def test_each_narrowed_policy_repeats_the_root_license_allow_list(self):
        # The allow-list is the root's; a narrowed policy may mark its entries `unused-allowed-license`, nothing else.
        for path in NARROWED:
            with self.subTest(file=str(path.relative_to(REPO))):
                self.assertEqual(load(path)["licenses"]["allow"], self.root["licenses"]["allow"])


class EntriesAreTheRootsOwn(unittest.TestCase):
    def setUp(self):
        self.root = load(ROOT)
        self.root_crates = {entry["crate"] for entry in self.root["bans"]["deny"]}
        self.root_git = set(self.root["sources"]["allow-git"])

    def test_every_narrowed_ban_names_a_crate_the_root_bans(self):
        for path in NARROWED:
            for entry in load(path)["bans"]["deny"]:
                with self.subTest(file=str(path.relative_to(REPO)), crate=entry["crate"]):
                    self.assertIn(entry["crate"], self.root_crates, "the root does not ban this crate")

    def test_every_narrowed_git_source_is_one_the_root_allows(self):
        for path in NARROWED:
            for url in load(path)["sources"]["allow-git"]:
                with self.subTest(file=str(path.relative_to(REPO)), url=url):
                    self.assertIn(url, self.root_git, "the root does not allow this fork")


if __name__ == "__main__":
    unittest.main()
