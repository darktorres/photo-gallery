#!/usr/bin/env bash
# Counts rustc's `unreachable_pub` warnings for each crate's library target (docs/MODULARITY_PLAN.md, M-18), and any
# compile errors (a crate that does not build here is reported, with its first error lines). Run it on the target you
# want counted: Windows on the host (Git Bash, with QT_DIR and its bin folder on PATH), Linux in the pg-dev container.
#
#   bash tools/modgraph/unreachable_pub.sh
#
# Crates with no library target (pg-bench, pg-gallery) are not listed: the script takes only the library crates.

set -u
cd "$(dirname "$0")/../.." || exit 1
# python3 on Linux, python on Windows (where python3 may be a store stub).
PY=python3
"$PY" -c 'pass' >/dev/null 2>&1 || PY=python

crates=$(cargo metadata --no-deps --format-version 1 | "$PY" -c '
import json, sys
for p in json.load(sys.stdin)["packages"]:
    if any(t["kind"] == ["lib"] for t in p["targets"]):
        print(p["name"])
' | tr -d '\r')

for crate in $crates; do
    out=$(cargo rustc -p "$crate" --lib --profile check -- -W unreachable_pub 2>&1)
    count=$(printf '%s\n' "$out" | grep -c 'unreachable `pub`')
    errors=$(printf '%s\n' "$out" | grep -c '^error')
    echo "$crate unreachable_pub=$count errors=$errors"
    if [ "$errors" -gt 0 ]; then
        printf '%s\n' "$out" | grep -m 3 -A2 '^error' | sed 's/^/    /'
    fi
done
