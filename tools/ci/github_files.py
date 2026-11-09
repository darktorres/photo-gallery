#!/usr/bin/env python3
"""Writes to the files GitHub Actions reads between steps, on every OS: `env NAME VALUE` appends to GITHUB_ENV (the later
steps' environment) and `path DIR` appends to GITHUB_PATH (their PATH; a relative DIR is taken from the workspace). A step
calls it from any shell, so CI's steps need no PowerShell for it.

    python tools/ci/github_files.py env PG_DESKTOP_CONTENT_KEY <hash>
    python tools/ci/github_files.py path .deps/ffmpeg/bin
"""

import argparse
import os
import re
import sys
from pathlib import Path

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


def main() -> None:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    set_env = commands.add_parser("env", help="add NAME=VALUE to the later steps' environment")
    set_env.add_argument("name")
    set_env.add_argument("value")
    add_path = commands.add_parser("path", help="add DIR to the later steps' PATH")
    add_path.add_argument("directory")
    args = parser.parse_args()
    if args.command == "env":
        append("GITHUB_ENV", env_line(args.name, args.value))
    else:
        append("GITHUB_PATH", path_line(args.directory, Path(os.environ.get("GITHUB_WORKSPACE", Path.cwd()))))


if __name__ == "__main__":
    main()
