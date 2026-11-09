#!/bin/bash
# The instruction-count gate (`just perf`): the callgrind benches of the committed HEAD (tools/profiling/bench.sh), compared
# with docs/perf/baseline.json (tools/perf-compare.py). Run in the container, by tools/linux.py perf:
#   bash tools/perf-check.sh [--tolerance 0.01] [--update]
# PG_BENCH_SKIP_CATALOG=1 (for a machine with no catalog data, such as CI): the catalog benches panic without it. So only the
# per-file benches run (a gungraun filter), and the comparer skips the catalog's baseline entries by name, printed as
# SKIPPED. The catalog is still gated where its data is (docs/PROFILING.md).
set -euo pipefail
REPO=$(cd "$(dirname "$0")/.." && pwd)
filter=()
skip=()
if [ "${PG_BENCH_SKIP_CATALOG:-}" = 1 ]; then
    filter=('*per_file*')
    skip=(--skip pg-bench/ir/catalog/)
fi
bash "$REPO/tools/profiling/bench.sh" "${filter[@]}"
# The benches are HEAD's (/head, built into /bench/target). The comparer and the baseline are the checkout's: the gate has
# to run before a commit too (the first baseline is made from the uncommitted tree's tools).
python3 "$REPO/tools/perf-compare.py" "$REPO/docs/perf/baseline.json" /bench/target/gungraun "${skip[@]}" "$@"
