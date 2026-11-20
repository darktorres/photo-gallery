#!/usr/bin/env python3
"""Formats the tracked QML files in place with the Qt kit's qmlformat (`just fmt`, every OS). The files are the repository's
own (git ls-files, vendor/ left out), so an untracked scratch file is not touched.

The kit is QT_DIR (from the environment or .env), and qmlformat is read from its bin folder, never from PATH, as
tools/lint_qml.py reads its tools: another Qt on PATH cannot format against a different version.
"""

import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def qmlformat_path(qt: Path) -> str:
    exe = "qmlformat" + (".exe" if os.name == "nt" else "")
    tool = qt / "bin" / exe
    if not tool.is_file():
        sys.exit(f"{exe} is not in {qt / 'bin'}: QT_DIR must be the Qt kit folder (the one with bin and lib)")
    return str(tool)


def tracked_qml() -> list[str]:
    listed = subprocess.run(["git", "ls-files", "-z", "--", "*.qml"], cwd=REPO, capture_output=True, check=True).stdout.decode(
        "utf-8"
    )
    return [name for name in listed.split("\0") if name and not name.startswith("vendor/")]


def main() -> int:
    # `just` loads .env into the environment (dotenv-load), so QT_DIR is set here.
    qt_dir = os.environ.get("QT_DIR")
    if not qt_dir:
        sys.exit("QT_DIR is not set: point it at the Qt kit folder (the one with bin and lib), in the environment or in .env.")
    files = tracked_qml()
    if not files:
        print("format_qml: no tracked QML files")
        return 0
    # One qmlformat call per batch keeps the command line short; --inplace writes each file back.
    tool = qmlformat_path(Path(qt_dir))
    for start in range(0, len(files), 50):
        batch = files[start : start + 50]
        subprocess.run([tool, "--inplace", *batch], cwd=REPO, check=True)
    print(f"format_qml: {len(files)} QML files formatted")
    return 0


if __name__ == "__main__":
    sys.exit(main())
