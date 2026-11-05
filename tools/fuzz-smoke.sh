#!/bin/bash
# Each fuzz target (fuzz/, S-18.1) for a short time, on a clone of the committed HEAD in WSL's own disk (like
# tools/profiling/wsl-bench.sh). From the repo, in PowerShell:
#   wsl -d Ubuntu-26.04 -u root -- bash tools/fuzz-smoke.sh [seconds per target, default 60]
# The fuzzer needs Linux and nightly Rust (cargo-fuzz's sanitizers). The C libraries the crates link come from
# tools/profiling/wsl-setup.sh, run once before this. A crash exits non-zero and leaves its input in fuzz/artifacts/.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/.." && pwd)
CLONE=${PG_FUZZ_CLONE:-$HOME/photo-gallery-fuzz}
SECONDS_EACH=${1:-60}

[ -d "$CLONE/.git" ] || git clone "$REPO" "$CLONE"
git -C "$CLONE" fetch "$REPO" HEAD
git -C "$CLONE" checkout --force --detach FETCH_HEAD
echo "fuzz at $(git -C "$CLONE" log --oneline -1)"

. "$HOME/.cargo/env"
command -v cargo-fuzz >/dev/null || cargo install --locked cargo-fuzz
rustup toolchain list | grep -q '^nightly' || rustup toolchain install nightly --profile minimal
export FFMPEG_DIR=$HOME/.local/share/pg-bench/ffmpeg
LIBCLANG_PATH=$(find /usr/lib -maxdepth 2 -path '/usr/lib/llvm-*/lib' | sort -V | tail -1)
export LIBCLANG_PATH

cd "$CLONE/fuzz"
# Captured first: in a `for ... in $(cargo fuzz list)` a failing list runs no target and the script still exits 0.
targets=$(cargo fuzz list)
[ -n "$targets" ] || { echo "cargo fuzz list found no fuzz targets in $CLONE/fuzz" >&2; exit 1; }
for target in $targets; do
    echo "== $target: ${SECONDS_EACH}s"
    cargo +nightly fuzz run "$target" -- -max_total_time="$SECONDS_EACH"
done
