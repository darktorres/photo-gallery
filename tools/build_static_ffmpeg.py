#!/usr/bin/env python3
"""Builds FFmpeg 9.0 as static libraries with MSVC, for pg-video and Qt Multimedia's FFmpeg backend. Windows only; it was
build-static-ffmpeg.ps1.

Reproducible: the inputs are pinned below (the FFmpeg commit, the MSVC toolset and Windows SDK, the MSYS2 packages), and each
one is checked before it is used. The source is fetched by commit hash. Every step's log and the outputs' hashes go to
<work>\\manifest.json.

    python tools/build_static_ffmpeg.py                           # into <PG_HOME>\\static\\ffmpeg
    python tools/build_static_ffmpeg.py --work D:\\scratch\\run2    # a clean run, into another folder

The build needs MSVC and MSYS2 (for FFmpeg's configure and make), both on this machine.
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

HERE = Path(__file__).resolve().parent

# ---- Pins. Change them here, and note the reason beside each one.
FFMPEG_URL = "https://github.com/FFmpeg/FFmpeg.git"
FFMPEG_COMMIT = "7adfbe40dcde598f93a6963d2f7b22cd75202ccc"  # n9.0.2-39-g7adfbe40dc, the commit the pinned build came from
FFMPEG_RELEASE = "9.0.2"  # the RELEASE file at that commit
WINDOWS_SDK = "10.0.26100.0"
MSYS_PACKAGES = {"make": "4.4.1-3", "diffutils": "3.12-1"}  # installed if missing; a different version is recorded
# The Windows libraries the static link needs. FFmpeg's .pc files don't list them (Libs.private is empty).
SYSTEM_LIBS = ["user32.lib", "bcrypt.lib"]

# The configure line. --prefix is added below. Each flag is the pilot's.
CONFIGURE_FLAGS = [
    "--toolchain=msvc", "--enable-static", "--disable-shared",
    "--extra-cflags='-MD -Brepro'",  # the app's dynamic CRT (MSVCRT); -Brepro: same bytes per build
    "--disable-debug",  # no /Z7: its debug data embeds the build folder's source paths
    "--disable-programs", "--disable-doc", "--disable-autodetect", "--disable-network",
    "--disable-avdevice", "--disable-avfilter", "--disable-everything",
    "--enable-protocol=file",
    # The containers of the player's scope (mp4, m4v, mov, 3gp: mov; webm, mkv: matroska; avi; wmv: asf; mpg, mpeg: mpeg),
    # with the video and audio codecs they carry. Every audio codec the player mixes down (AAC, AC-3, E-AC-3, Opus) is here.
    "--enable-demuxer=mov,matroska,avi,asf,mpeg,mpegts,image2",
    "--enable-decoder=h264,hevc,vp9,vp8,mpeg4,mjpeg,mpeg1video,mpeg2video,wmv1,wmv2,wmv3,vc1,"
    "aac,ac3,eac3,opus,vorbis,mp3float,flac",
    "--enable-parser=h264,hevc,vp9,vp8,mpeg4video,mjpeg,mpegvideo,vc1,mpegaudio,aac,ac3,opus,vorbis",
    "--enable-swscale", "--enable-swresample",
    "--enable-d3d11va", "--enable-dxva2",
]  # fmt: skip

# The link check's policy for the DLLs a static-check.exe imports. A DLL is accepted by name, not by prefix, and a name
# that is not listed fails the build (Decisions: a new DLL is added here on purpose, with its reason).
# The DLLs every supported Windows provides, by exact file name (matched case-blind: dumpbin prints mixed case).
SYSTEM_DLLS = frozenset({"kernel32.dll", "user32.dll", "bcrypt.dll"})
# The API-set DLLs: the loader resolves each name to the real DLL on every supported Windows, so the name is the contract.
API_SET_PREFIXES = ("api-ms-win-", "ext-ms-win-")
# The VC runtime. It ships with the Visual C++ redistributable, a deployment requirement, not a part of Windows.
REDISTRIBUTED = frozenset({"vcruntime140.dll"})


def is_system_dll(name: str) -> bool:
    """True for a DLL every supported Windows provides: an exact name in SYSTEM_DLLS, or an API-set name."""
    lower = name.lower()
    return lower in SYSTEM_DLLS or lower.startswith(API_SET_PREFIXES)


def is_redistributed(name: str) -> bool:
    """True for a DLL that ships with the VC redistributable (REDISTRIBUTED), matched by exact name."""
    return name.lower() in REDISTRIBUTED


def git(src: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", "-C", str(src), *args], capture_output=True, text=True, check=False)


def posix(path: Path) -> str:
    """D:\\a\\b -> /d/a/b, for MSYS2's bash."""
    full = str(path.resolve())
    return "/" + full[0].lower() + full[2:].replace("\\", "/")


