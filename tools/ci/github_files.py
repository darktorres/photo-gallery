#!/usr/bin/env python3
"""Writes to the files GitHub Actions reads between steps, on every OS: `env NAME VALUE` appends to GITHUB_ENV (the later
steps' environment) and `path DIR` appends to GITHUB_PATH (their PATH; a relative DIR is taken from the workspace). A step
calls it from any shell, so CI's steps need no PowerShell for it.

    python tools/ci/github_files.py env PG_DESKTOP_CONTENT_KEY <hash>
    python tools/ci/github_files.py path .deps/ffmpeg/bin
    python tools/ci/github_files.py pins        # the first step of each job: the Justfile's pins, as its environment
"""

import argparse
import os
import re
import sys
from pathlib import Path

# The Justfile's pins for CI come from pgtools (the Justfile's text, before `just` is installed).
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from pgtools import LINUX_QT_KIT, WINDOWS_QT_KIT

NAME = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


def env_line(name: str, value: str) -> str:
    """The GITHUB_ENV line for NAME=VALUE. A value with a line break would write a second variable, so it is refused."""
    if not NAME.fullmatch(name):
        sys.exit(f"not an environment variable name: {name!r}")
    if "\n" in value or "\r" in value:
        sys.exit(f"the value of {name} has a line break")
    return f"{name}={value}\n"


def path_line(directory: str, workspace: Path) -> str:
    """The GITHUB_PATH line for DIRECTORY: a relative one is taken from the workspace, and the line is absolute."""
    path = Path(directory)
    return f"{path if path.is_absolute() else workspace / path}\n"


def append(variable: str, text: str) -> None:
    target = os.environ.get(variable)
    if not target:
        sys.exit(f"{variable} is not set: this runs in a GitHub Actions step")
    with Path(target).open("a", encoding="utf-8", newline="\n") as file:
        file.write(text)


def pin_values() -> list[tuple[str, str]]:
    """The pins the jobs use, as (NAME, VALUE) for the later steps' environment, read from the Justfile (pgtools.just_value)
    and from the platform this runs on. The Qt kit is installed under QT_INSTALL_ROOT (the install step's `dir`): C:\\ on
    Windows, the runner's temp folder on Linux. PG_QT_DIR is the kit folder inside it, the one `just` recipes and cxx-qt use.
    """
    from pgtools import just_value

    qt = just_value("qt_version")
    if sys.platform == "win32":
        root, kit, arch = "C:\\", WINDOWS_QT_KIT, "win64_msvc2022_64"
        sep = "\\"
    else:
        temp = os.environ.get("RUNNER_TEMP")
        if not temp:
            sys.exit("RUNNER_TEMP is not set: the pins for Linux are written in a GitHub Actions step")
        root, kit, arch = temp + "/", LINUX_QT_KIT, "linux_gcc_64"
        sep = "/"
    qt_dir = f"{root}Qt{sep}{qt}{sep}{kit}"
    values = [
        ("QT_VERSION", qt),
        ("QT_ARCH", arch),
        ("QT_INSTALL_ROOT", root),
        ("PG_QT_DIR", qt_dir),
        ("AQTINSTALL_COMMIT", just_value("aqtinstall_commit")),
        ("EXIFTOOL_VERSION", just_value("exiftool_version")),
        # The ExifTool archive for this platform (tools/install_exiftool.py takes either).
        ("EXIFTOOL_URL", just_value("exiftool_windows_url" if sys.platform == "win32" else "exiftool_source_url")),
        ("EXIFTOOL_SHA256", just_value("exiftool_windows_sha256" if sys.platform == "win32" else "exiftool_source_sha256")),
        # The FFmpeg archive's SHA-256: the FFmpeg cache's key, so a new pin is a new cache.
        ("FFMPEG_SHA256", just_value("ffmpeg_sha256")),
        (
            "TAIKI_TOOLS",
            ",".join(
                f"{tool}@{just_value(name)}"
                for tool, name in (
                    ("just", "just_version"),
                    ("cargo-deny", "cargo_deny_version"),
                    ("cargo-machete", "cargo_machete_version"),
                    ("cargo-modules", "cargo_modules_version"),
                    ("uv", "uv_version"),
                )
            ),
        ),
    ]
    if sys.platform != "win32":
        values += [
            ("QMAKE", f"{qt_dir}/bin/qmake"),
            ("LD_LIBRARY_PATH", f"{qt_dir}/lib"),
        ]
    return values


def main() -> None:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    set_env = commands.add_parser("env", help="add NAME=VALUE to the later steps' environment")
    set_env.add_argument("name")
    set_env.add_argument("value")
    add_path = commands.add_parser("path", help="add DIR to the later steps' PATH")
    add_path.add_argument("directory")
    commands.add_parser("pins", help="add the Justfile's pins to the later steps' environment (the first step of a job)")
    args = parser.parse_args()
    if args.command == "env":
        append("GITHUB_ENV", env_line(args.name, args.value))
    elif args.command == "pins":
        append("GITHUB_ENV", "".join(env_line(name, value) for name, value in pin_values()))
    else:
        append("GITHUB_PATH", path_line(args.directory, Path(os.environ.get("GITHUB_WORKSPACE", Path.cwd()))))


if __name__ == "__main__":
    main()
