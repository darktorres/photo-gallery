#!/bin/bash
# Wall-clock timing of a library scan (S-17.7 step 8). One library, three timed scans, each into the catalog of its run:
#   first      a fresh catalog, the library scanned from nothing;
#   unchanged  the same library scanned again, nothing changed;
#   renumbered every file renamed to another number in its own folder (its folder and content kept), then scanned again:
#              each file is paired with its row, not made new (docs/S17_7_PLAN.md, decision 6).
# Not gated: perf-compare.py reads callgrind outputs only. The numbers go into docs/PROFILING.md, with the commit, the
# binary, the drive the library is on and whether the caches were dropped.
#
#   bash tools/profiling/scan-timing.sh GALLERY LIBRARY WORK [--drop-caches]
#     GALLERY         a built gallery binary; time a release build (what users run)
#     LIBRARY         the library folder. If it does not exist, it is made with synthetic_library.py: 100 folders of 100
#                     files, 10,000 in all (about 15 GB; not timed)
#     WORK            a scratch folder: catalog/ is made fresh by every run, and each scan's output is kept beside it
#     --drop-caches   drop the page cache before each timed scan (as root): a cold read of the library's metadata
# The renumbering is what the scan pairs (the same folder, size, mtime and identity under a new name). The library is
# renumbered in place, so a second run of this script on the same LIBRARY renumbers it back, and both timings stay valid.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/../.." && pwd)

usage() { echo "usage: $0 GALLERY LIBRARY WORK [--drop-caches]" >&2; exit 2; }
[ $# -ge 3 ] || usage
GALLERY=$1 LIBRARY=$2 WORK=$3
drop=0
[ "${4:-}" = "--drop-caches" ] && drop=1
[ $# -le 4 ] || usage
[ -x "$GALLERY" ] || { echo "scan-timing.sh: $GALLERY is not an executable" >&2; exit 1; }

if [ ! -d "$LIBRARY" ]; then
    python3 "$REPO/tools/profiling/synthetic_library.py" "$LIBRARY" --folders 100 --per-folder 100
fi
files=$(find "$LIBRARY" -type f | wc -l)
echo "library $LIBRARY: $files files"
[ "$files" -gt 0 ] || { echo "scan-timing.sh: no files under $LIBRARY" >&2; exit 1; }

mkdir -p "$WORK"
CATALOG="$WORK/catalog"
rm -rf "$CATALOG"

if head_sha=$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null); then
    echo "gallery $GALLERY, built from the tree at $head_sha with $(git -C "$REPO" status --porcelain | wc -l) changed paths"
else
    echo "gallery $GALLERY (not a git checkout: the commit it was built from is for the caller to record)"
fi
echo "caches: $([ $drop = 1 ] && echo dropped before each scan || echo as they are)"

# Runs a scan with its output in $WORK/<label>.log, and prints its wall-clock seconds.
timed() {
    local label=$1; shift
    [ $drop = 1 ] && { sync; echo 3 > /proc/sys/vm/drop_caches; }
    local start end
    start=$(date +%s%N)
    if ! "$@" > "$WORK/$label.log" 2>&1; then
        echo "scan-timing.sh: $label failed, see $WORK/$label.log" >&2
        exit 1
    fi
    end=$(date +%s%N)
    awk -v label="$label" -v a="$start" -v b="$end" 'BEGIN { printf "%-11s %9.3f s\n", label, (b - a) / 1e9 }'
}

timed first "$GALLERY" --catalog-dir "$CATALOG" scan "$LIBRARY"
timed unchanged "$GALLERY" --catalog-dir "$CATALOG" scan

# Every file's number moves to the other range: 0..999 to 1000..1999 and back. Two phases, through temporary names, so
# no rename lands on a name still in use. Not timed.
python3 - "$LIBRARY" <<'EOF'
import os
import re
import sys

root = sys.argv[1]
pattern = re.compile(r"IMG_(\d{2})_(\d+)\.(\w+)$")
moves = []
for folder in sorted(os.listdir(root)):
    directory = os.path.join(root, folder)
    if not os.path.isdir(directory):
        continue
    for index, name in enumerate(sorted(os.listdir(directory))):
        match = pattern.match(name)
        if not match:
            continue
        number = int(match[2])
        new = number + 1000 if number < 1000 else number - 1000
        target = os.path.join(directory, f"IMG_{match[1]}_{new:03d}.{match[3]}")
        moves.append((os.path.join(directory, name), os.path.join(directory, f".renumber-{index}"), target))
for source, temporary, _ in moves:
    os.rename(source, temporary)
for _, temporary, target in moves:
    if os.path.exists(target):
        sys.exit(f"renumber: {target} is in use")
    os.rename(temporary, target)
print(f"renumbered {len(moves)} files")
EOF

timed renumbered "$GALLERY" --catalog-dir "$CATALOG" scan
echo "logs: $WORK/{first,unchanged,renumbered}.log"
