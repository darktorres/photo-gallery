"""The static FFmpeg link check's DLL policy (`build_static_ffmpeg.is_system_dll` and `is_redistributed`): stdlib unittest,
run by `just test-tools`. Importing the script is safe: `main()` runs only under `if __name__ == "__main__"`."""

import unittest

import build_static_ffmpeg as build


class SystemDlls(unittest.TestCase):
    def test_an_exact_system_dll_is_accepted(self):
        self.assertTrue(build.is_system_dll("KERNEL32.dll"))
        self.assertTrue(build.is_system_dll("USER32.dll"))
        self.assertTrue(build.is_system_dll("bcrypt.dll"))

    def test_an_api_set_dll_is_accepted_by_its_prefix(self):
        self.assertTrue(build.is_system_dll("api-ms-win-crt-runtime-l1-1-0.dll"))
        self.assertTrue(build.is_system_dll("ext-ms-win-ntuser-window-l1-1-0.dll"))

    def test_a_longer_name_with_a_listed_prefix_is_rejected(self):
        # The old regex matched by prefix (`bcrypt` took `bcryptprimitives.dll`); exact names do not.
        self.assertFalse(build.is_system_dll("bcryptprimitives.dll"))
        self.assertFalse(build.is_system_dll("KERNEL32X.dll"))

    def test_an_unknown_dll_is_rejected(self):
        self.assertFalse(build.is_system_dll("made-up.dll"))

    def test_the_vc_runtime_is_not_a_system_dll(self):
        self.assertFalse(build.is_system_dll("VCRUNTIME140.dll"))

    def test_the_vc_runtime_is_accepted_only_through_redistributed(self):
        self.assertTrue(build.is_redistributed("VCRUNTIME140.dll"))
        self.assertTrue(build.is_redistributed("vcruntime140.DLL"))
        # The C runtime's `_1` companion is not in the list: exact names reject it (correct for a C program).
        self.assertFalse(build.is_redistributed("VCRUNTIME140_1.dll"))
        # MSVCP140 is the C++ runtime, which the plain-C static check does not import.
        self.assertFalse(build.is_redistributed("MSVCP140.dll"))

    def test_a_redistributed_dll_is_not_a_system_dll(self):
        self.assertFalse(any(build.is_system_dll(name) for name in build.REDISTRIBUTED))


if __name__ == "__main__":
    unittest.main()
