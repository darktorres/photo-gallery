#!/usr/bin/env python3
"""Builds FFmpeg 9.0 as static libraries with MSVC, for pg-video and Qt Multimedia's FFmpeg backend. Windows only; it was
build-static-ffmpeg.ps1.

Reproducible: the inputs are pinned below (the FFmpeg commit, the MSVC toolset and Windows SDK, the MSYS2 packages), and each
one is checked before it is used. The source is fetched by commit hash. Every step's log and the outputs' hashes go to
<work>\\manifest.json. The reasons for each step are in docs/STATIC_BUILD.md.

    python tools/build_static_ffmpeg.py                           # into <PG_HOME>\\static\\ffmpeg
    python tools/build_static_ffmpeg.py --work D:\\scratch\\run2    # a clean run, into another folder

The build needs MSVC and MSYS2 (for FFmpeg's configure and make), both on this machine.
"""

import argparse
import datetime
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

HERE = Path(__file__).resolve().parent

# ---- Pins. Change them here, and record the reason in docs/STATIC_BUILD.md. --------------------------------------------
FFMPEG_URL = "https://github.com/FFmpeg/FFmpeg.git"
FFMPEG_COMMIT = "7adfbe40dcde598f93a6963d2f7b22cd75202ccc"  # n9.0.2-39-g7adfbe40dc, the commit the pinned build came from
FFMPEG_RELEASE = "9.0.2"  # the RELEASE file at that commit
TOOLSET = "14.44.35207"  # MSVC toolset (vcvars64 -vcvars_ver)
WINDOWS_SDK = "10.0.26100.0"
MSYS_PACKAGES = {"make": "4.4.1-3", "diffutils": "3.12-1"}  # installed if missing; a different version is recorded
# The Windows libraries the static link needs. FFmpeg's .pc files don't list them (Libs.private is empty).
SYSTEM_LIBS = ["user32.lib", "bcrypt.lib"]
DEFAULT_BUILD_TOOLS = Path(r"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools")
DEFAULT_MSYS = Path(r"C:\tools\msys64")

# The configure line. --prefix is added below. Each flag is the pilot's, see docs/STATIC_BUILD.md.
CONFIGURE_FLAGS = [
    "--toolchain=msvc", "--enable-static", "--disable-shared",
    "--extra-cflags='-MD -Brepro'",  # the app's dynamic CRT (MSVCRT); -Brepro: same bytes per build
    "--disable-debug",  # no /Z7: its debug data embeds the build folder's source paths
    "--disable-programs", "--disable-doc", "--disable-autodetect", "--disable-network",
    "--disable-avdevice", "--disable-avfilter", "--disable-everything",
    "--enable-protocol=file",
    "--enable-demuxer=mov,matroska,avi,mpegts,image2",
    "--enable-decoder=h264,hevc,vp9,mpeg4,mjpeg",
    "--enable-parser=h264,hevc,vp9,mpeg4video,mjpeg",
    "--enable-swscale", "--enable-swresample",
    "--enable-d3d11va", "--enable-dxva2",
]  # fmt: skip

SYSTEM_DLL = re.compile(r"^(api-ms-win-|KERNEL32|USER32|bcrypt|VCRUNTIME140|MSVCP140)", re.IGNORECASE)


def checked(what: str, command: list[str], log: Path | None = None, cwd: Path | None = None) -> None:
    """Runs `command`, its output in `log` when given; a failure ends this program."""
    if log is None:
        result = subprocess.run(command, cwd=cwd, check=False)
    else:
        with log.open("w", encoding="utf-8") as file:
            result = subprocess.run(command, cwd=cwd, stdout=file, stderr=subprocess.STDOUT, check=False)
    if result.returncode != 0:
        sys.exit(f"{what} failed (exit {result.returncode})")


