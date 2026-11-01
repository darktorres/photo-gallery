#!/bin/bash
# The callgrind benches (crates/core/benches/ir.rs) on the committed HEAD of this checkout, built in a clone on WSL's
# own disk: faster than /mnt/c, and the Windows tree's uncommitted work stays out. From the repo, in PowerShell:
#   wsl -d Ubuntu-26.04 -u root -- bash tools/profiling/wsl-bench.sh [gungraun args, e.g. --save-baseline=before]
# The clone (PG_BENCH_CLONE, default ~/photo-gallery) belongs to this script: it's reset to HEAD on every run.
# Results go to $CLONE/target/gungraun/; read them with tools/profiling/cgtab.py (docs/PROFILING.md).
# A bench name as the argument runs only the matching benches (gungraun's FILTER), e.g. `catalog::list`.
# The catalog benches' data (tools/profiling/bench_data.py, on D:) is copied to $PG_BENCH_DATA, default
# ~/pg-bench, once per layout.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/../.." && pwd)
CLONE=${PG_BENCH_CLONE:-$HOME/photo-gallery}
export PG_BENCH_DATA=${PG_BENCH_DATA:-$HOME/pg-bench}
BENCH_DATA_SOURCE=${PG_BENCH_DATA_SOURCE:-/mnt/d/PhotoGallery/bench-data}

mkdir -p "$PG_BENCH_DATA"
for layout in "$BENCH_DATA_SOURCE"/*/; do
    [ -d "$layout" ] || continue
    name=$(basename "$layout")
    [ -d "$PG_BENCH_DATA/$name" ] || cp -r "$layout" "$PG_BENCH_DATA/$name"
done

[ -d "$CLONE/.git" ] || git clone -q "$REPO" "$CLONE"
git -C "$CLONE" fetch -q "$REPO" HEAD
git -C "$CLONE" checkout -q --force --detach FETCH_HEAD
echo "bench at $(git -C "$CLONE" log --oneline -1)"

. "$HOME/.cargo/env"
# .cargo/config.toml's [env] is for Windows (FFmpeg's DLLs on D:, LLVM in C:\Program Files). The real environment
# wins over it: the system FFmpeg through wsl-setup.sh's prefix, and the newest installed libclang.
export FFMPEG_DIR=$HOME/.local/share/pg-bench/ffmpeg
[ -e "$FFMPEG_DIR/include/libavcodec/avcodec.h" ] || { echo "no FFmpeg headers in $FFMPEG_DIR: run wsl-setup.sh" >&2; exit 1; }
LIBCLANG_PATH=$(find /usr/lib -maxdepth 2 -path '/usr/lib/llvm-*/lib' | sort -V | tail -1)
export LIBCLANG_PATH

cd "$CLONE"
# The Linux benches compile only here: `just lint` on Windows never sees them.
cargo clippy -q -p pg-core --benches -- -D warnings
cargo bench -q -p pg-core --bench ir -- "$@"
