"""JPEG encoders against each other for docs/JPEG_ENCODERS.md: bytes at equal SSIMULACRA2, and encode speed.

In WSL (Ubuntu-26.04), with wsl-build.sh's tools in ~/jpeg-eval/bin:
    python3 eval/jpeg-decoders/encoders.py prepare      # the inputs decoded once (libjpeg-turbo) to PPM
    python3 eval/jpeg-decoders/encoders.py encode       # CPU encoders, every quality, in parallel
    (Windows: nvimgcodec_encode.py and gpujpeg via encoders-win.cmd write into the same out/ directory)
    python3 eval/jpeg-decoders/encoders.py speed        # one encode at a time, Q75: ms per megapixel
    python3 eval/jpeg-decoders/encoders.py score        # SSIMULACRA2 of every output, then the tables

Every output is decoded by the same libjpeg-turbo (djpeg) and scored against the PPM it was encoded from, so
decoder differences don't enter. The reference setting is ImageReduce's: libjpeg-turbo-compatible baseline, Q75,
4:4:4, optimized Huffman tables (it runs mozjpeg in JCP_FASTEST mode, which is plain libjpeg-turbo). For each
encoder, the bytes it needs to reach the reference's score are interpolated on its quality sweep (log bytes, linear
in score) and reported as a ratio to the reference's bytes.
"""

import csv
import math
import os
import statistics
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

BIN = Path.home() / "jpeg-eval/bin"
W = Path.home() / "jpeg-enc"
INPUTS = Path.home() / "jpeg-inputs"
NAMES = ["pin-base444-1mp", "pin-base444-3mp", "pin-prog420-1mp", "pin-base420-1mp", "phone-base420-4mp",
         "sony-base422-7mp", "a6000-base422-24mp"]
QUALITIES = [50, 60, 70, 75, 80, 85, 90, 95]
GUETZLI_Q = [84, 88, 92, 95]  # guetzli refuses quality below 84
GUETZLI_MAX_MP = 3.5  # minutes per megapixel and ~300 MB per megapixel: Pinterest-sized inputs only
REFERENCE = "turbo"
REF_Q = 75


def run(cmd, **kw):
    return subprocess.run([str(c) for c in cmd], check=True, capture_output=True, **kw)


def png_of(ppm):  # guetzli reads PNG only
    png = ppm.with_suffix(".png")
    if not png.exists():
        from PIL import Image  # python3-pil

        Image.open(ppm).save(png, compress_level=1)
    return png


# name -> (command for quality q, qualities); {i} input PPM, {o} output JPEG. All 4:4:4, as ImageReduce encodes.
ENCODERS = {
    "turbo": (lambda q: [BIN / "cjpeg-turbo", "-quality", q, "-sample", "1x1", "-optimize", "-outfile", "{o}", "{i}"],
              QUALITIES),
    "turbo-prog": (lambda q: [BIN / "cjpeg-turbo", "-quality", q, "-sample", "1x1", "-progressive", "-outfile", "{o}",
                              "{i}"], QUALITIES),
    "mozjpeg": (lambda q: [BIN / "cjpeg-mozjpeg", "-quality", q, "-sample", "1x1", "-outfile", "{o}", "{i}"],
                QUALITIES),
    "mozjpeg-base": (lambda q: [BIN / "cjpeg-mozjpeg", "-quality", q, "-sample", "1x1", "-baseline", "-outfile", "{o}",
                                "{i}"], QUALITIES),
    "jpegli": (lambda q: [BIN / "cjpegli", "{i}", "{o}", "-q", q, "--chroma_subsampling=444"], QUALITIES),
    "jpegli-base": (lambda q: [BIN / "cjpegli", "{i}", "{o}", "-q", q, "--chroma_subsampling=444", "-p", "0"],
                    QUALITIES),
    "guetzli": (lambda q: [BIN / "guetzli", "--quality", q, "{png}", "{o}"], GUETZLI_Q),
}


def ppm_size(ppm):
    with open(ppm, "rb") as f:
        parts = f.read(64).split()
    return int(parts[1]), int(parts[2])


def jobs():
    for name in NAMES:
        ppm = W / "src" / f"{name}.ppm"
        w, h = ppm_size(ppm)
        for enc, (cmd, qs) in ENCODERS.items():
            if enc == "guetzli" and w * h / 1e6 > GUETZLI_MAX_MP:
                continue
            for q in qs:
                yield name, ppm, enc, q, cmd(q)


def expand(cmd, ppm, out):
    sub = {"{i}": ppm, "{o}": out, "{png}": png_of(ppm) if "{png}" in cmd else None}
    return [sub.get(c, c) if isinstance(c, str) else c for c in cmd]


def prepare():
    (W / "src").mkdir(parents=True, exist_ok=True)
    (W / "out").mkdir(exist_ok=True)
    for name in NAMES:
        ppm = W / "src" / f"{name}.ppm"
        if not ppm.exists():
            run([BIN / "djpeg-turbo", "-ppm", "-outfile", ppm, INPUTS / f"{name}.jpg"])
        print(name, *ppm_size(ppm))


