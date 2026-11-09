#!/usr/bin/env python3
"""Builds callgraph.dll (callgraph.cpp) into target/pin with the Pin kit that comes with Intel SDE. Windows only (Intel Pin,
clang-cl): it was build.sh, which needed Git Bash.

Pin 4's wrappers (pinrt/bin/pin-clang-cl++, pin-ld) around LLVM's clang-cl and lld-link, with Pin's own C and C++ runtime.
SDE's mbuild scripts do the same but don't know Visual Studio 18. Needs LLVM (LLVM_DIR, default %ProgramFiles%\\LLVM) and the
Windows SDK's kernel32.lib (KitsRoot10 in the registry). LLVM_DIR and PIN_ROOT (the SDE kit's pinkit folder, default under
LOCALAPPDATA) can come from the repo's .env, for keys the environment does not set.
"""

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

from pgtools import REPO, WINDOWS, load_env_file


def newest(pattern: Path) -> Path:
    """The folder matching `pattern` with the highest version (its digits compared in order), or exits when there is none."""
    anchor = Path(pattern.anchor)
    matches = [str(match) for match in anchor.glob(str(pattern.relative_to(anchor)))]
    if not matches:
        sys.exit(f"build.py: nothing matches {pattern}")
    return Path(max(matches, key=lambda path: [int(n) for n in re.findall(r"\d+", path)]))


def windows_kit_root() -> Path:
    """The Windows SDK's root (KitsRoot10, in the registry)."""
    import winreg

    key = winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows Kits\Installed Roots")
    value, _ = winreg.QueryValueEx(key, "KitsRoot10")
    return Path(value)


def tool(path: Path) -> str:
    """The program at `path`, with the extension Windows uses for it (.exe, .cmd, .bat); exits when it is not there."""
    found = shutil.which(str(path))
    if found is None:
        sys.exit(f"build.py: {path} is not there: check the Pin kit (PIN_ROOT) and LLVM (LLVM_DIR)")
    return found


def main() -> None:
    if not WINDOWS:
        sys.exit("build.py: Windows only (Intel Pin and clang-cl)")
    load_env_file()
    llvm_dir = os.environ.get("LLVM_DIR") or (
        str(Path(os.environ["PROGRAMFILES"]) / "LLVM") if os.environ.get("PROGRAMFILES") else ""
    )
    if not llvm_dir:
        sys.exit("build.py: LLVM_DIR is unset and ProgramFiles is unavailable: set LLVM_DIR")
    llvm = Path(llvm_dir)
    pin = (
        Path(os.environ["PIN_ROOT"])
        if os.environ.get("PIN_ROOT")
        else Path(os.environ["LOCALAPPDATA"]) / "Programs" / "sde" / "pinkit"
    )
    here = Path(__file__).resolve().parent
    out = REPO / "target" / "pin"
    out.mkdir(parents=True, exist_ok=True)

    # The wrapper puts $INCLUDE's directories after Pin's runtime headers: clang's own (intrinsics) must come last.
    clang_inc = newest(llvm / "lib" / "clang" / "*" / "include")
    sdk_lib = newest(windows_kit_root() / "Lib" / "*" / "um" / "x64")
    env = {**os.environ, "PATH": f"{llvm / 'bin'}{os.pathsep}{os.environ.get('PATH', '')}", "INCLUDE": str(clang_inc)}

    compile_command = [
        tool(pin / "intel64" / "pinrt" / "bin" / "pin-clang-cl++"),
        "-c",
        str(here / "callgraph.cpp"),
        f"-Fo{out / 'callgraph.obj'}",
        "//std:c++17",
        "//O2",
        "//MD",
        "//Zi",
        "//W3",
        "-Wno-non-c-typedef-for-linkage",
        "-Wno-microsoft-include",
        "-Wno-unicode",
        "-Wno-unused-command-line-argument",
        # Pin's musl defines max_align_t, which clang's stddef.h would define again for an MSVC target.
        "-D__CLANG_MAX_ALIGN_T_DEFINED",
        "-DPIN_DISABLE_CRT_REG_DEF",
        "-D_XOPEN_SOURCE=700",
        "-D_POSIX_C_SOURCE=200809L",
        f"-I{pin / 'source' / 'include' / 'pin'}",
        f"-I{pin / 'source' / 'include' / 'pin' / 'gen'}",
        f"-I{pin / 'intel64' / 'pinrt' / 'include' / 'adaptor'}",
        f"-I{pin / 'extras' / 'components' / 'include'}",
        f"-I{pin / 'extras' / 'xed-intel64' / 'include' / 'xed'}",
        "-Xclang",
        "-internal-isystem",
        "-Xclang",
        str(clang_inc),
    ]
    link_command = [
        tool(pin / "intel64" / "pinrt" / "bin" / "pin-ld"),
        "-dll",
        "-c++",
        "//EXPORT:main",
        "//INCREMENTAL:NO",
        "//IGNORE:4210",
        "//IGNORE:4049",
        "//DYNAMICBASE",
        "//NXCOMPAT",
        "//OPT:REF",
        "//DEBUG",
        f"-libpath:{pin / 'intel64' / 'lib'}",
        f"-libpath:{pin / 'extras' / 'xed-intel64' / 'lib'}",
        f"-libpath:{sdk_lib}",
        str(out / "callgraph.obj"),
        "pin.lib",
        "pinrt-adaptor-static.lib",
        "xed.lib",
        "kernel32.lib",
        f"-out:{out / 'callgraph.dll'}",
    ]
    for command in (compile_command, link_command):
        print("+ " + " ".join(command), flush=True)
        result = subprocess.run(command, env=env, check=False)
        if result.returncode != 0:
            sys.exit(result.returncode)
    print(f"built {out / 'callgraph.dll'}")


if __name__ == "__main__":
    main()
