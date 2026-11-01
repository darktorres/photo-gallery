#!/bin/bash
# The harness's CPU decoders in WSL (docs/JPEG_DECODERS.md), after wsl-build.sh. From the repo, in PowerShell:
#   wsl -d Ubuntu-26.04 -u root -- bash eval/jpeg-decoders/harness-wsl.sh STEP...
# Steps:
#   compare     output against libjpeg-turbo (time mode with no budget: the times are not meant to be read)
#   ir          callgrind, every thread: (repeat 3 - repeat 1) / 2 = instructions for one decode, setup excluded
#   time        median wall ms and CPU ms per decode (needs an idle machine)
#   throughput  12 workers, one image each at a time: images/s and CPU ms per image (needs an idle machine)
# Inputs: ~/jpeg-restart (restart.sh): each file as it is (.orig), the progressive one converted losslessly to
# baseline (.base), and each with a restart marker every MCU row (.rstrow).
set -euo pipefail
H=$HOME/jpeg-eval/bin/harness
cd "$HOME/jpeg-restart"
NAMES=(pin-base444-1mp pin-base444-3mp pin-prog420-1mp pin-base420-1mp phone-base420-4mp sony-base422-7mp a6000-base422-24mp)
orig=(); rst=()
for n in "${NAMES[@]}"; do orig+=("$n.orig.jpg"); rst+=("$n.rstrow.jpg"); done
files=("${orig[@]}" pin-prog420-1mp.base.jpg "${rst[@]}")
CPU_DECODERS=turbo,turbo-yuv,parajpeg-t1,parajpeg-t6,parajpeg-t12,jpegli,wuffs

ir_one() {  # decoder file -> M instructions for one decode, all threads
    local d=$1 f=$2 t
    t=$(mktemp -d)
    for n in 1 3; do
        valgrind --tool=callgrind --callgrind-out-file="$t/$n" "$H" repeat $n "$d" "$f" >/dev/null 2>&1 || { echo ERR; return; }
    done
    awk -v a="$(awk '/^(summary|totals):/ { print $2; exit }' "$t/1")" \
        -v b="$(awk '/^(summary|totals):/ { print $2; exit }' "$t/3")" 'BEGIN { printf "%.1f", (b - a) / 2e6 }'
    rm -rf "$t"
}
export -f ir_one
export H

for step in "$@"; do
    case $step in
        compare) "$H" time --secs 0 --dec "${CPU_DECODERS/turbo-yuv,/}" "${files[@]}" ;;
        ir)
            decs=(turbo turbo-yuv parajpeg-t1 parajpeg-t12 jpegli wuffs)
            echo "M instructions per decode (callgrind, all threads); columns: ${decs[*]}"
            for f in "${files[@]}"; do
                printf '%-32s' "$f"
                # one job per decoder in parallel: instruction counts don't depend on load
                for d in "${decs[@]}"; do echo "$d"; done | xargs -P "${#decs[@]}" -I{} bash -c 'echo "{} $(ir_one {} '"$f"')"' \
                    | sort -k1,1 | awk -v order="${decs[*]}" 'BEGIN { n = split(order, o, " ") } { v[$1] = $2 }
                        END { for (i = 1; i <= n; i++) printf " %10s", v[o[i]]; print "" }'
            done
            ;;
        time) "$H" time --secs 2 --dec "$CPU_DECODERS" "${files[@]}" ;;
        throughput)
            for d in turbo parajpeg-t1 jpegli wuffs; do "$H" throughput --secs 3 --workers 12 "$d" "${orig[@]}"; done ;;
        *) echo "unknown step $step" >&2; exit 1 ;;
    esac
done
