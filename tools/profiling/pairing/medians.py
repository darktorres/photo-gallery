"""Per workload and build, the medians of a keep.ps1 CSV's timed parts: what the keep rule's ratios are of.

Usage: python -I medians.py <runs.csv> [parts]

`parts` is a comma list of the CSV's columns (default `pass,load,join,write,commit`; also `cycles`, `sort`, `scan`, ...).
The values are as the CSV holds them (ms, or M cycles); a decimal comma (a PowerShell culture's) is read as a point.
"""

import csv
import statistics
import sys
from pathlib import Path


def main() -> None:
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    with Path(sys.argv[1]).open(newline="", encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    parts = sys.argv[2].split(",") if len(sys.argv) > 2 else ["pass", "load", "join", "write", "commit"]
    for workload in dict.fromkeys(r["workload"] for r in rows):
        for build in "AB":
            runs = [r for r in rows if r["workload"] == workload and r["build"] == build]
            if not runs:
                continue
            medians = {p: statistics.median(float(r[p].replace(",", ".")) for r in runs) for p in parts}
            print(workload, build, " ".join(f"{p} {medians[p]:.0f}" for p in parts))


if __name__ == "__main__":
    main()
