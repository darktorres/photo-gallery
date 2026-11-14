#!/usr/bin/env python3
"""The C side of the PNG decoder comparison on Windows: zlib (static, shared by libpng and libspng), libpng, libspng,
Wuffs, and the harness (harness/, which builds png-c-harness.exe). It follows eval/jpeg-decoders/win_build.py.

    python eval/png-decoders/win_build.py [zlib libpng libspng wuffs harness]   (default: all)

Sources go to %PG_ROOT%\\eval\\png-decoders\\src, each at its pinned commit; builds go to ...\\build and installs to
...\\deps. The MSVC environment is set up as win_env.py's setup() does it (vswhere, then vcvars64), without that
function's CUDA requirement: nothing here uses CUDA. Windows only. NASM and CMake/Ninja must be on PATH.
"""

import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "tools"))

from pgtools import import_msvc_environment, load_env_file, run  # noqa: E402

# The pins: each commit is what its tag (or Wuffs' snapshot) resolved to when the build was made.
ZLIB = ("https://github.com/madler/zlib", "99623ef7097352da7993ba614161fd6a311e046e")  # v1.4
LIBPNG = ("https://github.com/pnggroup/libpng", "cd952f49f95bb27154ae77dbb103032d95f6e580")  # v1.6.59
LIBSPNG = ("https://github.com/randy408/libspng", "fb768002d4288590083a476af628e51c3f1d47cd")  # v0.7.4
WUFFS = ("https://github.com/google/wuffs", "d28ecd494491baa9b11d2557d10819111050fd24")  # d28ecd4, 2026-10-31

ORDER = ["zlib", "libpng", "libspng", "wuffs", "harness"]

# zlib's CMake names its static library libz plus a suffix ("s" with MSVC), so it installs as libzs.lib.
ZLIB_STATIC_LIB = "libzs.lib"


def msvc() -> None:
    """Sets this process's environment for cl. vswhere finds the newest Visual Studio with the C++ tools; vcvars64 sets
    the rest (as win_env.setup does, which also needs CUDA and is not used here)."""
    if os.name != "nt":
        sys.exit("win_build: Windows only (MSVC)")
    load_env_file()
    installer = Path(os.environ.get("PROGRAMFILES(X86)", r"C:\Program Files (x86)")) / "Microsoft Visual Studio" / "Installer"
    vswhere = installer / "vswhere.exe"
    if not vswhere.is_file():
        sys.exit("win_build: vswhere.exe not found in the Visual Studio Installer folder")
    found = subprocess.run(
        [str(vswhere), "-products", "*", "-latest", "-property", "installationPath"],
        capture_output=True,
        text=True,
        check=False,
    ).stdout.split("\n")
    installation = [line.strip() for line in found if line.strip()]
    if not installation:
        sys.exit("win_build: vswhere found no Visual Studio installation with the C++ tools")
    # vcvars64.bat runs vswhere.exe by its bare name, which resolves only with the Installer folder on PATH.
    os.environ["PATH"] = os.pathsep.join([str(installer), os.environ.get("PATH", "")])
    import_msvc_environment(Path(installation[-1]))


def png_root() -> Path:
    """eval/png-decoders under PG_ROOT (the repository's .env sets it on this machine)."""
    load_env_file()
    pg_root = os.environ.get("PG_ROOT")
    if not pg_root:
        sys.exit("win_build: PG_ROOT is not set (for example in the repository's .env)")
    root = Path(pg_root) / "eval" / "png-decoders"
    for folder in ["src", "build", "deps"]:
        (root / folder).mkdir(parents=True, exist_ok=True)
    return root


def clone(src: Path, name: str, pin: tuple[str, str]) -> Path:
    """A full clone with autocrlf off (LF line endings, as upstream), checked out at the pinned commit. A folder already
    there is kept, but it must be at the pin."""
    url, commit = pin
    dest = src / name
    if dest.exists():
        head = subprocess.run(
            ["git", "-C", str(dest), "rev-parse", "HEAD"], capture_output=True, text=True, check=False
        ).stdout.strip()
        if head != commit:
            sys.exit(f"win_build: {dest} is at {head or 'no commit'}, not the pin {commit}")
        return dest
    run(["git", "-c", "core.autocrlf=false", "clone", "-q", url, str(dest)])
    run(["git", "-C", str(dest), "-c", "core.autocrlf=false", "checkout", "-q", commit])
    return dest