def encode():
    def one(job):
        name, ppm, enc, q, cmd = job
        out = W / "out" / f"{name}.{enc}.q{q}.jpg"
        if not out.exists():
            run(expand(cmd, ppm, out.with_suffix(".tmp")))
            out.with_suffix(".tmp").rename(out)
        return out

    todo = sorted(jobs(), key=lambda j: j[2] == "guetzli", reverse=True)  # guetzli first: the longest
    with ThreadPoolExecutor(os.cpu_count()) as ex:
        for out in ex.map(one, todo):
            print(out.name, out.stat().st_size, flush=True)


def speed():
    print("Q75, one encode at a time, median of 3 runs of the command (PPM in, JPEG out): ms per megapixel")
    rows = []
    for name in NAMES:
        ppm = W / "src" / f"{name}.ppm"
        mp = math.prod(ppm_size(ppm)) / 1e6
        line = [name]
        for enc, (cmd, qs) in ENCODERS.items():
            q = REF_Q if REF_Q in qs else qs[0]
            if enc == "guetzli" and mp > GUETZLI_MAX_MP:
                line.append("—")
                continue
            out = W / "speed.jpg"
            reps = 1 if enc == "guetzli" else 3
            ts = []
            for _ in range(reps):
                t = time.perf_counter()
                run(expand(cmd(q), ppm, out))
                ts.append(time.perf_counter() - t)
            line.append(f"{statistics.median(ts) * 1e3 / mp:.1f}")
        rows.append(line)
        print(*line, sep="\t", flush=True)
    lines = ["\t".join(["image", *ENCODERS])] + ["\t".join(r) for r in rows]
    (W / "speed.tsv").write_text("\n".join(lines) + "\n")


def score_one(out):
    name = out.name.split(".")[0]
    dec = out.with_suffix(".dec.ppm")
    run([BIN / "djpeg-turbo", "-ppm", "-outfile", dec, out])
    r = run([BIN / "ssimulacra2", W / "src" / f"{name}.ppm", dec], text=True)
    dec.unlink()
    return float(r.stdout.split()[-1])


def score():
    cache = W / "scores.csv"
    known = {}
    if cache.exists():
        with open(cache) as f:
            known = {r["file"]: float(r["ssimulacra2"]) for r in csv.DictReader(f)}
    outs = sorted(p for p in (W / "out").glob("*.jpg"))
    todo = [p for p in outs if p.name not in known]
    with ThreadPoolExecutor(os.cpu_count()) as ex:
        for p, s in zip(todo, ex.map(score_one, todo)):
            known[p.name] = s
    with open(cache, "w", newline="") as f:
        wr = csv.writer(f)
        wr.writerow(["file", "ssimulacra2"])
        for k in sorted(known):
            wr.writerow([k, f"{known[k]:.3f}"])
    # name -> enc -> [(q, bytes, score)]
    data = {}
    for p in outs:
        name, enc, q = p.name.split(".")[:3]
        data.setdefault(name, {}).setdefault(enc, []).append((int(q[1:]), p.stat().st_size, known[p.name]))
    encs = sorted({e for d in data.values() for e in d}, key=lambda e: (e != REFERENCE, e))
    print(f"Bytes to reach the score of {REFERENCE} Q{REF_Q} (4:4:4, optimized Huffman), as a ratio of its bytes; "
          "'<' = even the lowest quality tried scores higher, '>' = the highest scores lower")
    print("image".ljust(22), "ref score".rjust(9), "ref bpp".rjust(8), *[e.rjust(13) for e in encs[1:]])
    ratios = {e: [] for e in encs}
    for name in NAMES:
        if name not in data or REFERENCE not in data[name]:
            continue
        w, h = ppm_size(W / "src" / f"{name}.ppm")
        ref = next(r for r in data[name][REFERENCE] if r[0] == REF_Q)
        cells = []
        for e in encs[1:]:
            pts = sorted(data[name].get(e, []), key=lambda r: r[2])
            if not pts:
                cells.append("—")
                continue
            if ref[2] < pts[0][2]:
                cells.append("<" + f"{pts[0][1] / ref[1]:.3f}")
                continue
            if ref[2] > pts[-1][2]:
                cells.append(">" + f"{pts[-1][1] / ref[1]:.3f}")
                continue
            for a, b in zip(pts, pts[1:]):
                if a[2] <= ref[2] <= b[2]:
                    t = 0 if b[2] == a[2] else (ref[2] - a[2]) / (b[2] - a[2])
                    by = math.exp(math.log(a[1]) + t * (math.log(b[1]) - math.log(a[1])))
                    ratios[e].append(by / ref[1])
                    cells.append(f"{by / ref[1]:.3f}")
                    break
        print(name.ljust(22), f"{ref[2]:9.2f}", f"{ref[1] * 8 / (w * h):8.3f}", *[c.rjust(13) for c in cells])
    print("median".ljust(40), *[(f"{statistics.median(ratios[e]):.3f}" if ratios[e] else "—").rjust(13)
                                for e in encs[1:]])
    print("\nEvery point: image encoder quality bytes bpp ssimulacra2")
    for name in NAMES:
        if name not in data:
            continue
        w, h = ppm_size(W / "src" / f"{name}.ppm")
        for e in encs:
            for q, by, s in sorted(data[name].get(e, [])):
                print(name, e, q, by, f"{by * 8 / (w * h):.3f}", f"{s:.2f}", sep="\t")


if __name__ == "__main__":
    {"prepare": prepare, "encode": encode, "speed": speed, "score": score}[sys.argv[1]]()