def git(src: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", "-C", str(src), *args], capture_output=True, text=True, check=False)


def posix(path: Path) -> str:
    """D:\\a\\b -> /d/a/b, for MSYS2's bash."""
    full = str(path.resolve())
    return "/" + full[0].lower() + full[2:].replace("\\", "/")


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
    return Path(home) / "static" / "ffmpeg"


def setup_environment(build_tools: Path, msys: Path) -> str:
    """The MSVC environment, with MSYS2's usr\\bin after it: MSYS2's link (coreutils) would otherwise shadow MSVC's link.exe.
    Returns cl.exe's path."""
    cl = import_msvc_environment(build_tools, TOOLSET)
    os.environ["PATH"] = os.pathsep.join([str(Path(cl).parent), str(msys / "usr" / "bin"), os.environ.get("PATH", "")])
    os.environ["MSYS2_PATH_TYPE"] = "inherit"
    program_files = os.environ.get("PROGRAMFILES(X86)", r"C:\Program Files (x86)")  # Windows' environment is case-blind
    dxva = Path(program_files) / "Windows Kits" / "10" / "Include" / WINDOWS_SDK / "um" / "dxva.h"
    if not dxva.is_file():
        sys.exit(f"Windows SDK {WINDOWS_SDK} not installed (dxva.h missing)")
    return cl


def pacman_versions(pacman: Path) -> dict[str, str]:
    """The installed version of each pinned MSYS2 package (absent when not installed)."""
    installed: dict[str, str] = {}
    for name in MSYS_PACKAGES:
        result = subprocess.run([str(pacman), "-Q", name], capture_output=True, text=True, check=False)
        if result.returncode == 0 and result.stdout.split():
            installed[name] = result.stdout.split()[1]
    return installed


def check_msys_packages(msys: Path, logs: Path) -> dict[str, str]:
    """Installs the pinned MSYS2 packages that are missing, and warns about any whose version differs from the pin."""
    pacman = msys / "usr" / "bin" / "pacman.exe"
    if not pacman.is_file():
        sys.exit(f"MSYS2 not found at {msys}")
    installed = pacman_versions(pacman)
    missing = [name for name in MSYS_PACKAGES if name not in installed]
    if missing:
        checked("pacman -Sy", [str(pacman), "-Sy", "--noconfirm"], logs / "pacman-sync.log")
        checked("pacman -S", [str(pacman), "-S", "--needed", "--noconfirm", *missing], logs / "pacman-install.log")
        installed = pacman_versions(pacman)
    different = [name for name in MSYS_PACKAGES if installed.get(name) != MSYS_PACKAGES[name]]
    if different:
        described = ", ".join(f"{name} {installed.get(name, 'missing')} (pinned {MSYS_PACKAGES[name]})" for name in different)
        print(f"warning: MSYS2 packages differ from the pins: {described}", file=sys.stderr)
    return installed


def fetch_source(src: Path, logs: Path) -> str:
    """The FFmpeg source at the pinned commit, checked out and cleaned. Returns the HEAD it found, and the RELEASE file's text
    is checked here too."""
    if not (src / ".git").is_dir():
        checked("git init", ["git", "init", "-q", str(src)])
    if "origin" not in git(src, "remote").stdout.split():
        git(src, "remote", "add", "origin", FFMPEG_URL)
    # The network drops connections now and then: retry the fetch a few times before failing.
    for attempt in range(1, 4):
        with (logs / "git-fetch.log").open("w", encoding="utf-8") as file:
            result = subprocess.run(
                ["git", "-C", str(src), "fetch", "-q", "--depth", "1", "origin", FFMPEG_COMMIT],
                stdout=file,
                stderr=subprocess.STDOUT,
                check=False,
            )
        if result.returncode == 0:
            break
        if attempt == 3:
            sys.exit(f"git fetch failed after 3 attempts (exit {result.returncode}): see {logs / 'git-fetch.log'}")
        time.sleep(10 * attempt)
    checked("git checkout", ["git", "-C", str(src), "checkout", "-q", "FETCH_HEAD"])
    head = git(src, "rev-parse", "HEAD").stdout.strip()
    if head != FFMPEG_COMMIT:
        sys.exit(f"source HEAD is {head}, not the pin {FFMPEG_COMMIT}")
    # The build is in-tree: start from the pinned commit with nothing left from an earlier build.
    checked("git clean", ["git", "-C", str(src), "clean", "-fdxq"])
    checked("git reset", ["git", "-C", str(src), "reset", "-q", "--hard", FFMPEG_COMMIT])
    release = (src / "RELEASE").read_text(encoding="utf-8").strip()
    if release != FFMPEG_RELEASE:
        sys.exit(f"RELEASE is {release}, not {FFMPEG_RELEASE}")
    return head


def run_bash(bash: Path, script: Path, what: str) -> None:
    checked(what, [str(bash), "--noprofile", "--norc", posix(script)])


def write_script(path: Path, text: str) -> None:
    """Writes a shell script with LF line endings: MSYS2's bash reads CRLF as a broken command."""
    path.write_text(text, encoding="utf-8", newline="\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    parser.add_argument("--work", type=Path, default=None, help="the work folder (default: PG_HOME's static\\ffmpeg)")
    parser.add_argument("--build-tools", type=Path, default=DEFAULT_BUILD_TOOLS, help="the Visual Studio Build Tools folder")
    parser.add_argument("--msys", type=Path, default=DEFAULT_MSYS, help="the MSYS2 root")
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    args = parser.parse_args()
    if not WINDOWS:
        sys.exit("build_static_ffmpeg.py: Windows only (MSVC and MSYS2)")
    load_env_file()

    started = datetime.datetime.now(datetime.UTC)
    work = (args.work or default_work()).resolve()
    logs, test = work / "logs", work / "test"
    for folder in [logs, test]:
        folder.mkdir(parents=True, exist_ok=True)
    src, prefix, staged = work / "src", work / "prefix", work / "staged"

    cl = setup_environment(args.build_tools, args.msys)
    print(f"cl: {cl}")
    msys_versions = check_msys_packages(args.msys, logs)

    # 1. Source, by commit.
    head = fetch_source(src, logs)

    # 2. Configure, in the source folder (an in-tree build). Two reasons, both from the reproducibility runs:
    #    - __FILE__ (assertions, log messages) gets the source path as passed to the compiler: relative in-tree, absolute
    #      out of tree, and the absolute one would put the work folder into every library.
    #    - FFMPEG_CONFIGURATION, compiled into avutil, holds --prefix. The prefix is fixed (/ffbuild/prefix, as the pinned
    #      build has it) and install goes to a staging folder with DESTDIR, which doesn't appear in the libraries.
    flags = " ".join(CONFIGURE_FLAGS)
    bash = args.msys / "usr" / "bin" / "bash.exe"
    configure_sh = work / "configure.sh"
    write_script(
        configure_sh,
        "#!/bin/sh\n"
        "set -eo pipefail\n"
        f"cd '{posix(src)}'\n"
        f"./configure {flags} --prefix=/ffbuild/prefix 2>&1 | tee '{posix(work)}/logs/configure.log'\n",
    )
    run_bash(bash, configure_sh, "configure")

    configure_log = (logs / "configure.log").read_text(encoding="utf-8", errors="replace")
    if "cmp: command not found" in configure_log:
        sys.exit("configure ran without cmp (diffutils missing): the result cannot be trusted")
    disabled = re.findall(r"WARNING: Disabled [^\r\n]+", configure_log)
    license_line = re.search(r"License: [^\r\n]+", configure_log)

    # 3. Build and install.
    build_sh = work / "build.sh"
    write_script(
        build_sh,
        "#!/bin/sh\n"
        "set -eo pipefail\n"
        f"cd '{posix(src)}'\n"
        f"make -j{args.jobs} ARFLAGS='-nologo -Brepro' 2>&1 | tee '{posix(work)}/logs/build.log'\n"
        f"make install DESTDIR='{posix(staged)}' 2>&1 | tee '{posix(work)}/logs/install.log'\n",
    )
    run_bash(bash, build_sh, "make")

    # The install went to <staged>\ffbuild\prefix (the fixed prefix under DESTDIR). Move it to the work folder's prefix.
    staged_prefix = staged / "ffbuild" / "prefix"
    if not (staged_prefix / "lib").is_dir():
        sys.exit(f"install produced nothing under {staged_prefix}")
    shutil.rmtree(prefix, ignore_errors=True)
    shutil.move(str(staged_prefix), str(prefix))
    shutil.rmtree(staged, ignore_errors=True)

    # 4. The CRT: the libraries must use the dynamic runtime (MSVCRT), never the static one (LIBCMT).
    libs = sorted((prefix / "lib").glob("*.lib"), key=lambda path: path.name)
    crt = []
    for lib in libs:
        data = lib.read_bytes()
        crt.append({"Name": lib.name, "MSVCRT": data.count(b"MSVCRT"), "LIBCMT": data.count(b"LIBCMT")})
    if any(entry["LIBCMT"] > 0 for entry in crt):
        sys.exit("a library references LIBCMT (static CRT): see the manifest")

    # 5. Link check: a program that calls into each library, linked with the system libraries, then run.
    shutil.copyfile(HERE / "static-ffmpeg" / "static-check.c", test / "static-check.c")
    libraries = ["avformat.lib", "avcodec.lib", "avutil.lib", "swresample.lib", "swscale.lib", *SYSTEM_LIBS]
    link_command = [
        "cl.exe", "/nologo", "/MD", "/W3", f"/I{prefix / 'include'}", "static-check.c", "/Fe:static-check.exe",
        "/link", f"/LIBPATH:{prefix / 'lib'}", *libraries,
    ]  # fmt: skip
    checked("cl (link check)", link_command, logs / "link-check.log", cwd=test)
    run = subprocess.run([str(test / "static-check.exe")], capture_output=True, text=True, check=False, cwd=test)
    output = run.stdout.splitlines()
    if run.returncode != 0:
        sys.exit(f"static-check.exe exited {run.returncode}")
    for expected in ["h264 decoder: yes", "d3d11va hwdevice: yes", "dxva2 hwdevice: yes", "open missing file: -2"]:
        if not any(expected in line for line in output):
            sys.exit(f"static-check output lacks: {expected}")
    dumped = subprocess.run(
        ["dumpbin.exe", "/nologo", "/dependents", "static-check.exe"], capture_output=True, text=True, check=False, cwd=test
    )
    dependents = [line.strip() for line in dumped.stdout.splitlines() if re.match(r"^\s+\S+\.dll$", line, re.IGNORECASE)]
    non_system = [name for name in dependents if not SYSTEM_DLL.match(name)]

    # 6. Manifest: what went in and what came out, with hashes.
    manifest = {
        "built": started.isoformat(timespec="seconds"),
        "seconds": int((datetime.datetime.now(datetime.UTC) - started).total_seconds()),
        "ffmpeg": {"commit": head, "release": FFMPEG_RELEASE, "configure": f"./configure {flags} --prefix=/ffbuild/prefix"},
        "tools": {
            "cl": cl,
            "msvc_toolset": TOOLSET,
            "windows_sdk": WINDOWS_SDK,
            "msys2_pacman": [f"{name} {version}" for name, version in msys_versions.items()],
            "msys2_root": str(args.msys),
        },
        "configure": {
            "license": license_line.group(0) if license_line else "",
            "disabled_with_warning": disabled,
        },
        "libraries": [{"name": lib.name, "bytes": lib.stat().st_size, "sha256": sha256(lib)} for lib in libs],
        "crt": crt,
        "link_check": {"output": output, "dependents": dependents, "non_system": non_system},
    }
    (work / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"done: {work / 'manifest.json'}")
    if non_system:
        print(f"warning: the link check imports non-system DLLs: {', '.join(non_system)}", file=sys.stderr)


if __name__ == "__main__":
    main()
