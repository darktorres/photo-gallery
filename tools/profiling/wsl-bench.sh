#!/bin/bash
# The callgrind benches (crates/bench/benches/ir.rs) on the committed HEAD of this checkout, built in a clone on WSL's
# own disk: faster than /mnt/c, and the Windows tree's uncommitted work stays out. From the repo, in PowerShell:
#   wsl -d Ubuntu-26.04 -u root -- bash tools/profiling/wsl-bench.sh [gungraun args, e.g. --save-baseline=before]
# The clone (PG_BENCH_CLONE, default ~/photo-gallery) belongs to this script: it's reset to HEAD on every run.
# Results go to $CLONE/target/gungraun/, and a full run's .out files are archived (zstd) to docs/perf/callgrind/<sha>.tar.zst
# in the checkout: committed, one archive per benched commit. Unpack with `tar --zstd -xf`, then read them with
# tools/profiling/cgtab.py (docs/PROFILING.md).
# A bench name as the argument runs only the matching benches (gungraun's FILTER), e.g. `catalog::list`. Such a run is
# partial, so it isn't archived: an archive would replace the commit's full one with a subset. Options (starting with -,
# e.g. --save-baseline=before) don't make a run partial, but an option's value must follow `=`: a bare option before a
# plain argument is refused, since that argument could be its value or a bench name.
# The catalog benches' data (tools/profiling/bench_data.py, made on the Windows side) is copied to $PG_BENCH_DATA, default
# ~/pg-bench, once per layout.
set -euo pipefail

# Checked before anything runs, so a refused argument costs no build.
filtered=0
bare=
for arg in "$@"; do
    if [ -n "$bare" ] && [[ $arg != -* ]]; then
        echo "wsl-bench.sh: '$bare $arg' is ambiguous: write a value as $bare=VALUE, or put the bench name before the options" >&2
        exit 2
    fi
    bare=
    case "$arg" in
        -*=*) ;;
        -*) bare=$arg ;;
        *) filtered=1 ;;
    esac
done

REPO=$(cd "$(dirname "$0")/../.." && pwd)
CLONE=${PG_BENCH_CLONE:-$HOME/photo-gallery}
export PG_BENCH_DATA=${PG_BENCH_DATA:-$HOME/pg-bench}
# The bench data folder as a WSL path. The Justfile's WSLENV passes PG_BENCH_DATA_SOURCE (a Windows path in .env) here
# already converted, so it is required: there is no default.
BENCH_DATA_SOURCE=${PG_BENCH_DATA_SOURCE:-}
[ -n "$BENCH_DATA_SOURCE" ] || { echo "wsl-bench.sh: PG_BENCH_DATA_SOURCE is unset: the bench data folder (a WSL path) is needed; run through just, which reads it from .env" >&2; exit 1; }

mkdir -p "$PG_BENCH_DATA"
for layout in "$BENCH_DATA_SOURCE"/*/; do
    [ -d "$layout" ] || continue
    name=$(basename "$layout")
    dest="$PG_BENCH_DATA/$name"
    # Copied under a temporary name and renamed when complete: an interrupted copy must not look finished.
    if [ ! -d "$dest" ]; then
        rm -rf "$dest.partial"
        cp -r "$layout" "$dest.partial" && mv "$dest.partial" "$dest"
    fi
done

[ -d "$CLONE/.git" ] || git clone "$REPO" "$CLONE"
git -C "$CLONE" fetch "$REPO" HEAD
git -C "$CLONE" checkout --force --detach FETCH_HEAD
echo "bench at $(git -C "$CLONE" log --oneline -1)"

. "$HOME/.cargo/env"
# .cargo/config.toml's [env] is for Windows (FFmpeg's folder and LLVM's, both Windows paths). The real environment
# wins over it: the system FFmpeg through wsl-setup.sh's prefix, and the newest installed libclang.
export FFMPEG_DIR=$HOME/.local/share/pg-bench/ffmpeg
[ -e "$FFMPEG_DIR/include/libavcodec/avcodec.h" ] || { echo "no FFmpeg headers in $FFMPEG_DIR: run wsl-setup.sh" >&2; exit 1; }
LIBCLANG_PATH=$(find /usr/lib -maxdepth 2 -path '/usr/lib/llvm-*/lib' | sort -V | tail -1)
export LIBCLANG_PATH

cd "$CLONE"
# Cleared first: gungraun writes into target/gungraun and nothing else removes older results, so a directory from an
# earlier layout (ir/…, pg-core/…) would be read as a bench of this run by perf-compare.py.
rm -rf "$CLONE/target/gungraun"
# The Linux benches compile only here: `just lint` on Windows never sees them.
cargo clippy -p pg-bench --benches -- -D warnings
cargo bench -p pg-bench --bench ir -- "$@"

# A bench-name argument makes this run partial (see the header, and the check at the top).
if [ $filtered -eq 0 ]; then
    # A full run's callgrind outputs are kept in the checkout, as a reference over time: one archive per benched commit,
    # docs/perf/callgrind/<short sha of HEAD>.tar.zst, holding the .out files with the target's layout (pg-bench/ir/...).
    # Only the .out files: .out.old is the run before, and the .log files are gungraun's text.
    sha=$(git -C "$CLONE" rev-parse --short HEAD)
    archive="$REPO/docs/perf/callgrind/$sha.tar.zst"
    mkdir -p "$REPO/docs/perf/callgrind"
    list=$(mktemp)
    (cd "$CLONE/target/gungraun" && find . -name 'callgrind.*.out' | sed 's#^\./##' | sort) > "$list"
    [ -s "$list" ] || { echo "no callgrind outputs to archive" >&2; exit 1; }
    ZSTD_CLEVEL=19 tar --zstd -cf "$archive" -C "$CLONE/target/gungraun" -T "$list"
    rm -f "$list"
    echo "profiles: $(wc -l < <(tar --zstd -tf "$archive")) files in ${archive#"$REPO/"} ($(du -h "$archive" | cut -f1))"
else
    echo "profiles: not archived (filtered run)"
fi
