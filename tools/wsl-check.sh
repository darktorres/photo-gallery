#!/bin/bash
# Clippy (-D warnings), then the tests of the whole workspace, in WSL. From the repo:
#   wsl -d Ubuntu-26.04 -u root -- bash tools/wsl-check.sh                 (or `just wsl-check`): the committed HEAD,
#       in WSL's own clone of this checkout (the sync is wsl-bench.sh's: the same clone, reset to HEAD).
#   wsl -d Ubuntu-26.04 -u root -- bash tools/wsl-check.sh --working-tree  (or `just wsl-check-tree`): the working tree,
#       uncommitted changes included (tracked and untracked files that aren't ignored), copied to ~/pg-wt (the copy of the
#       last run is replaced first). Both modes build into the clone's target/, so the build is kept between runs.
# The code that only builds on Linux (the callgrind benches, the platform's Unix paths, the Posix path rules, the desktop
# crate's Qt glue) is checked here: `just lint` on Windows never sees it.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/.." && pwd)
CLONE=${PG_BENCH_CLONE:-$HOME/photo-gallery}
WT=${PG_WT_DIR:-$HOME/pg-wt}

case "${1:-}" in
    "") mode=committed ;;
    --working-tree) mode=working-tree ;;
    *) echo "usage: wsl-check.sh [--working-tree]" >&2; exit 2 ;;
esac

if [ "$mode" = working-tree ]; then
    mkdir -p "$WT"
    find "$WT" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
    # A file the index lists but the tree no longer has is skipped (--ignore-failed-read), not an error.
    git -c safe.directory='*' -C "$REPO" ls-files -z --cached --others --exclude-standard \
        | tar --null --ignore-failed-read -cf - -C "$REPO" -T - \
        | tar -xf - -C "$WT"
    CHECKOUT=$WT
    echo "checking the working tree of $REPO, uncommitted changes included"
else
    [ -d "$CLONE/.git" ] || git clone -q "$REPO" "$CLONE"
    git -C "$CLONE" fetch -q "$REPO" HEAD
    git -C "$CLONE" checkout -q --force --detach FETCH_HEAD
    CHECKOUT=$CLONE
    echo "checking $(git -C "$CLONE" log --oneline -1)"
fi

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
# One target directory for both modes: the clone's (the working tree's build is kept between runs, as the clone's is).
export CARGO_TARGET_DIR=$CLONE/target

# The tests that skip without a tool (ExifTool, ffmpeg) or without the permissions to be stopped (root) say so on stderr:
# PG_REQUIRE_TOOLS makes a missing tool a failure, so the checks here are the real ones.
export PG_REQUIRE_TOOLS=1

cd "$CHECKOUT"
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
echo "wsl-check: clean ($mode)"
