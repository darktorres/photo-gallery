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

# The permission tests skip as root, and this check runs as root: the permissions don't stop it. They run here as an
# ordinary user too: a copy of each test binary runs as that user, and a skip there fails the check.
if [ "$(id -u)" -eq 0 ]; then
    ORDINARY=pgcheck
    id "$ORDINARY" >/dev/null 2>&1 || useradd -m "$ORDINARY"
    RUN=$(mktemp -d /tmp/pg-ordinary.XXXXXX)
    trap 'rm -rf "$RUN"; userdel -r "$ORDINARY" 2>/dev/null || true' EXIT
    chmod 755 "$RUN"
    mkdir "$RUN/scratch"
    chown "$ORDINARY" "$RUN/scratch"
    check_as_ordinary() {  # $1: the test binary (a `--test` target of pg-library), $2: the test's name
        local exe out
        exe=$(cargo test -p pg-library --test "$1" --no-run 2>&1 | grep Executable | sed 's/.*(\(.*\))$/\1/')
        cp "$exe" "$RUN/$1"
        chmod 755 "$RUN/$1"
        out=$(su "$ORDINARY" -s /bin/bash -c "cd '$RUN' && PG_TEST_DIR='$RUN/scratch' PG_REQUIRE_TOOLS=1 ./$1 --exact $2 --nocapture 2>&1") \
            || { echo "$out"; echo "$2 failed as $ORDINARY" >&2; exit 1; }
        if echo "$out" | grep -q "skipped:"; then
            echo "$out"
            echo "$2 skipped as $ORDINARY: the permissions it checks were not stopping the process" >&2
            exit 1
        fi
        echo "as $ORDINARY: $2 ok"
    }
    check_as_ordinary borders an_unreadable_file_stays_due_for_the_next_pass
    check_as_ordinary writeback a_finishing_copy_that_keeps_failing_is_given_up
fi
echo "wsl-check: clean ($mode)"
