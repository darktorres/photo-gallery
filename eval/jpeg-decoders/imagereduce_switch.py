"""ImageReduce's JPEG encode against jpegli baseline 4:4:4, on real inputs, for docs/temp/JPEG_ENCODERS.md "ImageReduce".

ImageReduce (its own repo, not this one) encodes with libvips's jpegsave at Q75, 4:4:4, optimize_coding, after a
thumbnail to 3840x2160 for images over 3840x2160 (mainwindow.cpp processImage). This runs that encode on the same
decoded pixels as jpegli's cjpegli at jpegli's quality grid, and scores every output with SSIMULACRA2 against the
pixels it was encoded from. Both outputs are decoded by the same libvips, so decoder differences don't enter.

    python eval/jpeg-decoders/imagereduce_switch.py run library --n 0 --out WORK\\library.csv
    python eval/jpeg-decoders/imagereduce_switch.py run camera --n 40 --out WORK\\camera.csv
    python eval/jpeg-decoders/imagereduce_switch.py summarize WORK\\library.csv WORK\\camera.csv

Windows only, run from the eval's venv (pyvips installed). Settings come from the environment (the repo's .env):

- IMAGEREDUCE_VIPS_BIN: ImageReduce's vips-dev bin folder (libvips 8.18.x DLLs). Required.
- JPEGLI_BUILD: jpegli's build folder, with tools\\cjpegli.exe and tools\\ssimulacra2.exe, and lib\\ for their DLLs.
  Default PG_ROOT\\tools\\jpeg-eval\\build\\jpegli.
- PG_DATA_ROOT: the Pinterest library (default PG_ROOT\\datasets\\pinterest). Its originals are the files without
  "mozjpeg" in the name: ImageReduce never reprocesses its own outputs.
- CAMERA_ROOT: the camera originals (default E:\\Cameras).
"""

import argparse
import csv
import itertools
import math
import os
import random
import statistics
import subprocess
import tempfile
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

GRID = [65, 70, 75, 80, 85, 90]  # jpegli qualities; the libjpeg quality scale, which jpegli's -q follows
REF_Q = 75  # ImageReduce's quality
BIG_W, BIG_H = 3840, 2160  # ImageReduce's thumbnail box for images over this size


def _env_path(name: str, default: Path | None) -> Path:
    value = os.environ.get(name)
    if value:
        return Path(value)
    if default is None:
        raise SystemExit(f"imagereduce_switch: set {name} (for example in the repo's .env)")
    return default


def _root() -> Path | None:
    return Path(os.environ["PG_ROOT"]) if os.environ.get("PG_ROOT") else None


def _setup_paths() -> tuple[Path, Path, Path]:
    """Put libvips and jpegli's DLLs on PATH before pyvips loads; returns (cjpegli, ssimulacra2, Pinterest root)."""
    root = _root()
    vips = _env_path("IMAGEREDUCE_VIPS_BIN", None)
    jpegli = _env_path("JPEGLI_BUILD", root and root / "tools" / "jpeg-eval" / "build" / "jpegli")
    os.environ["PATH"] = os.pathsep.join([str(vips), str(jpegli / "lib"), os.environ["PATH"]])
    pinterest = _env_path("PG_DATA_ROOT", root and root / "datasets" / "pinterest")
    return jpegli / "tools" / "cjpegli.exe", jpegli / "tools" / "ssimulacra2.exe", pinterest


CJPEGLI, SSIM2, PINTEREST = _setup_paths()
import pyvips  # noqa: E402  (after PATH: it loads ImageReduce's libvips)


def segs_meta(b: bytes) -> int:
    """Bytes of the APPn and COM segments (metadata) in a JPEG."""
    i, meta = 2, 0
    while i + 4 <= len(b) and b[i] == 0xFF:
        m = b[i + 1]
        if m in (0xDA, 0xD9):
            break
        n = (b[i + 2] << 8) | b[i + 3]
        if 0xE0 <= m <= 0xEF or m == 0xFE:
            meta += 2 + n
        i += 2 + n
    return meta


def write_ppm(im: pyvips.Image, path: Path) -> Path:
    """An 8-bit, 3-band image as a binary PPM (this libvips has no PPM saver)."""
    if im.format != "uchar" or im.bands != 3:
        raise ValueError(f"need 8-bit RGB, got {im.format} with {im.bands} bands")
    with path.open("wb") as fh:
        fh.write(b"P6\n%d %d\n255\n" % (im.width, im.height))
        fh.write(im.write_to_memory())
    return path


def score(ref: Path, dec: Path) -> float:
    out = subprocess.run([str(SSIM2), str(ref), str(dec)], capture_output=True, text=True, check=True)
    return float(out.stdout.strip().split()[-1])


