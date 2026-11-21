#!/usr/bin/env python3
"""Builds Qt 6.12.0 as static libraries with MSVC, for the app's static build. Windows only.

Two inputs come from the other static recipes, and each is checked before the configure: the libjpeg-turbo prefix
(build_jpeg_turbo.py, for -system-libjpeg) and the static FFmpeg prefix (build_static_ffmpeg.py, for Qt Multimedia's FFmpeg
backend). The source is Qt's single-archive zip, checked by SHA-256 before it is unpacked.

    python tools/build_static_qt.py --jpeg <jpeg>\\prefix --ffmpeg <ffmpeg>\\prefix          # the whole build, about an hour
    python tools/build_static_qt.py --jpeg ... --ffmpeg ... --configure-only              # stop after configure

The work folder (default: PG_HOME's static\\qt, or --work) holds downloads\\, the source, build\\ (CMake's build tree),
prefix\\ (the install: the app's PG_QT_DIR), logs\\ and manifest.json. Put it on a short path: the source's paths are long.
"""

import argparse
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.request
import zipfile
from pathlib import Path

from pgtools import (
    DEFAULT_BUILD_TOOLS,
    DEFAULT_MSYS,
    MSVC_TOOLSET,
    WINDOWS,
    checked,
    import_msvc_environment,
    load_env_file,
    not_set,
    sha256,
)

# ---- Pins. Change them here, and note the reason beside each one. --------------------------------------
QT_VERSION = "6.12.0"
SOURCE_NAME = f"qt-everywhere-src-{QT_VERSION}"
# Qt publishes an MD5 file, not a SHA-256 one: the SHA-256 here was computed from the zip (its MD5 matches Qt's md5sums.txt).
SOURCE_URL = f"https://download.qt.io/official_releases/qt/6.12/{QT_VERSION}/single/{SOURCE_NAME}.zip"
SOURCE_BYTES = 604_643_023
SOURCE_SHA256 = "A72E89C33726D742E67EDF494ABF7B625EAFFC756673221C9823322ECD1A32D1"  # upper case, as sha256() gives it
# The modules the app uses. qtbase carries the platforms, the image formats' base, the TLS and network plugins.
SUBMODULES = ["qtbase", "qtshadertools", "qtdeclarative", "qtmultimedia", "qtimageformats", "qtsvg"]
# Each of these folders must hold static plugin libraries after the install, or the app's plugin list cannot be linked.
PLUGIN_FOLDERS = ["platforms", "imageformats", "multimedia", "tls", "iconengines"]
TOOLS = ["cmake", "ninja", "perl"]


def pg_home_folder(name: str) -> Path:
    home = os.environ.get("PG_HOME")
    if not home:
        not_set("PG_HOME", "pass --work and the prefixes, or set it in .env to the data folder")
    return Path(home) / "static" / name


def download(archive: Path) -> None:
    """The source zip, from download.qt.io, with three tries (the network drops connections). A file already there is kept
    only if its size and SHA-256 are the pins."""
    if archive.is_file() and archive.stat().st_size == SOURCE_BYTES and sha256(archive) == SOURCE_SHA256:
        return
    archive.parent.mkdir(parents=True, exist_ok=True)
    partial = archive.with_name(archive.name + ".part")
    for attempt in range(1, 4):
        try:
            with urllib.request.urlopen(SOURCE_URL, timeout=60) as response, partial.open("wb") as file:
                shutil.copyfileobj(response, file, 1 << 20)
            break
        except OSError as error:
            if attempt == 3:
                sys.exit(f"download of {SOURCE_URL} failed after 3 attempts: {error}")
            time.sleep(10 * attempt)
    size, actual = partial.stat().st_size, sha256(partial)
    if size != SOURCE_BYTES or actual != SOURCE_SHA256:
        partial.unlink()
        sys.exit(f"the download is {size} bytes with SHA-256 {actual}, not the pins ({SOURCE_BYTES} bytes, {SOURCE_SHA256})")
    partial.replace(archive)


