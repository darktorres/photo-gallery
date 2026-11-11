"""Duplicate code: every function body (production and test) of >= N lines, and every window of W consecutive
normalized code lines, hashed across all .rs files under crates/. Prints groups that occur in more than one place.

A report for a review (`just modgraph-report`), not a gate: it matches by name, so it can both miss a use and invent one.
"""

import hashlib
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
W = int(sys.argv[1]) if len(sys.argv) > 1 else 6


def norm(line: str) -> str:
    line = line.split("//")[0].strip()
    return re.sub(r"\s+", " ", line)


TRIVIAL = re.compile(r"^[{}()\[\];,]*$|^(use |mod |#\[|pub mod |\}\)?;?$|Ok\(\(\)\)$)")
windows = defaultdict(list)
for path in sorted(ROOT.glob("crates/*/src/**/*.rs")) + sorted(ROOT.glob("crates/*/tests/**/*.rs")):
    raw = path.read_text(encoding="utf-8", errors="replace").splitlines()
    test_start = next((i for i, l in enumerate(raw) if l.startswith("#[cfg(test)]")), len(raw))
    code = [(i + 1, norm(l)) for i, l in enumerate(raw)]
    code = [(n, l) for n, l in code if l and not TRIVIAL.match(l)]
    rel = path.relative_to(ROOT).as_posix()
    for k in range(len(code) - W + 1):
        block = "\n".join(l for _, l in code[k:k + W])
        if len(block) < 40 * W * 0.6:
            continue
        h = hashlib.sha1(block.encode()).hexdigest()
        windows[h].append((rel, code[k][0], code[k][0] - 1 >= test_start, block))

# Merge overlapping windows per pair of places into ranges.
pairs = defaultdict(list)
for h, places in windows.items():
    files = {(p[0]) for p in places}
    if len(places) < 2:
        continue
    key = tuple(sorted({(p[0], p[2]) for p in places}))
    pairs[key].append(sorted(places, key=lambda p: (p[0], p[1])))

out = []
for key, groups in pairs.items():
    starts = sorted({(p[0], p[1]) for g in groups for p in g})
    out.append((len(groups), key, starts[:6], groups[0][0][3]))
for n, key, starts, sample in sorted(out, key=lambda x: -x[0]):
    kinds = ",".join(f"{f}{' (test)' if t else ''}" for f, t in key)
    print(f"--- {n} windows: {kinds}")
    print("    at " + ", ".join(f"{f}:{l}" for f, l in starts))
    print("    " + sample.replace("\n", "\n    ")[:500])