def decode_out(jpg: bytes, tmp: Path, name: str) -> Path:
    """Every output is decoded the same way: libvips, to 8-bit sRGB."""
    im = pyvips.Image.new_from_buffer(jpg, "", access="sequential").colourspace("srgb")
    return write_ppm(im, tmp / f"{name}_dec.ppm")


def thumbnail_or_full(data: bytes, w: int, h: int) -> tuple[pyvips.Image, bool]:
    """ImageReduce's choice of image to encode: a thumbnail for big images, with integer aspect ratio (as C++ does)."""
    ratio = max(w // h, h // w)
    if (w > BIG_W or h > BIG_H) and ratio < 4:
        return pyvips.Image.thumbnail_buffer(data, BIG_W, height=BIG_H, size="down"), True
    return pyvips.Image.new_from_buffer(data, "", access="random"), False


def run_one(path: Path, tmp_root: str) -> dict:
    rec: dict = {"path": str(path), "in_bytes": path.stat().st_size}
    tmp = Path(tempfile.mkdtemp(dir=tmp_root))
    try:
        data = path.read_bytes()
        img = pyvips.Image.new_from_buffer(data, "", access="sequential")
        w, h = img.width, img.height
        rec.update(w=w, h=h, bands=img.bands)
        if img.bands == 4 or img.hasalpha():
            rec["skip"] = "alpha"
            return rec
        base, thumb = thumbnail_or_full(data, w, h)
        rec["thumb"] = int(thumb)
        # ImageReduce's own encode: metadata kept, as libvips does by default.
        vips_out = base.jpegsave_buffer(Q=REF_Q, subsample_mode="off", optimize_coding=True)
        rec["vips_bytes"] = len(vips_out)
        rec["vips_meta"] = segs_meta(vips_out)
        # The pixels both encoders get: 8-bit sRGB. A fresh load, since the sequential one is spent.
        base, _ = thumbnail_or_full(data, w, h)
        px = base.colourspace("srgb") if base.interpretation != "srgb" else base
        if px.format != "uchar":
            px = px.cast("uchar")
        if px.bands != 3:
            rec["skip"] = f"bands={px.bands}"
            return rec
        ref = write_ppm(px, tmp / "ref.ppm")
        rec["s_vips"] = score(ref, decode_out(vips_out, tmp, "vips"))
        for q in GRID:
            out = tmp / f"jpegli_{q}.jpg"
            subprocess.run(
                [str(CJPEGLI), str(ref), str(out), "-q", str(q), "-p", "0", "--chroma_subsampling=444"],
                check=True,
                capture_output=True,
            )
            jpg = out.read_bytes()
            rec[f"jp_bytes_{q}"] = len(jpg)
            rec[f"s_jp_{q}"] = score(ref, decode_out(jpg, tmp, f"jp{q}"))
    except (pyvips.Error, subprocess.CalledProcessError, OSError, ValueError) as e:
        rec["error"] = f"{type(e).__name__}: {str(e)[:200]}"
        return rec
    else:
        return rec
    finally:
        for f in tmp.glob("*"):
            f.unlink()
        tmp.rmdir()


def population(which: str) -> list[Path]:
    """Library: the Pinterest originals (not converted yet). Camera: the camera originals."""
    if which == "library":
        root, exts = PINTEREST, {".jpg", ".jpeg", ".png", ".webp"}
    else:
        root, exts = _env_path("CAMERA_ROOT", Path(r"E:\Cameras")), {".jpg", ".jpeg"}
    files = [
        Path(dp) / f
        for dp, _, names in os.walk(root)
        for f in names
        if Path(f).suffix.lower() in exts and "mozjpeg" not in f.lower()
    ]
    return sorted(files)


def run(which: str, n: int, seed: int, workers: int, out: Path) -> None:
    files = population(which)
    print(f"{which}: {len(files)} candidates", flush=True)
    if n and n < len(files):
        files = random.Random(seed).sample(files, n)
    tmp_root = tempfile.mkdtemp(prefix="isw_")
    rows = []
    t0 = time.time()
    with ProcessPoolExecutor(workers) as ex:
        futs = [ex.submit(run_one, f, tmp_root) for f in files]
        for k, fu in enumerate(as_completed(futs), 1):
            rows.append(fu.result())
            if k % 50 == 0 or k == len(files):
                print(f"{k}/{len(files)} {time.time() - t0:.0f}s", flush=True)
    keys = sorted({k for r in rows for k in r})
    with out.open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(fh, fieldnames=keys)
        writer.writeheader()
        writer.writerows(rows)
    print("wrote", out, len(rows), flush=True)


# --- summary ---------------------------------------------------------------------------------------------------------


def matched_bytes(r: dict) -> tuple[float, str]:
    """jpegli's image bytes at the score vips got: log(bytes) interpolated linearly in score along the grid.
    The flag is '>' when the grid's top scores below vips, '<' when its bottom scores above it."""
    target = float(r["s_vips"])
    pts = sorted((float(r[f"s_jp_{q}"]), math.log(float(r[f"jp_bytes_{q}"]))) for q in GRID)
    if target > pts[-1][0]:
        return math.exp(pts[-1][1]), ">"
    if target < pts[0][0]:
        return math.exp(pts[0][1]), "<"
    for (s0, b0), (s1, b1) in itertools.pairwise(pts):
        if s0 <= target <= s1:
            t = 0.0 if s1 == s0 else (target - s0) / (s1 - s0)
            return math.exp(b0 + t * (b1 - b0)), ""
    raise AssertionError("grid is not monotone in score")


def summarize(path: Path) -> None:
    rows = list(csv.DictReader(path.open(encoding="utf-8")))
    ok = [r for r in rows if not r.get("error") and not r.get("skip")]
    print(f"\n=== {path.name}: {len(rows)} rows, {len(ok)} encoded")
    for label, key in (("error", "error"), ("skipped", "skip")):
        n = sum(1 for r in rows if r.get(key))
        if n:
            print(f"  {label}: {n}")
    if not ok:
        return

    def fl(r: dict, key: str) -> float:
        return float(r[key])

    def med(xs) -> float:
        return statistics.median(list(xs))

    vips_kept = [r for r in ok if fl(r, "vips_bytes") > fl(r, "in_bytes")]
    print(
        f"  thumbnail path: {sum(r['thumb'] == '1' for r in ok)}; with metadata beyond JFIF: "
        f"{sum(int(r['vips_meta']) > 190 for r in ok)}"
    )
    print(
        f"  input median {med(fl(r, 'in_bytes') for r in ok):.0f} bytes; vips output median "
        f"{med(fl(r, 'vips_bytes') for r in ok):.0f}"
    )
    print(f"  today: output bigger than input (kept as original): {len(vips_kept)}/{len(ok)}")
    print(f"  SSIMULACRA2 of vips Q{REF_Q}: median {med(fl(r, 's_vips') for r in ok):.2f}")

    ratios, flags = [], {"": 0, ">": 0, "<": 0}
    for r in ok:
        b, flag = matched_bytes(r)
        flags[flag] += 1
        ratios.append(b / (fl(r, "vips_bytes") - int(r["vips_meta"])))
    print(
        f"  matched score, image bytes jpegli/vips: median {med(ratios):.3f}; outside the grid: "
        f"{flags['>']} above, {flags['<']} below"
    )

    base_total = sum(min(fl(r, "vips_bytes"), fl(r, "in_bytes")) for r in ok)
    print("  fixed jpegli quality, metadata copied, keep-original rule applied:")
    print(f"    {'Q':>4} {'median dScore':>14} {'median size/vips':>17} {'kept':>6} {'total bytes':>14} {'vs today':>9}")
    print(f"    {'vips':>4} {0:>14.2f} {1.0:>17.3f} {len(vips_kept):>6} {base_total:>14,.0f} {'':>9}")
    for q in GRID:
        ds, rs, kept, total = [], [], 0, 0.0
        for r in ok:
            jt = fl(r, f"jp_bytes_{q}") + int(r["vips_meta"])
            ds.append(fl(r, f"s_jp_{q}") - fl(r, "s_vips"))
            rs.append(jt / fl(r, "vips_bytes"))
            if jt > fl(r, "in_bytes"):
                kept += 1
                total += fl(r, "in_bytes")
            else:
                total += jt
        print(f"    {q:>4} {med(ds):>+14.2f} {med(rs):>17.3f} {kept:>6} {total:>14,.0f} {100 * (total / base_total - 1):>+8.1f}%")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run", help="encode and score a set")
    r.add_argument("which", choices=["library", "camera"])
    r.add_argument("--n", type=int, default=0, help="random sample size (0: all)")
    r.add_argument("--seed", type=int, default=1)
    r.add_argument("--workers", type=int, default=6)
    r.add_argument("--out", type=Path, required=True)
    s = sub.add_parser("summarize", help="summarize run CSVs")
    s.add_argument("csv", type=Path, nargs="+")
    a = ap.parse_args()
    if a.cmd == "run":
        run(a.which, a.n, a.seed, a.workers, a.out)
    else:
        for p in a.csv:
            summarize(p)


if __name__ == "__main__":
    main()
