#!/usr/bin/env python3
"""Instruction profile of the gallery, callgrind's format, one file per thread (Windows only: Intel Pin). It was profile.ps1.
It runs target/release/gallery.exe under Pin with callgraph.dll (build.py) until a startup stage (crates/trace/src/lib.rs),
then names the functions Pin could only name by address (symbolize.py). Read the result with tools/profiling/cgtab.py or
KCachegrind.

    python tools/profiling/callgraph/profile.py --out %PG_ROOT%\\profiles\\ready [--stop pg_stage_ready] [--start STAGE] [--qt DIR] [-- gallery arguments]

The app runs with this shell's environment: set PG_CATALOG_DIR and PG_PREFS for a scratch catalog first. Under Pin the start
takes some 50 times longer; the window shows meanwhile. --stop '' runs until the app exits. Reads QT_DIR (the Qt kit folder;
--qt overrides it), PIN_EXE (Pin's pin.exe; default under the SDE kit's pinkit folder in LOCALAPPDATA) and the repo's .env for
keys the environment does not set.
"""

import argparse
import os
import re
import subprocess
import sys
import time
from pathlib import Path

from pgtools import REPO, WINDOWS, load_env_file

HERE = Path(__file__).resolve().parent


def main() -> None:
    if not WINDOWS:
        sys.exit("profile.py: Windows only (Intel Pin)")
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    parser.add_argument("--out", required=True, type=Path, help="the profile's path; the runs' files are named from it")
    parser.add_argument("--stop", default="pg_stage_ready", help="the stage to stop at (empty: until the app exits)")
    parser.add_argument("--start", default="", help="the stage to start counting at")
    parser.add_argument("--exe", type=Path, default=REPO / "target" / "release" / "gallery.exe")
    parser.add_argument("--qt", help="the Qt kit folder (default QT_DIR)")
    parser.add_argument("app_args", nargs="*", help="arguments for the app, after --")
    args = parser.parse_args()

    load_env_file()
    qt = args.qt or os.environ.get("QT_DIR", "")
    if not qt:
        sys.exit("QT_DIR is not set: point it at the Qt kit folder (the one with bin and lib), in the environment or in .env (or pass --qt).")
    pin = Path(os.environ["PIN_EXE"]) if os.environ.get("PIN_EXE") else (
        Path(os.environ["LOCALAPPDATA"]) / "Programs" / "sde" / "pinkit" / "pin.exe"
    )
    tool = REPO / "target" / "pin" / "callgraph.dll"
    if not tool.is_file():
        sys.exit(f"no {tool}: run tools/profiling/callgraph/build.py first")
    args.out.parent.mkdir(parents=True, exist_ok=True)

    knobs = ["-o", str(args.out)]
    if args.stop:
        knobs += ["-stop", args.stop]
    if args.start:
        knobs += ["-start", args.start]
    started = time.perf_counter()
    result = subprocess.run([str(pin), "-t", str(tool), *knobs, "--", str(args.exe.resolve()), *args.app_args], check=False)
    print(f"pin: exit {result.returncode} after {time.perf_counter() - started:,.0f} s")

    # The newest run's files: the runs' names are the profile's name, a dash and a number (one per thread).
    folder = args.out.parent
    files = sorted(folder.glob(f"{args.out.name}.*-*"), key=lambda path: path.stat().st_mtime)
    groups: dict[str, list[Path]] = {}
    for path in files:
        groups.setdefault(re.sub(r"-\d+$", "", path.name), []).append(path)
    # The group whose first file is the latest (Group-Object's order, then the last of its groups).
    newest = list(groups.values())[-1] if groups else []
    subprocess.run([sys.executable, str(HERE / "symbolize.py"), "--path", f"{qt}/bin", *[str(p) for p in newest]], check=False)


if __name__ == "__main__":
    main()
