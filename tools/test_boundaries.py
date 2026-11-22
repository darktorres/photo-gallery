"""The one-public-path check (`boundaries.check_public_paths`): a crate root's `pub use` names a private module, never a
`pub mod`, except one that is exempt for its own sake."""

import unittest

import boundaries
from pgtools import REPO

LIB = REPO / "crates" / "demo" / "src" / "lib.rs"


def failures(text):
    return boundaries.check_public_paths({LIB: text})


class PublicPaths(unittest.TestCase):
    def test_a_root_reexport_from_a_pub_mod_is_a_second_path(self):
        self.assertEqual(len(failures("pub mod a;\npub use a::X;\n")), 1)

    def test_a_multi_line_reexport_from_a_pub_mod_is_caught(self):
        self.assertEqual(len(failures("pub mod a;\npub use a::{\n    B,\n    C,\n};\n")), 1)

    def test_a_self_path_reexport_from_a_pub_mod_is_caught(self):
        self.assertEqual(len(failures("pub mod a;\npub use self::a::*;\n")), 1)

    def test_a_reexport_from_a_private_mod_is_fine(self):
        self.assertEqual(failures("mod a;\npub use a::X;\n"), [])

    def test_a_reexport_from_a_pub_crate_mod_is_fine(self):
        self.assertEqual(failures("pub(crate) mod a;\npub use a::X;\n"), [])

    def test_an_exempt_pub_mod_is_public_on_its_own(self):
        self.assertEqual(failures("pub mod fault;\n"), [])

    def test_only_crate_roots_are_checked(self):
        text = "pub mod a;\npub use a::X;\n"
        self.assertEqual(boundaries.check_public_paths({REPO / "crates" / "demo" / "src" / "other.rs": text}), [])


if __name__ == "__main__":
    unittest.main()
