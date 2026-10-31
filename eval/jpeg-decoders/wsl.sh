#!/bin/bash
# The JPEG decoder comparison under WSL (docs/PROFILING.md "JPEG decoders"). From the repo, in PowerShell:
#   wsl -d Ubuntu-26.04 -u root -- bash eval/jpeg-decoders/wsl.sh STEP...
# Steps: inputs (copy the 7 files to WSL's disk), check (clippy, rustfmt), compare (output vs libjpeg-turbo),
# time (median wall ms), bench (callgrind through gungraun, then the table), marginal (callgrind on whole runs,
# all threads: libvips and jpeg-decoder decode in their own threads, which gungraun doesn't count).
# Needs tools/profiling/wsl-setup.sh's toolchain and `apt-get install libvips-dev`.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/../.." && pwd)
CLONE=${PG_BENCH_CLONE:-$HOME/photo-gallery}
IN=${JPEG_INPUTS:-$HOME/jpeg-inputs}
DST=$CLONE/eval/jpeg-decoders  # beside vendor/turbojpeg-sys, which Cargo.toml patches in
NAMES=(pin-base444-1mp pin-base444-3mp pin-prog420-1mp pin-base420-1mp phone-base420-4mp sony-base422-7mp a6000-base422-24mp)

. "$HOME/.cargo/env"
export FFMPEG_DIR=$HOME/.local/share/pg-bench/ffmpeg
LIBCLANG_PATH=$(find /usr/lib -maxdepth 2 -path '/usr/lib/llvm-*/lib' | sort -V | tail -1)
export LIBCLANG_PATH JPEG_INPUTS=$IN
[ -d "$CLONE/vendor" ] || git clone -q "$REPO" "$CLONE"
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
            src=(
                "/mnt/d/PhotoGallery/datasets/pinterest/Photos/pinterest_1129770256504511017.jpg.mozjpeg.jpg"
                "/mnt/d/PhotoGallery/datasets/pinterest/Photos/barbara-palvin-scaled (4).webp.mozjpeg.jpg"
                "/mnt/d/PhotoGallery/datasets/pinterest/dups2/pinterest_1129770256522324934.jpg.mozjpeg.jpg"
                "/mnt/d/PhotoGallery/datasets/pinterest/dups2/amateur-alice-von-v-mbDfqe.jpg.mozjpeg.jpg"
                "/mnt/e/Cameras/Celular Rerrê/IMG_20160304_104540787.jpg"
                "/mnt/e/Cameras/101MSDCF/DSC00896.JPG"
                "/mnt/e/Cameras/Fotos A6000/_DSC5098.JPG"
            )
            for i in "${!NAMES[@]}"; do cp -n "${src[$i]}" "$IN/${NAMES[$i]}.jpg"; done
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
