#!/usr/bin/env python3
"""Fails with the fix when QT_DIR (the environment, or `.env`) is not the Qt 6.12.0 kit: the folder with `bin/qmake` (the
msvc2022_64 kit on Windows, gcc_64 on Linux). The Justfile's recipes that link Qt depend on it.
"""

import os
import sys
from pathlib import Path

from pgtools import WINDOWS, load_env_file


def main() -> None:
    load_env_file()
    qt = os.environ.get("QT_DIR", "")
    kit = "msvc2022_64" if WINDOWS else "gcc_64"
    if not qt:
        sys.exit(f"QT_DIR is not set: put the Qt kit folder in .env (QT_DIR=<path>/6.12.0/{kit})")
    qmake = Path(qt) / "bin" / ("qmake.exe" if WINDOWS else "qmake")
    if not qmake.is_file():
        sys.exit(f"no {qmake.name} in {qmake.parent}: QT_DIR must be the Qt 6.12.0 {kit} kit")


if __name__ == "__main__":
    main()