def default_work() -> Path:
    home = os.environ.get("PG_HOME")
    if not home:
        not_set("PG_HOME", "pass --work, or set it in .env to the data folder (static/ goes under it)")
    return Path(home) / "static" / "ffmpeg"


def setup_environment(build_tools: Path, msys: Path) -> str:
    """The MSVC environment, with MSYS2's usr\\bin after it: MSYS2's link (coreutils) would otherwise shadow MSVC's link.exe.
    Returns cl.exe's path."""
    cl = import_msvc_environment(build_tools, MSVC_TOOLSET)
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

    # 3. Build and install. An earlier run's staging folder would merge its files into this install, so it goes first.
    if staged.exists():
        shutil.rmtree(staged)
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
    # A prefix that survives the removal (its errors are not ignored, so a locked file stops here) would take the moved
    # one inside it.
    if prefix.exists():
        shutil.rmtree(prefix)
    shutil.move(str(staged_prefix), str(prefix))
    shutil.rmtree(staged)

    # 4. The CRT: the libraries must use the dynamic runtime (MSVCRT), never the static one (LIBCMT).
    libs = sorted((prefix / "lib").glob("*.lib"), key=lambda path: path.name)
    crt = []
    for lib in libs:
        data = lib.read_bytes()
        crt.append({"name": lib.name, "msvcrt": data.count(b"MSVCRT"), "libcmt": data.count(b"LIBCMT")})
    if any(entry["libcmt"] > 0 for entry in crt):
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
    non_system = [name for name in dependents if not is_system_dll(name)]
    unlisted = [name for name in non_system if not is_redistributed(name)]
    # The check must not pass vacuously: a failed dumpbin or an unmatched output leaves no dependents to judge.
    problems = []
    if dumped.returncode != 0:
        problems.append(f"dumpbin exited {dumped.returncode}")
    if not dependents:
        problems.append("dumpbin listed no DLLs of static-check.exe (KERNEL32.dll is always among them)")
    if unlisted:
        problems.append(f"the link check imports DLLs that are not system DLLs: {', '.join(unlisted)}")

    # 6. Manifest: what went in and what came out, with hashes.
    manifest = {
        "built": started.isoformat(timespec="seconds"),
        "seconds": int((datetime.datetime.now(datetime.UTC) - started).total_seconds()),
        "ffmpeg": {"commit": head, "release": FFMPEG_RELEASE, "configure": f"./configure {flags} --prefix=/ffbuild/prefix"},
        "tools": {
            "cl": cl,
            "msvc_toolset": MSVC_TOOLSET,
            "windows_sdk": WINDOWS_SDK,
            "msys2_pacman": [f"{name} {version}" for name, version in msys_versions.items()],
            "msys2_root": str(args.msys),
        },
        "configure": {
            "license": license_line.group(0) if license_line else "",
            "disabled_with_warning": disabled,
        },
        "libraries": [
            {"path": lib.relative_to(prefix).as_posix(), "bytes": lib.stat().st_size, "sha256": sha256(lib)} for lib in libs
        ],
        "crt": crt,
        "link_check": {"output": output, "dependents": dependents, "non_system": non_system, "unlisted": unlisted},
    }
    (work / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    # Written first, so a failed run is still recorded; the failure is raised before the done line.
    if problems:
        sys.exit(f"link check failed: {'; '.join(problems)} (the run is recorded in {work / 'manifest.json'})")
    print(f"done: {work / 'manifest.json'}")


if __name__ == "__main__":
    main()
