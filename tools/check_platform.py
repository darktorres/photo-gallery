#!/usr/bin/env python3
"""The platform rule (docs/MODULARITY_PLAN.md, M-13 and M-16): the Win32 calls and the per-OS branches of the production code
are in crates/platform. The crates above it hold none, except the Qt and FFI declarations that need per-platform signatures,
each listed in ALLOWED with its reason. Test code is not checked: a `#[cfg(test)]` item (its braces are counted), and the
files under a crate's tests folder.

    python tools/check_platform.py      # exit 1 and the sites, when one is neither in pg-platform nor allowed
"""

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PLATFORM = "crates/platform/"
# Test support (docs/MODULARITY_PLAN.md, M-14): its OS branches are test code, not the app's.
TEST_SUPPORT = "crates/testkit/"

# A platform marker: a cfg on the OS, the OS's std modules, Win32 bindings, or the libraries' OS modules.
MARKER = re.compile(r"cfg!?\([^)]*\b(windows|unix)\b|cfg!?\(target_os|os::(windows|unix)::|windows_sys|libloading::os::")
TEST_CFG = re.compile(r"#\[cfg\((?:all\()?test\b")

# The declarations that need the platform's own signatures, and the glue that needs the platform's own types, with the reason.
ALLOWED = {
    "crates/model/src/rules.rs": [
        ("cfg!(windows)", "the native path rules are defined from the OS here (PathRules::NATIVE)"),
    ],
    "crates/desktop/src/video_source.rs": [
        ("cfg(windows)", "the player's C++ takes a file's units on Windows and a descriptor on Unix: the declarations differ"),
        ("cfg(unix)", "the player's C++ takes a file's units on Windows and a descriptor on Unix: the declarations differ"),
    ],
    "crates/desktop/src/picker.rs": [
        ("cfg(windows)", "the system's dialog is Windows' (pg_platform::picker::Dialog exists on Windows only)"),
        ("cfg(not(windows))", "the system's dialog is Windows' (pg_platform::picker::Dialog exists on Windows only)"),
    ],
}


def production_lines(text: str):
    """(line number, line) of the code outside test items: a `#[cfg(test)]` item from its attribute to its closing brace."""
    depth = 0
    test_start = None  # the brace depth before the test item's body
    pending = False  # a test attribute whose item has not opened its body yet
    for number, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if test_start is None and TEST_CFG.match(stripped):
            pending = True
        in_test = test_start is not None or pending
        if not in_test:
            yield number, line
        if pending and "{" in line:
            test_start = depth
            pending = False
        elif pending and stripped.endswith(";"):
            pending = False  # a test `use` or `const`: no body
        depth += line.count("{") - line.count("}")
        if test_start is not None and depth <= test_start:
            test_start = None


def allowed(path: str, line: str) -> bool:
    return any(marker in line for marker, _ in ALLOWED.get(path, []))


def main() -> None:
    if sys.argv[1:]:
        sys.exit(__doc__ or "usage: check_platform.py")
    found = []
    for path in sorted(REPO.glob("crates/*/src/**/*.rs")):
        relative = path.relative_to(REPO).as_posix()
        if relative.startswith((PLATFORM, TEST_SUPPORT)):
            continue
        for number, line in production_lines(path.read_text(encoding="utf-8")):
            if MARKER.search(line) and not allowed(relative, line):
                found.append(f"{relative}:{number}: {line.strip()}")
    if found:
        print("platform code outside crates/platform (move it there, or list it in ALLOWED with its reason):")
        print("\n".join(found))
        sys.exit(1)
    print("platform code is in crates/platform only (test code and the listed declarations aside)")


if __name__ == "__main__":
    main()
