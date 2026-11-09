#!/usr/bin/env python3
"""The callgrind benches (crates/bench/benches/ir.rs) of the committed HEAD, in the container's tools image (`just perf`,
which runs tools/perf_check.py and so this). Usage, inside the container:
    python3 tools/profiling/bench.py [gungraun args, e.g. --save-baseline=before]
The tree benched is /head: the launcher's git archive of HEAD (tools/linux.py), so the working tree's uncommitted work stays
out. The Cargo target and the bench data are volumes of their own (/bench), and the results are in /bench/target/gungraun.
A full run's .out files are archived (zstd) to docs/perf/callgrind/<sha>.tar.zst in the checkout, one archive per benched
commit; unpack with `tar --zstd -xf`, then read them with tools/profiling/cgtab.py (docs/PROFILING.md).
A bench name as an argument runs only the matching benches (gungraun's FILTER), e.g. `catalog::list`. Such a run is partial,
so it isn't archived: an archive would replace the commit's full one with a subset. Options (starting with -, e.g.
--save-baseline=before) don't make a run partial, but an option's value must follow `=`: a bare option before a plain
argument is refused, since that argument could be its value or a bench name.
The catalog benches' data (tools/profiling/bench_data.py, made on the Windows side) is mounted at /bench-src and copied to
/bench/data once per layout.
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
TREE = Path("/head")
DATA = Path("/bench/data")
TARGET = Path("/bench/target")


def refuse_ambiguous(args: list[str]) -> bool:
    """Checks the arguments before anything runs, so a refused one costs no build. Returns whether a bench name filters
    the run."""
    filtered = False
    bare = ""
    for arg in args:
        if bare and not arg.startswith("-"):
            sys.exit(
                f"bench.py: '{bare} {arg}' is ambiguous: write a value as {bare}=VALUE, or put the bench name before the options"
            )
        bare = ""
        if arg.startswith("-"):
            if "=" not in arg:
                bare = arg
        else:
            filtered = True
    return filtered


def copy_bench_data() -> None:
    """Copies each layout of the bench data from /bench-src, under a temporary name renamed when complete (an interrupted copy
    must not look finished)."""
    source = Path("/bench-src")
    if not source.is_dir():
        return
    DATA.mkdir(parents=True, exist_ok=True)
    for layout in sorted(p for p in source.iterdir() if p.is_dir()):
        dest = DATA / layout.name
        if not dest.is_dir():
            partial = DATA / (layout.name + ".partial")
            shutil.rmtree(partial, ignore_errors=True)
            shutil.copytree(layout, partial)
            partial.rename(dest)


def tool_line(command: list[str]) -> str:
    """The first line of a tool's output, or an empty text when it doesn't run."""
    result = subprocess.run(command, capture_output=True, text=True, check=False)
    lines = (result.stdout + result.stderr).splitlines()
    return lines[0] if lines else ""


def main() -> None:
    filtered = refuse_ambiguous(sys.argv[1:])
    sha = os.environ.get("PG_HEAD_SHA")
    if not sha:
        sys.exit("bench.py: PG_HEAD_SHA is unset: run it through tools/linux.py perf, which passes the commit")
    os.environ["PG_BENCH_DATA"] = str(DATA)
    os.environ["CARGO_TARGET_DIR"] = str(TARGET)
    copy_bench_data()

    print(f"bench at {sha}")
    # The tools the counts depend on, printed with every run: the callgrind baseline changes with them (docker/apt-packages.txt
    # is not pinned to a snapshot; see docker/Dockerfile).
    pkg = subprocess.run(["pkg-config", "--modversion", "libavcodec"], capture_output=True, text=True, check=False).stdout.strip()
    print(
        f"valgrind {tool_line(['valgrind', '--version'])}, {tool_line(['ldd', '--version'])}, libavcodec {pkg}, "
        f"{tool_line(['rustc', '--version'])}"
    )
    # Cleared first: gungraun writes into target/gungraun and nothing else removes older results, so a directory from an
    # earlier layout (ir/…, pg-core/…) would be read as a bench of this run by perf-compare.py.
    gungraun = TARGET / "gungraun"
    shutil.rmtree(gungraun, ignore_errors=True)
    extra = sys.argv[1:]
    for command in (
        ["cargo", "clippy", "-p", "pg-bench", "--benches", "--", "-D", "warnings"],
        ["cargo", "bench", "-p", "pg-bench", "--bench", "ir", "--", *extra],
    ):
        print("+ " + " ".join(command), flush=True)
        result = subprocess.run(command, cwd=TREE, env=os.environ, check=False)
        if result.returncode != 0:
            sys.exit(result.returncode)

    if filtered:
        print("profiles: not archived (filtered run)")
        return
    # Only the .out files: .out.old is the run before, and the .log files are gungraun's text. The archive holds them with the
    # target's layout (pg-bench/ir/...).
    outputs = sorted(p.relative_to(gungraun).as_posix() for p in gungraun.rglob("callgrind.*.out"))
    if not outputs:
        sys.exit("no callgrind outputs to archive")
    archive = REPO / "docs" / "perf" / "callgrind" / f"{sha}.tar.zst"
    archive.parent.mkdir(parents=True, exist_ok=True)
    listing = gungraun / "archive-list.txt"
    listing.write_text("\n".join(outputs) + "\n", encoding="utf-8")
    try:
        subprocess.run(
            ["tar", "--zstd", "-cf", str(archive), "-C", str(gungraun), "-T", str(listing)],
            env={**os.environ, "ZSTD_CLEVEL": "19"},
            check=True,
        )
    finally:
        listing.unlink(missing_ok=True)
    count = subprocess.run(["tar", "--zstd", "-tf", str(archive)], capture_output=True, text=True, check=True)
    size = archive.stat().st_size / (1024 * 1024)
    print(f"profiles: {len(count.stdout.splitlines())} files in {archive.relative_to(REPO)} ({size:.1f} MiB)")


if __name__ == "__main__":
    main()
