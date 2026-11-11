#!/usr/bin/env python3
"""The instruction-count gate (`just perf`): the callgrind benches of the committed HEAD (tools/profiling/bench.py), compared
with docs/perf/baseline.json (tools/perf-compare.py). Run in the container, by tools/linux.py perf:
    python3 tools/perf_check.py [--tolerance 0.01] [--update] [--budgets [FILE]]
PG_BENCH_SKIP_CATALOG=1 (for a machine with no catalog data, such as CI): the catalog benches panic without it. So only the
per-file benches run (a gungraun filter), and the comparer skips the catalog's baseline entries by name, printed as SKIPPED.
The catalog is still gated where its data is (docs/PROFILING.md).

--budgets (`just perf --budgets`) gates one comparison whose change moves some counts on purpose: the benches named in
docs/perf/budgets.json (or FILE) are judged by their budget instead of the tolerance; the others keep the tolerance. It
is for that one run (the notifications' F1 against the F0 baseline: `scan-walk` <= +10%, `import-flush` <= +15%,
`inbox-refresh` down by 90% or more). A bench that misses its budget fails, and is not re-baselined with --update (the
two are refused together).
"""

import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BUDGETS = REPO / "docs" / "perf" / "budgets.json"


def compare_arguments(argv: list[str]) -> list[str]:
    """The options for perf-compare.py: a bare --budgets names the committed budgets file, so the comparer finds it
    whatever its own default resolves to in the container."""
    out = []
    for i, arg in enumerate(argv):
        bare = arg == "--budgets" and (i + 1 == len(argv) or argv[i + 1].startswith("-"))
        out.append(f"--budgets={BUDGETS}" if bare else arg)
    return out


def main() -> None:
    bench_filter: list[str] = []
    skip: list[str] = []
    if os.environ.get("PG_BENCH_SKIP_CATALOG") == "1":
        bench_filter = ["*per_file*"]
        skip = ["--skip", "pg-bench/ir/catalog/"]
    options = compare_arguments(sys.argv[1:])
    # The budgets file is checked before the benches run, which take minutes.
    for i, arg in enumerate(options):
        budgets = arg.split("=", 1)[1] if arg.startswith("--budgets=") else options[i + 1] if arg == "--budgets" else ""
        if budgets and not Path(budgets).is_file():
            sys.exit(f"no budgets file at {budgets}")
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
        *options,
    ]
    sys.exit(subprocess.run(compare, cwd=REPO, check=False).returncode)


if __name__ == "__main__":
    main()
