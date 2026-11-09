#!/usr/bin/env python3
"""Builds libjpeg-turbo (JPEG 8.0 API, static, the dynamic CRT) once, for both the static Qt's JPEG plugin and the app's
turbojpeg. One archive in the process: Qt's qjpeg and the app's turbojpeg must not carry two libjpeg copies (their structs
differ in size, and the link resolves both to one of them). The reasons are in docs/STATIC_BUILD.md. Windows only (MSVC).
It was build-jpeg-turbo.ps1.

    python tools/build_jpeg_turbo.py                            # into <PG_HOME>\\static\\jpeg-turbo
    python tools/build_jpeg_turbo.py --work D:\\scratch\\jpeg     # a clean run, into another folder

Outputs: <work>\\prefix (headers and lib, for Qt's -system-libjpeg), and <work>\\link\\turbojpeg.lib, the same archive under
the name turbojpeg-sys links for TURBOJPEG_SOURCE=explicit (TURBOJPEG_LIB_DIR, TURBOJPEG_INCLUDE_DIR). The default folder is
PG_HOME's static folder (from the environment or .env): pass --work when PG_HOME is not set.
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

from pgtools import WINDOWS, import_msvc_environment, load_env_file

# ---- Pins. Change them here, and record the reason in docs/STATIC_BUILD.md. --------------------------------------------
# The libjpeg-turbo sources are inside the rust-turbojpeg fork at this commit (turbojpeg-sys/libjpeg-turbo), the same commit
# the root Cargo.toml pins for turbojpeg-sys, so the app's and Qt's libjpeg come from one source.
FORK_URL = "https://github.com/darktorres/rust-turbojpeg"
FORK_COMMIT = "945d853b47a76f98fd97ac5ff43830997712caa2"
TOOLSET = "14.44.35207"  # MSVC toolset (vcvars64 -vcvars_ver)
DEFAULT_BUILD_TOOLS = Path(r"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools")


def checked(what: str, command: list[str], log: Path | None = None, cwd: Path | None = None) -> None:
    """Runs `command`, its output in `log` when given; a failure ends this program."""
    if log is None:
        result = subprocess.run(command, cwd=cwd, check=False)
    else:
        with log.open("w", encoding="utf-8") as file:
            result = subprocess.run(command, cwd=cwd, stdout=file, stderr=subprocess.STDOUT, check=False)
    if result.returncode != 0:
        sys.exit(f"{what} failed (exit {result.returncode})")


def fetch_pinned(src: Path, log: Path) -> None:
    """Fetches the pinned commit into `src`, retrying: the network drops connections now and then. Three tries, then exits."""
    for attempt in range(1, 4):
        with log.open("w", encoding="utf-8") as file:
            result = subprocess.run(
                ["git", "-C", str(src), "fetch", "-q", "--depth", "1", FORK_URL, FORK_COMMIT],
                stdout=file,
                stderr=subprocess.STDOUT,
                check=False,
            )
        if result.returncode == 0:
            return
        if attempt < 3:
            time.sleep(10 * attempt)
    sys.exit(f"git fetch failed after 3 attempts: see {log}")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest().upper()


def default_work() -> Path:
    home = os.environ.get("PG_HOME")
    if not home:
        sys.exit("PG_HOME is not set: pass --work, or set PG_HOME in .env to the data folder (static/ goes under it)")
    return Path(home) / "static" / "jpeg-turbo"


def main() -> None:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    parser.add_argument("--work", type=Path, default=None, help="the work folder (default: PG_HOME's static\\jpeg-turbo)")
    parser.add_argument("--build-tools", type=Path, default=DEFAULT_BUILD_TOOLS, help="the Visual Studio Build Tools folder")
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    args = parser.parse_args()
    if not WINDOWS:
        sys.exit("build_jpeg_turbo.py: Windows only (MSVC)")
    load_env_file()

    work = (args.work or default_work()).resolve()
    logs = work / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    src, build, prefix, link = work / "src", work / "build", work / "prefix", work / "link"

    import_msvc_environment(args.build_tools, TOOLSET)

    # The source: the fork at the pinned commit, by hash.
    if (src / ".git").is_dir():
        checked("git clean", ["git", "-C", str(src), "clean", "-fdxq"])
    else:
        src.mkdir(parents=True, exist_ok=True)
        checked("git init", ["git", "-C", str(src), "init", "-q"])
        # Git for Windows converts line endings (core.autocrlf=true by default), so the headers would differ by machine.
        checked("git config", ["git", "-C", str(src), "config", "core.autocrlf", "false"])
    fetch_pinned(src, logs / "git-fetch.log")
    checked("git checkout", ["git", "-C", str(src), "checkout", "-q", "-f", FORK_COMMIT])
    libjpeg = src / "turbojpeg-sys" / "libjpeg-turbo"
    if not (libjpeg / "CMakeLists.txt").is_file():
        sys.exit(f"libjpeg-turbo sources not found in {libjpeg}")

    # Clean output folders, so each run starts empty.
    for folder in [build, prefix, link]:
        shutil.rmtree(folder, ignore_errors=True)
    build.mkdir(parents=True)

    # The configure line. JPEG8: the IJG 8.0 API (JPEG_LIB_VERSION 80), which Qt's libjpeg expects. WITH_CRT_DLL: the dynamic
    # CRT (/MD), as the app and Qt are. WITH_TURBOJPEG: the TurboJPEG API the app uses (turbojpeg-static.lib).
    cmake_args = [
        "-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release",
        f"-DCMAKE_INSTALL_PREFIX={prefix}", "-DCMAKE_INSTALL_LIBDIR=lib",
        "-DENABLE_SHARED=OFF", "-DENABLE_STATIC=ON",
        "-DWITH_JPEG8=ON", "-DWITH_CRT_DLL=ON", "-DWITH_TURBOJPEG=ON",
        str(libjpeg),
    ]  # fmt: skip
    checked("cmake configure", ["cmake", *cmake_args], logs / "configure.log", cwd=build)
    checked("cmake build", ["cmake", "--build", ".", "--parallel", str(args.jobs)], logs / "build.log", cwd=build)
    checked("cmake install", ["cmake", "--install", "."], logs / "install.log", cwd=build)

    # The app's link name (turbojpeg-sys links `turbojpeg` in TURBOJPEG_LIB_DIR): the same file, under that name.
    link.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(prefix / "lib" / "turbojpeg-static.lib", link / "turbojpeg.lib")

    # The JPEG API version the headers say, checked: Qt's configure and the app both depend on it.
    jconfig = (prefix / "include" / "jconfig.h").read_text(encoding="utf-8")
    if not re.search(r"#define\s+JPEG_LIB_VERSION\s+80", jconfig):
        sys.exit("jconfig.h does not have JPEG_LIB_VERSION 80")

    library = prefix / "lib" / "turbojpeg-static.lib"
    manifest = {
        "fork_url": FORK_URL,
        "fork_commit": FORK_COMMIT,
        "msvc_toolset": TOOLSET,
        "configure": cmake_args[:-1],
        "libraries": [{"path": str(library), "sha256": sha256(library)}],
    }
    (work / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"done: {work / 'manifest.json'}")


if __name__ == "__main__":
    main()
