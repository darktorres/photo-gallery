#!/usr/bin/env python3
"""Fails with the fix when PG_QT_DIR (the environment, or `.env`) is not the Qt kit the Justfile pins (`qt_version`): the
folder with `bin/qmake`, of that version (the msvc2022_64 kit on Windows, gcc_64 on Linux). The Justfile's recipes that link
Qt depend on it.
"""

import os
import subprocess
import sys
from pathlib import Path

from pgtools import QT_DIR_HOW, QT_KIT, WINDOWS, just_value, load_env_file, not_set


def main() -> None:
    load_env_file()
    wanted = just_value("qt_version")
    qt = os.environ.get("PG_QT_DIR", "")
    kit = QT_KIT
    if not qt:
        not_set("PG_QT_DIR", f"{QT_DIR_HOW}; the Justfile pins Qt {wanted} {kit} (PG_QT_DIR=<path>/{wanted}/{kit})")
    qmake = Path(qt) / "bin" / ("qmake.exe" if WINDOWS else "qmake")
    if not qmake.is_file():
        sys.exit(f"no {qmake.name} in {qmake.parent}: PG_QT_DIR must be the Qt {wanted} {kit} kit")
    # The kit's own version, not its folder name: a kit renamed to the pinned folder is still the wrong kit.
    query = subprocess.run([str(qmake), "-query", "QT_VERSION"], capture_output=True, text=True, check=False)
    found = query.stdout.strip()
    if query.returncode != 0 or found != wanted:
        sys.exit(
            f"{qmake} reports Qt {found or 'nothing'}, the Justfile pins Qt {wanted}: PG_QT_DIR must be the {wanted} {kit} kit"
        )


if __name__ == "__main__":
    main()
