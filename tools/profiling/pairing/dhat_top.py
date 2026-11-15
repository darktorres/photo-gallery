"""The largest heap sites of a dhat profile at its global peak, charged to the first frame of ours.

Usage: python -I dhat_top.py <dhat-heap.json> [count]

Prints the total at the peak and over the run, then the `count` (default 14) sites with the most bytes live at the
peak, each with the first frames of its backtrace. A site is named by its innermost frame in a `pg_` crate (or
rusqlite/SQLite's), which is the code that decided to allocate (docs/reference/PROFILING.md, "Tool: dhat").
"""
import json
import sys

OURS = ("pg_", "rusqlite", "sqlite3", "libsqlite")


def main() -> None:
    profile = json.load(open(sys.argv[1], encoding="utf-8"))
    count = int(sys.argv[2]) if len(sys.argv) > 2 else 14
    frames = profile["ftbl"]

    def frame(i: int) -> str:
        return frames[i].split(": ", 1)[-1][:110]

    rows = sorted(profile["pps"], key=lambda pp: -pp["gb"])
    print(
        f"at the peak: {sum(pp['gb'] for pp in rows) / 1e6:.1f} MB; allocated over the run "
        f"{sum(pp['tb'] for pp in rows) / 1e6:.0f} MB in {sum(pp['tbk'] for pp in rows)} blocks"
    )
    for pp in rows[:count]:
        stack = [frame(i) for i in pp["fs"]]
        ours = [f for f in stack if any(o in f for o in OURS)]
        site = ours[0] if ours else (stack[0] if stack else "?")
        print(f"{pp['gb'] / 1e6:8.1f} MB at the peak | {pp['tb'] / 1e6:9.1f} MB in {pp['tbk']:7d} blocks | {site}")
        for f in stack[:6]:
            print("            ", f)


if __name__ == "__main__":
    main()