def zlib(root: Path) -> None:
    """zlib 1.4, static (libz.lib). libpng and libspng link this one copy, so the deflate code is the same."""
    src = clone(root / "src", "zlib", ZLIB)
    build, deps = root / "build" / "zlib", root / "deps" / "zlib"
    run(
        [
            "cmake", "-G", "Ninja", "-S", str(src), "-B", str(build), "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_C_COMPILER=cl",
            "-DZLIB_BUILD_SHARED=OFF", "-DZLIB_BUILD_STATIC=ON", "-DZLIB_BUILD_TESTING=OFF", f"-DCMAKE_INSTALL_PREFIX={deps}",
        ],
        quiet=True,
    )  # fmt: skip
    run(["cmake", "--build", str(build), "--target", "zlibstatic"])
    run(["cmake", "--install", str(build)], quiet=True)


def libpng(root: Path) -> None:
    """libpng 1.6.59, static, over the zlib above."""
    src = clone(root / "src", "libpng", LIBPNG)
    build, deps, zdeps = root / "build" / "libpng", root / "deps" / "libpng", root / "deps" / "zlib"
    run(
        [
            "cmake", "-G", "Ninja", "-S", str(src), "-B", str(build), "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_C_COMPILER=cl",
            "-DPNG_SHARED=OFF", "-DPNG_STATIC=ON", "-DPNG_TESTS=OFF", "-DPNG_TOOLS=OFF",
            f"-DZLIB_INCLUDE_DIR={zdeps / 'include'}", f"-DZLIB_LIBRARY={zdeps / 'lib' / ZLIB_STATIC_LIB}",
            f"-DCMAKE_INSTALL_PREFIX={deps}",
        ],
        quiet=True,
    )  # fmt: skip
    run(["cmake", "--build", str(build), "--target", "png_static"])
    run(["cmake", "--install", str(build)], quiet=True)


def libspng(root: Path) -> None:
    """libspng 0.7.4, static, over the same zlib (no miniz: its zlib is the one above)."""
    src = clone(root / "src", "libspng", LIBSPNG)
    build, deps, zdeps = root / "build" / "libspng", root / "deps" / "libspng", root / "deps" / "zlib"
    run(
        [
            "cmake", "-G", "Ninja", "-S", str(src), "-B", str(build), "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_C_COMPILER=cl",
            "-DSPNG_SHARED=OFF", "-DSPNG_STATIC=ON", "-DBUILD_EXAMPLES=OFF",
            f"-DZLIB_INCLUDE_DIR={zdeps / 'include'}", f"-DZLIB_LIBRARY={zdeps / 'lib' / ZLIB_STATIC_LIB}",
            f"-DCMAKE_INSTALL_PREFIX={deps}",
        ],
        quiet=True,
    )  # fmt: skip
    run(["cmake", "--build", str(build), "--target", "spng_static"])
    run(["cmake", "--install", str(build)], quiet=True)


def wuffs(root: Path) -> None:
    """Wuffs at d28ecd4: the harness compiles its single-file C release in, so there is nothing to build here."""
    src = clone(root / "src", "wuffs", WUFFS)
    if not (src / "release" / "c" / "wuffs-unsupported-snapshot.c").is_file():
        sys.exit(f"win_build: {src} has no release/c/wuffs-unsupported-snapshot.c")


def harness(root: Path) -> None:
    """png-c-harness.exe, Release, with the libraries above."""
    src, build, deps = root / "src", root / "build", root / "deps"
    run(
        [
            "cmake", "-G", "Ninja", "-S", str(HERE / "harness"), "-B", str(build / "harness"), "-DCMAKE_BUILD_TYPE=Release",
            "-DCMAKE_CXX_COMPILER=cl",
            f"-DZLIB_LIB={deps / 'zlib' / 'lib' / ZLIB_STATIC_LIB}",
            f"-DZLIB_INCLUDE={deps / 'zlib' / 'include'}",
            f"-DLIBPNG_INCLUDE={deps / 'libpng' / 'include' / 'libpng16'}",
            f"-DLIBPNG_LIB={deps / 'libpng' / 'lib' / 'libpng16_static.lib'}",
            f"-DSPNG_INCLUDE={deps / 'libspng' / 'include'}",
            f"-DSPNG_LIB={deps / 'libspng' / 'lib' / 'spng_static.lib'}",
            f"-DWUFFS_SRC={src / 'wuffs'}",
        ],
        quiet=True,
    )  # fmt: skip
    run(["cmake", "--build", str(build / "harness"), "-j", "4"])


STEPS = {"zlib": zlib, "libpng": libpng, "libspng": libspng, "wuffs": wuffs, "harness": harness}


def main() -> None:
    steps = sys.argv[1:] or ORDER
    unknown = [step for step in steps if step not in STEPS]
    if unknown:
        sys.exit(f"win_build.py: unknown step(s) {' '.join(unknown)}; the steps are {' '.join(ORDER)}")
    msvc()
    root = png_root()
    for step in steps:
        STEPS[step](root)


if __name__ == "__main__":
    main()
