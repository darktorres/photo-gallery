#!/bin/bash
# Each fuzz target (fuzz/, S-18.1) for a short time, in the container's tools image (`just fuzz-smoke`, which runs this
# through tools/linux.py fuzz). The tree fuzzed is /head: the launcher's git archive of HEAD. The fuzzer's build is a volume
# of its own. A crash stops the run, and its input is copied to fuzz/artifacts/ in the checkout.
#   bash tools/fuzz-smoke.sh [seconds per target, default 60]
set -euo pipefail
REPO=$(cd "$(dirname "$0")/.." && pwd)
TREE=/head
SECONDS_EACH=${1:-60}
export CARGO_TARGET_DIR=/bench/fuzz-target

cd "$TREE/fuzz"
echo "fuzz at ${PG_HEAD_SHA:-HEAD}"
# fuzz/ is its own workspace with its own Cargo.lock, and cargo-fuzz does not pass --locked: fail here if the lock is stale.
cargo metadata --locked --format-version 1 >/dev/null
# Captured first: in a `for ... in $(cargo fuzz list)` a failing list runs no target and the script still exits 0.
targets=$(cargo fuzz list)
[ -n "$targets" ] || { echo "cargo fuzz list found no fuzz targets in $TREE/fuzz" >&2; exit 1; }
for target in $targets; do
    echo "== $target: ${SECONDS_EACH}s"
    if ! cargo fuzz run "$target" -- -max_total_time="$SECONDS_EACH"; then
        mkdir -p "$REPO/fuzz/artifacts"
        cp -r "$TREE/fuzz/artifacts/." "$REPO/fuzz/artifacts/"
        echo "crash: the input is in fuzz/artifacts/ in the checkout" >&2
        exit 1
    fi
done
echo "fuzz-smoke: every target ran for ${SECONDS_EACH}s with no crash"
