"""Duplicate code: every window of W consecutive normalized code lines (W is the first argument, default 6), hashed across
the .rs files under each crate's src and tests folders. A window in an inline test module is marked (test). Prints the
groups of windows that occur in more than one place.

A report for a review (`just modgraph-report`), not a gate (see `testcut.py`).
"""

import hashlib
import re
import sys
from collections import defaultdict
from pathlib import Path

from testcut import production_part

ROOT = Path(__file__).resolve().parents[2]

# A window is compared only when its normalized text is at least CHARS_PER_LINE * MIN_FILL characters per line. The
# threshold skips windows of short lines (a run of `});`, `}` or bare calls), which repeat across unrelated code.
CHARS_PER_LINE = 40
MIN_FILL = 0.6  # a heuristic: neither number is derived from a measurement

TRIVIAL = re.compile(r"^[{}()\[\];,]*$|^(use |mod |#\[|pub mod |\}\)?;?$|Ok\(\(\)\)$)")


def norm(line: str) -> str:
    line = line.split("//", maxsplit=1)[0].strip()
    return re.sub(r"\s+", " ", line)


def main() -> None:
    w = int(sys.argv[1]) if len(sys.argv) > 1 else 6
    windows = defaultdict(list)
    for path in sorted(ROOT.glob("crates/*/src/**/*.rs")) + sorted(ROOT.glob("crates/*/tests/**/*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        raw = text.splitlines()
        test_start = len(production_part(text).splitlines())  # the first line of the inline test module, 0-based
        code = [(i + 1, norm(text_line)) for i, text_line in enumerate(raw)]
        code = [(n, text_line) for n, text_line in code if text_line and not TRIVIAL.match(text_line)]
        rel = path.relative_to(ROOT).as_posix()
        for k in range(len(code) - w + 1):
            block = "\n".join(text_line for _, text_line in code[k : k + w])
            if len(block) < CHARS_PER_LINE * w * MIN_FILL:
                continue
            h = hashlib.blake2b(block.encode(), digest_size=16).hexdigest()
            windows[h].append((rel, code[k][0], code[k][0] - 1 >= test_start, block))

    # Merge overlapping windows per pair of places into ranges.
    pairs = defaultdict(list)
    for places in windows.values():
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
        print("    at " + ", ".join(f"{f}:{start_line}" for f, start_line in starts))
        print("    " + sample.replace("\n", "\n    ")[:500])


if __name__ == "__main__":
    main()
