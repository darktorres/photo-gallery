#!/bin/bash
# The JPEG decoder comparison under WSL (docs/JPEG_DECODERS.md). From the repo, in PowerShell:
#   wsl -d Ubuntu-26.04 -u root -- bash eval/jpeg-decoders/wsl.sh STEP...
# Steps: inputs (copy the 7 files to WSL's disk), check (clippy, rustfmt), compare (output vs libjpeg-turbo),
# time (median wall ms), bench (callgrind through gungraun, then the table), marginal (callgrind on whole runs,
# all threads: libvips and jpeg-decoder decode in their own threads, which gungraun doesn't count).
# Needs tools/profiling/wsl-setup.sh's toolchain and `apt-get install libvips-dev`.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/../.." && pwd)
CLONE=${PG_BENCH_CLONE:-$HOME/photo-gallery}
IN=${JPEG_INPUTS:-$HOME/jpeg-inputs}
DST=$CLONE/eval/jpeg-decoders  # its Cargo.toml takes turbojpeg-sys from the darktorres fork by rev (vendor/README.md)
NAMES=(pin-base444-1mp pin-base444-3mp pin-prog420-1mp pin-base420-1mp phone-base420-4mp sony-base422-7mp a6000-base422-24mp)

. "$HOME/.cargo/env"
export FFMPEG_DIR=$HOME/.local/share/pg-bench/ffmpeg
LIBCLANG_PATH=$(find /usr/lib -maxdepth 2 -path '/usr/lib/llvm-*/lib' | sort -V | tail -1)
export LIBCLANG_PATH JPEG_INPUTS=$IN
[ -d "$CLONE/.git" ] || git clone -q "$REPO" "$CLONE"
mkdir -p "$DST"
cp -r "$REPO/eval/jpeg-decoders/Cargo.toml" "$REPO/eval/jpeg-decoders/src" "$REPO/eval/jpeg-decoders/benches" "$DST/"
cd "$DST"
files=()
for n in "${NAMES[@]}"; do files+=("$IN/$n.jpg"); done

marginal() {  # callgrind on `repeat 3` minus `repeat 1`, halved: one decode, every thread
    local out bin=$DST/target/release/jpeg-decoders
    out=$(mktemp -d)
    ir() {
        valgrind --tool=callgrind --callgrind-out-file="$out/cg" "$bin" repeat "$@" >/dev/null 2>&1 || { echo ERR; return; }
        awk '/^(summary|totals):/ { print $2; exit }' "$out/cg"
    }
    cargo build -q --release
    echo "M instructions per decode, all threads; columns: ${NAMES[*]}"
    for dec in "libjpeg-turbo" "libvips" "jpeg-decoder"; do
        for mode in "full" "analyze scale"; do
            printf '%-20s %-14s' "$dec" "$mode"
            for f in "${files[@]}"; do
                a=$(ir 1 "$dec" "$mode" "$f")
                b=$(ir 3 "$dec" "$mode" "$f")
                if [ "$a" = ERR ] || [ "$b" = ERR ]; then printf ' %9s' "—"; else
                    awk -v a="$a" -v b="$b" 'BEGIN { printf " %9.1f", (b - a) / 2e6 }'; fi
            done
            echo
        done
    done
    rm -rf "$out"
}

for step in "$@"; do
    case $step in
        inputs)
            mkdir -p "$IN"
            # The source photos are not in the repo: $JPEG_SRC_DIR holds one <name>.jpg per NAMES entry.
            : "${JPEG_SRC_DIR:?set JPEG_SRC_DIR to a directory with <name>.jpg for each entry in NAMES}"
            for n in "${NAMES[@]}"; do cp -n "$JPEG_SRC_DIR/$n.jpg" "$IN/$n.jpg"; done
            ls -la "$IN"
            ;;
        check) cargo clippy -q --all-targets -- -D warnings && cargo fmt --check && echo "clippy, rustfmt: ok" ;;
        compare) cargo run -q --release -- compare "${files[@]}" ;;
        time) cargo run -q --release -- time --secs 1 "${files[@]}" ;;
        bench)
            cargo bench -q --bench ir
            python3 "$REPO/eval/jpeg-decoders/table.py" "$DST/target/gungraun"
            ;;
        marginal) marginal ;;
        *) echo "unknown step $step" >&2; exit 1 ;;
    esac
done
