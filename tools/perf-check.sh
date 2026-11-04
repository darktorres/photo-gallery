#!/bin/bash
# The instruction-count gate (`just perf`, the nightly job): the callgrind benches on the committed HEAD in WSL
# (tools/profiling/wsl-bench.sh), compared with docs/perf/baseline.json (tools/perf-compare.py). Run from the repo:
#   wsl -d Ubuntu-26.04 -u root -- bash tools/perf-check.sh [--tolerance 0.01] [--update]
set -euo pipefail
REPO=$(cd "$(dirname "$0")/.." && pwd)
CLONE=${PG_BENCH_CLONE:-$HOME/photo-gallery}
PG_BENCH_CLONE=$CLONE bash "$REPO/tools/profiling/wsl-bench.sh"
# The benches are HEAD's (the clone). The comparer and the baseline are the checkout's: the clone has only what is
# committed, and the gate has to run before a commit too (the first baseline is made from the uncommitted tree's tools).
python3 "$REPO/tools/perf-compare.py" "$REPO/docs/perf/baseline.json" "$CLONE/target/gungraun" "$@"
