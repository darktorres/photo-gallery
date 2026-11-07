#!/bin/bash
# Restart markers and progressive -> baseline, both lossless (jpegtran), on the 7 decoder inputs
# (docs/JPEG_DECODERS.md "Restart markers and lossless baseline conversion"): size, pixel identity (djpeg, full and 1/2),
# and libjpeg-turbo's cost (the vendored 3.1, through the eval harness): callgrind instructions per decode
# (3 decodes minus 1, halved) and wall ms per decode (best of 3 runs of N decodes).
# In WSL, after `wsl.sh inputs time` (which copy the inputs and build the harness):
#   bash eval/jpeg-decoders/restart.sh    (needs apt-get install libjpeg-turbo-progs, for jpegtran and djpeg)
set -uo pipefail
IN=${JPEG_INPUTS:-$HOME/jpeg-inputs}
OUT=${JPEG_RESTART_OUT:-$HOME/jpeg-restart}  # the variants, kept for GPU tests
BIN=${PG_BENCH_CLONE:-$HOME/photo-gallery}/eval/jpeg-decoders/target/release/jpeg-decoders  # wsl.sh builds it
mkdir -p "$OUT"
NAMES=(pin-base444-1mp pin-base444-3mp pin-prog420-1mp pin-base420-1mp phone-base420-4mp sony-base422-7mp a6000-base422-24mp)
VARIANTS=(orig base rst10b rstrow)

make_variant() {  # NAME VARIANT -> path
    local src=$IN/$1.jpg dst=$OUT/$1.$2.jpg
    case $2 in
        orig) cp "$src" "$dst" ;;
        base) jpegtran -copy all -optimize "$src" >"$dst" ;;              # baseline, optimized tables, no markers
        rst10b) jpegtran -copy all -optimize -restart 10B "$src" >"$dst" ;; # a marker every 10 MCUs (Fastvideo's 4:4:4)
        rstrow) jpegtran -copy all -optimize -restart 1 "$src" >"$dst" ;;   # a marker every MCU row
    esac
    echo "$dst"
}

ir() {  # MODE FILE -> M instructions per decode, all threads
    local t a b
    t=$(mktemp -d)
    valgrind --tool=callgrind --callgrind-out-file="$t/a" "$BIN" repeat 1 libjpeg-turbo "$1" "$2" >/dev/null 2>&1
    valgrind --tool=callgrind --callgrind-out-file="$t/b" "$BIN" repeat 3 libjpeg-turbo "$1" "$2" >/dev/null 2>&1
    a=$(awk '/^(summary|totals):/ {print $2; exit}' "$t/a"); b=$(awk '/^(summary|totals):/ {print $2; exit}' "$t/b")
    rm -rf "$t"
    awk -v a="$a" -v b="$b" 'BEGIN { printf "%.1f", (b - a) / 2e6 }'
}

wall() {  # MODE FILE N -> ms per decode, best of 3 runs of N decodes (less sensitive to outside load)
    local best=999999 s e ms
    for _ in 1 2 3; do
        s=$(date +%s%N); "$BIN" repeat "$3" libjpeg-turbo "$1" "$2" >/dev/null; e=$(date +%s%N)
        ms=$(awk -v s="$s" -v e="$e" -v n="$3" 'BEGIN { printf "%.3f", (e - s) / 1e6 / n }')
        best=$(awk -v a="$best" -v b="$ms" 'BEGIN { print (b < a) ? b : a }')
    done
    printf "%.2f" "$best"
}

printf '%-20s %-7s %9s %8s %-9s %-9s %8s %8s %8s %8s\n' input variant bytes size% px-full px-1/2 Mir-full Mir-scal ms-full ms-scal
for n in "${NAMES[@]}"; do
    orig_size=$(stat -c %s "$IN/$n.jpg")
    djpeg -ppm "$IN/$n.jpg" >"$OUT/ref-full.ppm"; djpeg -ppm -scale 1/2 "$IN/$n.jpg" >"$OUT/ref-half.ppm"
    reps=$(( 60000000 / orig_size + 3 ))  # roughly constant work per wall measurement
    for v in "${VARIANTS[@]}"; do
        f=$(make_variant "$n" "$v")
        size=$(stat -c %s "$f")
        pf=$(djpeg -ppm "$f" | cmp -s - "$OUT/ref-full.ppm" && echo same || echo DIFF)
        ph=$(djpeg -ppm -scale 1/2 "$f" | cmp -s - "$OUT/ref-half.ppm" && echo same || echo DIFF)
        printf '%-20s %-7s %9d %+7.1f%% %-9s %-9s %8s %8s %8s %8s\n' "$n" "$v" "$size" \
            "$(awk -v a="$orig_size" -v b="$size" 'BEGIN { print (b - a) * 100 / a }')" "$pf" "$ph" \
            "$(ir full "$f")" "$(ir 'analyze scale' "$f")" "$(wall full "$f" "$reps")" "$(wall 'analyze scale' "$f" "$reps")"
    done
done
rm -f "$OUT"/ref-*.ppm
