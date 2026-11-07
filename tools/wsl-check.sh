#!/bin/bash
# Clippy (-D warnings), then the tests of the whole workspace, on the committed HEAD of this checkout, in WSL's own clone
# (the sync is wsl-bench.sh's: the same clone, reset to HEAD). From the repo:
#   wsl -d Ubuntu-26.04 -u root -- bash tools/wsl-check.sh          (or `just wsl-check`)
# The code that only builds on Linux (the callgrind benches, the platform's Unix paths, the Posix path rules) is checked
# here: `just lint` on Windows never sees it. Uncommitted changes are not in this clone; commit first to check them.
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

# Qt 6.12 for Linux, as CI's Linux job has it (install-qt-action: aqtinstall at the commit ci.yml pins, linux_gcc_64,
# qtmultimedia, into ~/Qt/6.12.0/gcc_64). The desktop crate links it, so the whole workspace is checked here.
QT=$HOME/Qt/6.12.0/gcc_64
[ -x "$QT/bin/qmake" ] || { echo "no Qt 6.12 for Linux in $QT: install it as CI does (see ci.yml's linux job)" >&2; exit 1; }
export QMAKE=$QT/bin/qmake
export LD_LIBRARY_PATH=$QT/lib
export QT_QPA_PLATFORM=offscreen

cd "$CLONE"
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
echo "wsl-check: clean"
