#!/bin/bash
# Clippy (-D warnings) on every crate but the desktop, on the committed HEAD of this checkout, in WSL's own clone (the
# sync is wsl-bench.sh's: the same clone, reset to HEAD). The desktop needs Qt, which is Windows only. From the repo:
#   wsl -d Ubuntu-26.04 -u root -- bash tools/wsl-check.sh          (or `just wsl-check`)
# The code that only builds on Linux (the callgrind benches, the platform's Unix paths) is checked here: `just lint`
# on Windows never sees it.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/.." && pwd)
CLONE=${PG_BENCH_CLONE:-$HOME/photo-gallery}

[ -d "$CLONE/.git" ] || git clone -q "$REPO" "$CLONE"
git -C "$CLONE" fetch -q "$REPO" HEAD
git -C "$CLONE" checkout -q --force --detach FETCH_HEAD
echo "checking $(git -C "$CLONE" log --oneline -1)"

. "$HOME/.cargo/env"
# The same FFmpeg and libclang as the benches (wsl-setup.sh); the real environment wins over .cargo/config.toml's [env].
export FFMPEG_DIR=$HOME/.local/share/pg-bench/ffmpeg
[ -e "$FFMPEG_DIR/include/libavcodec/avcodec.h" ] || { echo "no FFmpeg headers in $FFMPEG_DIR: run wsl-setup.sh" >&2; exit 1; }
LIBCLANG_PATH=$(find /usr/lib -maxdepth 2 -path '/usr/lib/llvm-*/lib' | sort -V | tail -1)
export LIBCLANG_PATH

cd "$CLONE"
cargo clippy --workspace --exclude pg-ui --all-targets -- -D warnings
echo "wsl-check: clean"
