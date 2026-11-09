#!/usr/bin/env python3
"""Checks this machine for what the build and the tests need, and says how to fix each missing part (`just doctor`).
Run it through `just`, so the values in .env count (a direct run reads .env here too). It builds nothing and changes nothing.

Required parts: cargo, the Qt kit, FFmpeg (the headers the video build reads, and on Windows the DLLs the app loads), and
libclang (bindgen loads it). Optional parts are used by some tests and by the Linux checks (the container); a missing
optional part only says so. Exit status 1 when a required part is missing.
"""

import glob
import os
import shutil
import subprocess
import sys
from pathlib import Path

from pgtools import REPO, WINDOWS, load_env_file

# The FFmpeg libraries pg-video uses, as pkg-config names them (crates/video/build.rs).
FFMPEG_LIBS = ["libavcodec", "libavformat", "libavutil", "libswscale", "libswresample"]


class Doctor:
    def __init__(self) -> None:
        self.missing_required = False

    @staticmethod
    def show(status: str, name: str, detail: str) -> None:
        print(f"{status:<9} {name:<12} {detail}")

    def require(self, ok: bool, name: str, ok_detail: str, fix: str) -> None:
        if ok:
            self.show("ok", name, ok_detail)
        else:
            self.show("MISSING", name, fix)
            self.missing_required = True

    def optional(self, ok: bool, name: str, ok_detail: str, fix: str) -> None:
        if ok:
            self.show("ok", name, ok_detail)
        else:
            self.show("optional", name, fix)


def qt_kit() -> str:
    """The Qt kit this OS builds against: msvc2022_64 on Windows, gcc_64 elsewhere."""
    return "msvc2022_64" if WINDOWS else "gcc_64"


def ffmpeg_prefix() -> Path | None:
    """The FFmpeg prefix named by FFMPEG_DIR, else the pinned build `just setup` fetches (Windows only); None on Linux, where
    the development packages are found through pkg-config."""
    named = os.environ.get("FFMPEG_DIR")
    if named:
        return Path(named)
    return REPO / ".deps" / "ffmpeg" if WINDOWS else None


def ffmpeg_ok(prefix: Path | None) -> bool:
    if prefix is not None:
        headers = (prefix / "include" / "libavcodec" / "avcodec.h").is_file()
        return headers and (not WINDOWS or (prefix / "bin").is_dir())
    if shutil.which("pkg-config") is None:
        return False
    return subprocess.run(["pkg-config", "--exists", *FFMPEG_LIBS], check=False).returncode == 0


def libclang_dirs() -> list[Path]:
    """The folders libclang is looked for in, in order: LIBCLANG_PATH, then the usual install places."""
    dirs = []
    if os.environ.get("LIBCLANG_PATH"):
        dirs.append(Path(os.environ["LIBCLANG_PATH"]))
    if WINDOWS:
        if os.environ.get("LLVM_DIR"):
            dirs.append(Path(os.environ["LLVM_DIR"]) / "bin")
        if os.environ.get("ProgramFiles"):
            dirs.append(Path(os.environ["ProgramFiles"]) / "LLVM" / "bin")
        dirs.extend(Path(p) for p in os.environ.get("PATH", "").split(os.pathsep) if p)
    else:
        dirs.extend(Path(p) for p in sorted(glob.glob("/usr/lib/llvm-*/lib")))
        dirs.append(Path("/usr/lib/x86_64-linux-gnu"))
    return dirs


def libclang_in(folder: Path) -> bool:
    if WINDOWS:
        return (folder / "libclang.dll").is_file()
    return bool(glob.glob(str(folder / "libclang*.so*")))


def main() -> None:
    load_env_file()
    doctor = Doctor()
    if (REPO / ".env").is_file():
        doctor.show("ok", ".env", "read by just")
    else:
        doctor.show("note", ".env", "none: copy .env.example to .env to set this machine's paths")

    cargo = shutil.which("cargo")
    doctor.require(
        cargo is not None,
        "cargo",
        cargo or "",
        "install Rust with rustup (https://rustup.rs); the toolchain comes from rust-toolchain.toml",
    )

    # Qt: the kit's qmake is what cxx-qt-build runs.
    qt = os.environ.get("QT_DIR", "")
    qmake = Path(qt) / "bin" / ("qmake.exe" if WINDOWS else "qmake") if qt else None
    doctor.require(
        qmake is not None and qmake.is_file(),
        "Qt (QT_DIR)",
        qt,
        f"set QT_DIR in .env to the Qt 6.12.0 {qt_kit()} kit (the folder with bin/qmake), from the Qt Online Installer",
    )

    # FFmpeg: the headers are what the video build generates its bindings from; on Windows the DLLs are what the app loads.
    prefix = ffmpeg_prefix()
    where = str(prefix) if prefix is not None else "pkg-config (the FFmpeg development packages)"
    if WINDOWS:
        fix = "run `just setup` to fetch the pinned build, or set FFMPEG_DIR in .env to an FFmpeg 9.0 shared build"
    else:
        fix = "install the FFmpeg development packages (docker/apt-packages.txt), or set FFMPEG_DIR in .env"
    doctor.require(ffmpeg_ok(prefix), "FFmpeg", where, fix)

    # libclang: bindgen loads it. LIBCLANG_PATH names its folder; on Windows LLVM_DIR is the LLVM install.
    clang = next((folder for folder in libclang_dirs() if libclang_in(folder)), None)
    if WINDOWS:
        clang_fix = (
            "install LLVM (winget install LLVM.LLVM), then set LIBCLANG_PATH in .env to its bin folder "
            "(the one with libclang.dll)"
        )
    else:
        clang_fix = "install libclang (apt: libclang-dev), or set LIBCLANG_PATH in .env to the folder with libclang.so"
    doctor.require(clang is not None, "libclang", str(clang or ""), clang_fix)

    # Optional: the tests that need ExifTool and ffmpeg skip without them, unless CI's PG_REQUIRE_TOOLS makes them fail.
    exiftool = shutil.which(os.environ.get("PG_EXIFTOOL") or "exiftool")
    doctor.optional(
        exiftool is not None,
        "ExifTool",
        exiftool or "",
        "the metadata tests skip without it: install ExifTool, or set PG_EXIFTOOL in .env to its exe",
    )

    if os.environ.get("PG_FFMPEG"):
        ffmpeg_exe = os.environ["PG_FFMPEG"]
    elif WINDOWS and prefix is not None:
        ffmpeg_exe = str(prefix / "bin" / "ffmpeg.exe")
    else:
        ffmpeg_exe = "ffmpeg"
    ffmpeg = shutil.which(ffmpeg_exe)
    doctor.optional(
        ffmpeg is not None,
        "ffmpeg exe",
        ffmpeg or "",
        "the video tests skip without it: set PG_FFMPEG in .env to the ffmpeg executable, or run `just setup` (Windows)",
    )

    # Optional: Docker, which the Linux checks run in (`just check-linux`, `just perf`, docker/README.md).
    docker_up = False
    if shutil.which("docker"):
        probe = ["docker", "version", "--format", "{{.Server.Version}}"]
        docker_up = subprocess.run(probe, capture_output=True, check=False).returncode == 0
    doctor.optional(
        docker_up,
        "Docker",
        "running",
        "the Linux checks, benches and fuzzer run in a container: start Docker, then `just linux-build`",
    )

    if doctor.missing_required:
        print()
        print("Missing required parts: fix them above, then run `just doctor` again.")
        sys.exit(1)
    print()
    print("Every required part is in place.")


if __name__ == "__main__":
    main()
