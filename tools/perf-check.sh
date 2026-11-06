#!/bin/bash
# The instruction-count gate (`just perf`): the callgrind benches on the committed HEAD in WSL
# (tools/profiling/wsl-bench.sh), compared with docs/perf/baseline.json (tools/perf-compare.py). Run from the repo:
#   wsl -d Ubuntu-26.04 -u root -- bash tools/perf-check.sh [--tolerance 0.01] [--update]
# PG_BENCH_SKIP_CATALOG=1 (for a machine with no catalog data, such as CI): the catalog benches panic without it. So only the
# per-file benches run (a gungraun filter), and the comparer skips the catalog's baseline entries by name, printed as
# SKIPPED. The catalog is still gated where its data is (WSL, docs/PROFILING.md).
set -euo pipefail
REPO=$(cd "$(dirname "$0")/.." && pwd)
CLONE=${PG_BENCH_CLONE:-$HOME/photo-gallery}
filter=()
skip=()
if [ "${PG_BENCH_SKIP_CATALOG:-}" = 1 ]; then
    filter=('*per_file*')
    skip=(--skip pg-bench/ir/catalog/)
fi
PG_BENCH_CLONE=$CLONE bash "$REPO/tools/profiling/wsl-bench.sh" "${filter[@]}"
# The benches are HEAD's (the clone). The comparer and the baseline are the checkout's: the clone has only what is
# committed, and the gate has to run before a commit too (the first baseline is made from the uncommitted tree's tools).
python3 "$REPO/tools/perf-compare.py" "$REPO/docs/perf/baseline.json" "$CLONE/target/gungraun" "${skip[@]}" "$@"
