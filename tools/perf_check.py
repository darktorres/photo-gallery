#!/usr/bin/env python3
"""The instruction-count gate (`just perf`): the callgrind benches of the committed HEAD (tools/profiling/bench.py), compared
with docs/perf/baseline.json (tools/perf-compare.py). Run in the container, by tools/linux.py perf:
    python3 tools/perf_check.py [--tolerance 0.01] [--update]
PG_BENCH_SKIP_CATALOG=1 (for a machine with no catalog data, such as CI): the catalog benches panic without it. So only the
per-file benches run (a gungraun filter), and the comparer skips the catalog's baseline entries by name, printed as SKIPPED.
The catalog is still gated where its data is (docs/PROFILING.md).
"""

import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def main() -> None:
    bench_filter: list[str] = []
    skip: list[str] = []
    if os.environ.get("PG_BENCH_SKIP_CATALOG") == "1":
        bench_filter = ["*per_file*"]
        skip = ["--skip", "pg-bench/ir/catalog/"]
    bench = [sys.executable, str(REPO / "tools" / "profiling" / "bench.py"), *bench_filter]
    result = subprocess.run(bench, cwd=REPO, check=False)
    if result.returncode != 0:
        sys.exit(result.returncode)
    # The benches are HEAD's (/head, built into /bench/target). The comparer and the baseline are the checkout's: the gate has
    # to run before a commit too (the first baseline is made from the uncommitted tree's tools).
    compare = [
        sys.executable,
        str(REPO / "tools" / "perf-compare.py"),
        str(REPO / "docs" / "perf" / "baseline.json"),
        "/bench/target/gungraun",
        *skip,
        *sys.argv[1:],
    ]
    sys.exit(subprocess.run(compare, cwd=REPO, check=False).returncode)


if __name__ == "__main__":
    main()
