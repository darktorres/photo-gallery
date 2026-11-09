#!/bin/bash
# The callgrind benches (crates/bench/benches/ir.rs) of the committed HEAD, in the container's tools image (`just perf`,
# which runs tools/perf-check.sh and so this). Usage, inside the container:
#   bash tools/profiling/bench.sh [gungraun args, e.g. --save-baseline=before]
# The tree benched is /head: the launcher's git archive of HEAD (tools/linux.py), so the working tree's uncommitted work
# stays out. The Cargo target and the bench data are volumes of their own (/bench), and the results are in
# /bench/target/gungraun. A full run's .out files are archived (zstd) to docs/perf/callgrind/<sha>.tar.zst in the checkout,
# one archive per benched commit; unpack with `tar --zstd -xf`, then read them with tools/profiling/cgtab.py (docs/PROFILING.md).
# A bench name as an argument runs only the matching benches (gungraun's FILTER), e.g. `catalog::list`. Such a run is
# partial, so it isn't archived: an archive would replace the commit's full one with a subset. Options (starting with -,
# e.g. --save-baseline=before) don't make a run partial, but an option's value must follow `=`: a bare option before a
# plain argument is refused, since that argument could be its value or a bench name.
# The catalog benches' data (tools/profiling/bench_data.py, made on the Windows side) is mounted at /bench-src and copied
# to /bench/data once per layout.
set -euo pipefail

# Checked before anything runs, so a refused argument costs no build.
filtered=0
bare=
for arg in "$@"; do
    if [ -n "$bare" ] && [[ $arg != -* ]]; then
        echo "bench.sh: '$bare $arg' is ambiguous: write a value as $bare=VALUE, or put the bench name before the options" >&2
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
TREE=/head
SHA=${PG_HEAD_SHA:?bench.sh: PG_HEAD_SHA is unset: run it through tools/linux.py perf, which passes the commit}
export PG_BENCH_DATA=/bench/data
export CARGO_TARGET_DIR=/bench/target

if [ -d /bench-src ]; then
    mkdir -p "$PG_BENCH_DATA"
    for layout in /bench-src/*/; do
        [ -d "$layout" ] || continue
        name=$(basename "$layout")
        dest="$PG_BENCH_DATA/$name"
        # Copied under a temporary name and renamed when complete: an interrupted copy must not look finished.
        if [ ! -d "$dest" ]; then
            rm -rf "$dest.partial"
            cp -r "$layout" "$dest.partial" && mv "$dest.partial" "$dest"
        fi
    done
fi

cd "$TREE"
echo "bench at $SHA"
# The tools the counts depend on, printed with every run: the callgrind baseline changes with them (docker/apt-packages.txt is
# not pinned to a snapshot; see docker/Dockerfile).
echo "valgrind $(valgrind --version), $(ldd --version | head -1), libavcodec $(pkg-config --modversion libavcodec), $(rustc --version)"
# Cleared first: gungraun writes into target/gungraun and nothing else removes older results, so a directory from an
# earlier layout (ir/…, pg-core/…) would be read as a bench of this run by perf-compare.py.
rm -rf "$CARGO_TARGET_DIR/gungraun"
cargo clippy -p pg-bench --benches -- -D warnings
cargo bench -p pg-bench --bench ir -- "$@"

if [ $filtered -eq 0 ]; then
    # Only the .out files: .out.old is the run before, and the .log files are gungraun's text. The archive holds them with
    # the target's layout (pg-bench/ir/...).
    archive="$REPO/docs/perf/callgrind/$SHA.tar.zst"
    mkdir -p "$REPO/docs/perf/callgrind"
    list=$(mktemp)
    (cd "$CARGO_TARGET_DIR/gungraun" && find . -name 'callgrind.*.out' | sed 's#^\./##' | sort) > "$list"
    [ -s "$list" ] || { echo "no callgrind outputs to archive" >&2; exit 1; }
    ZSTD_CLEVEL=19 tar --zstd -cf "$archive" -C "$CARGO_TARGET_DIR/gungraun" -T "$list"
    rm -f "$list"
    echo "profiles: $(wc -l < <(tar --zstd -tf "$archive")) files in ${archive#"$REPO/"} ($(du -h "$archive" | cut -f1))"
else
    echo "profiles: not archived (filtered run)"
fi
