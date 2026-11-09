#!/usr/bin/env python3
"""Wall-clock timing of a library scan (S-17.7 step 8). One library, three timed scans, each into the catalog of its run:
  first      a fresh catalog, the library scanned from nothing;
  unchanged  the same library scanned again, nothing changed;
  renumbered every file renamed to another number in its own folder (its folder and content kept), then scanned again:
             each file is paired with its row, not made new (docs/S17_7_PLAN.md, decision 6).
Not gated: perf-compare.py reads callgrind outputs only. The numbers go into docs/PROFILING.md, with the commit, the binary,
the drive the library is on and whether the caches were dropped.

    python3 tools/profiling/scan_timing.py GALLERY LIBRARY WORK [--drop-caches]
      GALLERY         a built gallery binary; time a release build (what users run)
      LIBRARY         the library folder. If it does not exist, it is made with synthetic_library.py: 100 folders of 100
                      files, 10,000 in all (about 15 GB; not timed)
      WORK            a scratch folder: catalog/ is made fresh by every run, and each scan's output is kept beside it
      --drop-caches   drop the page cache before each timed scan (as root): a cold read of the library's metadata
The renumbering is what the scan pairs (the same folder, size, mtime and identity under a new name). The library is
renumbered in place, so a second run of this script on the same LIBRARY renumbers it back, and both timings stay valid.
"""

import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
PATTERN = re.compile(r"IMG_(\d{2})_(\d+)\.(\w+)$")


def usage() -> None:
    sys.exit("usage: scan_timing.py GALLERY LIBRARY WORK [--drop-caches]")


def drop_caches() -> None:
    """Drops the page cache (root only): the flush, then the drop."""
    os.sync()
    try:
        Path("/proc/sys/vm/drop_caches").write_text("3", encoding="utf-8")
    except OSError as error:
        sys.exit(f"scan_timing.py: dropping the caches needs root: {error}")


def timed(label: str, command: list[str], work: Path, drop: bool) -> None:
    """Runs a scan with its output in WORK/<label>.log, and prints its wall-clock seconds."""
    if drop:
        drop_caches()
    start = time.perf_counter()
    with (work / f"{label}.log").open("w", encoding="utf-8") as log:
        result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=False)
    elapsed = time.perf_counter() - start
    if result.returncode != 0:
        sys.exit(f"scan_timing.py: {label} failed, see {work / (label + '.log')}")
    print(f"{label:<11} {elapsed:9.3f} s")


def renumber(library: Path) -> int:
    """Moves each file's number to the other range (0..999 to 1000..1999 and back), in two phases through temporary names, so
    no rename lands on a name still in use. Returns the number of files moved."""
    moves: list[tuple[Path, Path, Path]] = []
    for folder in sorted(p for p in library.iterdir() if p.is_dir()):
        for index, name in enumerate(sorted(p.name for p in folder.iterdir())):
            match = PATTERN.match(name)
            if not match:
                continue
            number = int(match[2])
            new = number + 1000 if number < 1000 else number - 1000
            target = folder / f"IMG_{match[1]}_{new:03d}.{match[3]}"
            moves.append((folder / name, folder / f".renumber-{index}", target))
    for source, temporary, _ in moves:
        source.rename(temporary)
    for _, temporary, target in moves:
        if target.exists():
            sys.exit(f"renumber: {target} is in use")
        temporary.rename(target)
    return len(moves)


def main() -> None:
    args = sys.argv[1:]
    drop = "--drop-caches" in args
    args = [a for a in args if a != "--drop-caches"]
    if len(args) != 3:
        usage()
    gallery, library, work = Path(args[0]), Path(args[1]), Path(args[2])
    if not os.access(gallery, os.X_OK):
        sys.exit(f"scan_timing.py: {gallery} is not an executable")
    if not library.is_dir():
        maker = [sys.executable, str(REPO / "tools" / "profiling" / "synthetic_library.py"), str(library)]
        subprocess.run([*maker, "--folders", "100", "--per-folder", "100"], check=True)
    files = sum(1 for p in library.rglob("*") if p.is_file())
    print(f"library {library}: {files} files")
    if files == 0:
        sys.exit(f"scan_timing.py: no files under {library}")

    work.mkdir(parents=True, exist_ok=True)
    catalog = work / "catalog"
    shutil.rmtree(catalog, ignore_errors=True)
    head = subprocess.run(["git", "-C", str(REPO), "rev-parse", "--short", "HEAD"], capture_output=True, text=True, check=False)
    if head.returncode == 0:
        status = subprocess.run(["git", "-C", str(REPO), "status", "--porcelain"], capture_output=True, text=True, check=False)
        changed = len(status.stdout.splitlines())
        print(f"gallery {gallery}, built from the tree at {head.stdout.strip()} with {changed} changed paths")
    else:
        print(f"gallery {gallery} (not a git checkout: the commit it was built from is for the caller to record)")
    print(f"caches: {'dropped before each scan' if drop else 'as they are'}")

    scan = [str(gallery), "--catalog-dir", str(catalog), "scan"]
    timed("first", [*scan, str(library)], work, drop)
    timed("unchanged", scan, work, drop)
    # Every file's number moves to the other range. Not timed.
    print(f"renumbered {renumber(library)} files")
    timed("renumbered", scan, work, drop)
    print(f"logs: {work}/{{first,unchanged,renumbered}}.log")


if __name__ == "__main__":
    main()