def extract(archive: Path, work: Path) -> Path:
    """The source tree, unpacked into `work` unless it is already there (a run that was interrupted leaves no configure.bat)."""
    source = work / SOURCE_NAME
    if (source / "configure.bat").is_file():
        return source
    shutil.rmtree(source, ignore_errors=True)
    with zipfile.ZipFile(archive) as zipped:
        zipped.extractall(work)
    if not (source / "configure.bat").is_file():
        sys.exit(f"{archive} does not unpack to {source}")
    return source


def check_prefixes(jpeg: Path, ffmpeg: Path) -> None:
    """The two prefixes the configure reads: their headers, and the libraries it links."""
    jconfig = jpeg / "include" / "jconfig.h"
    if not jconfig.is_file() or not re.search(
        r"#define\s+JPEG_LIB_VERSION\s+80\b", jconfig.read_text(encoding="utf-8", errors="replace")
    ):
        sys.exit(f"{jpeg} is not the libjpeg-turbo prefix with the JPEG 8.0 API (run tools/build_jpeg_turbo.py)")
    if not (jpeg / "lib" / "turbojpeg-static.lib").is_file():
        sys.exit(f"{jpeg}\\lib\\turbojpeg-static.lib is missing (run tools/build_jpeg_turbo.py)")
    if not (ffmpeg / "include" / "libavcodec" / "avcodec.h").is_file() or not (ffmpeg / "lib" / "avcodec.lib").is_file():
        sys.exit(f"{ffmpeg} is not the static FFmpeg prefix (run tools/build_static_ffmpeg.py)")


def check_tools() -> None:
    missing = [tool for tool in TOOLS if shutil.which(tool) is None]
    if missing:
        sys.exit(f"not on PATH: {', '.join(missing)} (Qt's configure and build need them)")


def configure(source: Path, build: Path, prefix: Path, jpeg: Path, ffmpeg: Path, log: Path) -> None:
    """Qt's configure, out of tree. The JPEG options point the system-libjpeg at the same libjpeg-turbo the app links."""
    # The libjpeg archive is the app's own (turbojpeg-static.lib carries jpeg_* too, so Qt and the app share one copy).
    flags = [
        "-prefix", str(prefix), "-static", "-release", "-opensource", "-confirm-license",
        "-nomake", "tests", "-nomake", "examples",
        "-no-openssl", "-schannel",  # the TLS backend is Schannel, as the deployed app's is
        "-ffmpeg-dir", str(ffmpeg),
        "-system-libjpeg", "-feature-system-jpeg",  # -feature-: Qt's own check of JPEG_FOUND fails in its scope
        "-submodules", ",".join(SUBMODULES),
        "--",  # what follows goes to CMake
        f"-DJPEG_INCLUDE_DIR={(jpeg / 'include').as_posix()}",
        f"-DJPEG_LIBRARY_RELEASE={(jpeg / 'lib' / 'turbojpeg-static.lib').as_posix()}",
    ]  # fmt: skip
    # cmd.exe runs the batch file; no argument has a space (the work folder is checked for that), so none is quoted.
    checked("configure", ["cmd.exe", "/d", "/c", str(source / "configure.bat"), *flags], log, cwd=build)


def check_install(prefix: Path) -> dict[str, object]:
    """The install's checks: no DLLs (the libraries are static), the qmake version, and the plugin folders."""
    dlls = sorted(path.name for path in (prefix / "bin").glob("*.dll")) if (prefix / "bin").is_dir() else []
    if dlls:
        sys.exit(f"the install has DLLs, so it is not a static build: {', '.join(dlls)}")
    qmake = subprocess.run([str(prefix / "bin" / "qmake.exe"), "-v"], capture_output=True, text=True, check=False)
    if qmake.returncode != 0 or f"Qt version {QT_VERSION}" not in qmake.stdout:
        sys.exit(f"bin\\qmake.exe does not report Qt {QT_VERSION}: {qmake.stdout.strip()} {qmake.stderr.strip()}")
    for folder in PLUGIN_FOLDERS:
        if not any((prefix / "plugins" / folder).glob("*.lib")):
            sys.exit(f"plugins\\{folder} holds no static plugin library")
    if not (prefix / "lib" / "Qt6Core.lib").is_file():
        sys.exit("lib\\Qt6Core.lib is missing")
    return {"qmake": qmake.stdout.strip(), "dlls": dlls}


