"""The decoder benches' callgrind totals (gungraun's target dir) as tables: M instructions per decode, and per pixel."""

import re
import sys
from pathlib import Path

FILES = [
    ("pin-base444-1mp", 1.069),
    ("pin-base444-3mp", 2.865),
    ("pin-prog420-1mp", 0.813),
    ("pin-base420-1mp", 1.302),
    ("phone-base420-4mp", 3.774),
    ("sony-base422-7mp", 7.500),
    ("a6000-base422-24mp", 24.0),
]
DECS = [
    ("turbo", "libjpeg-turbo"),
    ("turbo_fast", "turbo fast"),
    ("zune", "zune-jpeg"),
    ("jpeg_decoder", "jpeg-decoder"),
    ("ffmpeg", "FFmpeg"),
    ("vips", "libvips"),
]

root = Path(sys.argv[1])
ir = {}
threads = {}
for out in root.rglob("callgrind.*.out"):
    # One file per thread (.t1.p1, .t2.p1, ...) for decoders that start threads: summed.
    m = re.match(r"callgrind\.(full|scaled)_(.+)\.file_(\d+)(\.t\d+\.p\d+)?\.out$", out.name)
    if not m:
        continue
    key = (m.group(1), m.group(2), int(m.group(3)))
    threads[key] = threads.get(key, 0) + 1
    for line in out.open():
        if line.startswith(("summary:", "totals:")):
            ir[key] = ir.get(key, 0) + int(line.split()[1])
            break

for mode in ("full", "scaled"):
    print(f"\n{mode}: M instructions per decode (instructions per pixel of the full image)")
    print(f"{'':20}" + "".join(f"{label:>22}" for _, label in DECS))
    for i, (name, mp) in enumerate(FILES):
        row = f"{name:20}"
        for key, _ in DECS:
            v = ir.get((mode, key, i))
            row += f"{'—':>22}" if v is None else f"{v / 1e6:>13.1f} ({v / (mp * 1e6):>5.1f})"
        print(row)
    multi = sorted({k[1] for k, n in threads.items() if n > 1 and k[0] == mode})
    if multi:
        # gungraun collects only while the benchmark function is on a thread's stack, so the work of the threads
        # these decoders start (libvips's worker, jpeg-decoder's) is mostly missing: `wsl.py marginal` counts it.
        print(f"  (undercounted, they decode in their own threads: {', '.join(multi)}; see `wsl.py marginal`)")
