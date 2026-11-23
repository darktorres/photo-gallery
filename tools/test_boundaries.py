"""The checks of `boundaries.py` that take a file's text: the one-public-path rule (`check_public_paths`), the
environment reads (`check_env`) and the foreign-function blocks of the platform rule (`check_platform`)."""

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

    def test_a_root_reexport_from_pub_mod_fault_is_a_second_path(self):
        self.assertEqual(len(failures("pub mod fault;\npub use fault::X;\n")), 1)

    def test_only_crate_roots_are_checked(self):
        text = "pub mod a;\npub use a::X;\n"
        self.assertEqual(boundaries.check_public_paths({REPO / "crates" / "demo" / "src" / "other.rs": text}), [])


ENV_OK = (
    "pub fn process(name: &str) -> Option<OsString> {\n    std::env::var_os(name)\n}\n"
    "pub(crate) unsafe fn set_process(name: &str, value: &str) {\n    std::env::set_var(name, value)\n}\n"
)


class EnvironmentReads(unittest.TestCase):
    def failures(self, path, text):
        return boundaries.check_env({path: text})

    def test_a_build_script_may_read_the_environment(self):
        build = REPO / "crates" / "demo" / "build.rs"
        self.assertEqual(self.failures(build, 'fn main() { let _ = std::env::var("CARGO_CFG_TARGET_OS"); }\n'), [])

    def test_a_build_module_in_src_may_not(self):
        module = REPO / "crates" / "demo" / "src" / "build.rs"
        self.assertEqual(len(self.failures(module, 'fn f() { let _ = std::env::var("PG_HOME"); }\n')), 1)

    def test_any_other_file_may_not(self):
        lib = REPO / "crates" / "demo" / "src" / "lib.rs"
        self.assertEqual(len(self.failures(lib, 'fn f() { let _ = std::env::var("PG_HOME"); }\n')), 1)

    def test_the_registry_reads_inside_its_two_functions_only(self):
        env = REPO / "crates" / "platform" / "src" / "env.rs"
        self.assertEqual(self.failures(env, ENV_OK), [])

    def test_a_stray_read_in_the_registry_file_fails(self):
        env = REPO / "crates" / "platform" / "src" / "env.rs"
        stray = ENV_OK + 'fn other() {\n    let _ = std::env::var_os("PG_HOME");\n}\n'
        self.assertEqual(len(self.failures(env, stray)), 1)

    def test_a_drifted_reader_header_fails(self):
        env = REPO / "crates" / "platform" / "src" / "env.rs"
        self.assertEqual(len(self.failures(env, "fn unrelated() {}\n")), 1)


class ForeignBlocks(unittest.TestCase):
    def failures(self, path, text):
        return boundaries.check_platform({path: text})

    def test_a_system_block_outside_the_allowed_crates_fails(self):
        lib = REPO / "crates" / "demo" / "src" / "lib.rs"
        self.assertEqual(len(self.failures(lib, 'unsafe extern "system" {\n    fn GetTickCount() -> u32;\n}\n')), 1)

    def test_a_c_block_outside_the_allowed_crates_fails(self):
        lib = REPO / "crates" / "demo" / "src" / "lib.rs"
        self.assertEqual(len(self.failures(lib, 'extern "C" {\n    fn abs(x: i32) -> i32;\n}\n')), 1)

    def test_a_block_in_an_allowed_crate_is_fine(self):
        for name in ("platform", "trace", "video", "desktop"):
            ffi = REPO / "crates" / name / "src" / "ffi.rs"
            self.assertEqual(self.failures(ffi, 'unsafe extern "C" {\n    fn f();\n}\n'), [], name)

    def test_a_cxx_qt_bridge_is_not_a_foreign_block(self):
        lib = REPO / "crates" / "demo" / "src" / "lib.rs"
        self.assertEqual(self.failures(lib, 'unsafe extern "C++" {\n    include!("bridge.h");\n}\n'), [])


if __name__ == "__main__":
    unittest.main()