def main() -> None:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    parser.add_argument("--work", type=Path, default=None, help="the work folder (default: PG_HOME's static\\qt)")
    parser.add_argument(
        "--jpeg", type=Path, default=None, help="the libjpeg-turbo prefix (default: PG_HOME's static\\jpeg-turbo\\prefix)"
    )
    parser.add_argument(
        "--ffmpeg", type=Path, default=None, help="the static FFmpeg prefix (default: PG_HOME's static\\ffmpeg\\prefix)"
    )
    parser.add_argument("--build-tools", type=Path, default=DEFAULT_BUILD_TOOLS, help="the Visual Studio Build Tools folder")
    parser.add_argument("--msys", type=Path, default=DEFAULT_MSYS, help="the MSYS2 root (its usr\\bin has Perl)")
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    parser.add_argument("--configure-only", action="store_true", help="stop after configure (the build takes about an hour)")
    args = parser.parse_args()
    if not WINDOWS:
        sys.exit("build_static_qt.py: Windows only (MSVC)")
    load_env_file()

    started = datetime.datetime.now(datetime.UTC)
    work = (args.work or pg_home_folder("qt")).resolve()
    jpeg = (args.jpeg or pg_home_folder("jpeg-turbo") / "prefix").resolve()
    ffmpeg = (args.ffmpeg or pg_home_folder("ffmpeg") / "prefix").resolve()
    for folder in (work, jpeg, ffmpeg):
        if " " in str(folder):
            sys.exit(f"{folder} has a space: the configure's arguments are not quoted, so use a folder without one")
    check_prefixes(jpeg, ffmpeg)

    logs, build, prefix = work / "logs", work / "build", work / "prefix"
    logs.mkdir(parents=True, exist_ok=True)
    cl = import_msvc_environment(args.build_tools, MSVC_TOOLSET)
    # MSYS2's usr\bin, for Perl, after MSVC's folder: MSYS2's own link (coreutils) would otherwise shadow MSVC's link.exe.
    os.environ["PATH"] = os.pathsep.join([str(Path(cl).parent), str(args.msys / "usr" / "bin"), os.environ.get("PATH", "")])
    check_tools()
    print(f"cl: {cl}")

    archive = work / "downloads" / f"{SOURCE_NAME}.zip"
    download(archive)
    source = extract(archive, work)

    # Configure out of tree, into a folder that starts empty. Install goes to prefix, which the configure is told.
    shutil.rmtree(build, ignore_errors=True)
    build.mkdir(parents=True)
    shutil.rmtree(prefix, ignore_errors=True)
    configure(source, build, prefix, jpeg, ffmpeg, logs / "configure.log")
    if args.configure_only:
        print(f"configured: {build}")
        return

    checked("cmake build", ["cmake", "--build", ".", "--parallel", str(args.jobs)], logs / "build.log", cwd=build)
    checked("cmake install", ["cmake", "--install", "."], logs / "install.log", cwd=build)

    install = check_install(prefix)
    libraries = sorted(
        [*(prefix / "lib").glob("*.lib"), *(prefix / "plugins").rglob("*.lib")], key=lambda path: str(path).lower()
    )
    manifest = {
        "built": started.isoformat(timespec="seconds"),
        "seconds": int((datetime.datetime.now(datetime.UTC) - started).total_seconds()),
        "qt": {"version": QT_VERSION, "source": SOURCE_URL, "source_sha256": SOURCE_SHA256},
        "tools": {"cl": cl, "msvc_toolset": MSVC_TOOLSET, "jobs": args.jobs},
        "inputs": {"jpeg": str(jpeg), "ffmpeg": str(ffmpeg)},
        "configure_log": str(logs / "configure.log"),
        "install": install,
        "libraries": [
            {"path": path.relative_to(prefix).as_posix(), "bytes": path.stat().st_size, "sha256": sha256(path)} for path in libraries
        ],
    }
    (work / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"done: {work / 'manifest.json'}")


if __name__ == "__main__":
    main()
