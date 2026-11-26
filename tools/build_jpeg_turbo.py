#!/usr/bin/env python3
"""Builds libjpeg-turbo (JPEG 8.0 API, static, the dynamic CRT) once, for both the static Qt's JPEG plugin and the app's
turbojpeg. One archive in the process: Qt's qjpeg and the app's turbojpeg must not carry two libjpeg copies (their structs
differ in size, and the link resolves both to one of them).
Windows only (MSVC). It was build-jpeg-turbo.ps1.

    python tools/build_jpeg_turbo.py                            # into <PG_HOME>\\static\\jpeg-turbo
    python tools/build_jpeg_turbo.py --work D:\\scratch\\jpeg     # a clean run, into another folder

Outputs: <work>\\prefix (headers and lib, for Qt's -system-libjpeg), and <work>\\link\\turbojpeg.lib, the same archive under
the name turbojpeg-sys links for TURBOJPEG_SOURCE=explicit (TURBOJPEG_LIB_DIR, TURBOJPEG_INCLUDE_DIR). The default folder is
PG_HOME's static folder (from the environment or .env): pass --work when PG_HOME is not set.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
import tomllib
from pathlib import Path

from pgtools import (
    DEFAULT_BUILD_TOOLS,
    MSVC_TOOLSET,
    REPO,
    WINDOWS,
    checked,
    import_msvc_environment,
    load_env_file,
    not_set,
    sha256,
)


def cargo_fork_pin() -> tuple[str, str]:
    """The rust-turbojpeg fork's URL and commit, as the root Cargo.toml's [patch.crates-io] pins turbojpeg-sys. The
    libjpeg-turbo sources are inside that fork (turbojpeg-sys/libjpeg-turbo), so the app's and Qt's libjpeg come from one
    source: the build reads the pin from Cargo.toml rather than keeping a second copy of it."""
    patch = tomllib.loads((REPO / "Cargo.toml").read_text(encoding="utf-8"))["patch"]["crates-io"]["turbojpeg-sys"]
    return patch["git"], patch["rev"]


FORK_URL, FORK_COMMIT = cargo_fork_pin()


def fetch_pinned(src: Path, log: Path) -> None:
    """Fetches the pinned commit into `src`, retrying: the network drops connections now and then. Three tries; the last
    one runs through checked(), so when every try has failed the program ends in checked()'s message (`git fetch failed
    (exit N)`), and its output is in `log`."""
    command = ["git", "-C", str(src), "fetch", "-q", "--depth", "1", FORK_URL, FORK_COMMIT]
    for attempt in range(1, 3):
        with log.open("w", encoding="utf-8") as file:
            result = subprocess.run(command, stdout=file, stderr=subprocess.STDOUT, check=False)
        if result.returncode == 0:
            return
        time.sleep(10 * attempt)
    checked("git fetch", command, log)


def default_work() -> Path:
    home = os.environ.get("PG_HOME")
    if not home:
        not_set("PG_HOME", "pass --work, or set it in .env to the data folder (static/ goes under it)")
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

    import_msvc_environment(args.build_tools, MSVC_TOOLSET)

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

    # Clean output folders, so each run starts empty. A folder that cannot be removed stops the run: its old files must not
    # be left in the new build.
    for folder in [build, prefix, link]:
        if folder.exists():
            shutil.rmtree(folder)
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

    # Every library the prefix installs (jpeg-static.lib as well as turbojpeg-static.lib, which is the one the process links).
    libraries = sorted((prefix / "lib").glob("*.lib"))
    manifest = {
        "fork_url": FORK_URL,
        "fork_commit": FORK_COMMIT,
        "msvc_toolset": MSVC_TOOLSET,
        "configure": cmake_args[:-1],
        "libraries": [
            {"path": lib.relative_to(prefix).as_posix(), "bytes": lib.stat().st_size, "sha256": sha256(lib)}
            for lib in libraries
        ],
    }
    (work / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"done: {work / 'manifest.json'}")


if __name__ == "__main__":
    main()
