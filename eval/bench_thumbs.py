"""Thumbnail benchmark: how to make the grid thumbnails, and which format and quality to store them in.

Usage: python bench_thumbs.py [--images 300] [--heights 256 384 512] [--workers 10] [--timing 60] [--no-sscd]

Thumbnails are for the justified grid: the height is the row height in device pixels, the width follows the
aspect ratio (at most 3x the height, for panoramas), and an image smaller than that is never enlarged. The images
are a random sample of the work dir's files.txt.

1. Resize. Each pipeline makes the same exact size from the original file:
     full_lanczos    full decode + Lanczos3 (PIL, 8-bit)
     full_bilinear   full decode + antialiased bilinear
     draft_lanczos   libjpeg-turbo decodes at 1/2, 1/4 or 1/8 scale in the DCT, staying >= the target, then Lanczos3
     draft2_lanczos  the same, but staying >= 2x the target before Lanczos3
     draft4_lanczos  ... >= 4x
     vips            libvips thumbnail (shrink-on-load + Lanczos3)
   Scored by SSIMULACRA2 against a float Lanczos3 resize of the full decode, and on a zone plate, where aliasing
   shows as moire rings (bench_thumbs_zoneplate.png).
2. Encode. The full_lanczos thumbnail in each format over a quality sweep: bytes, SSIMULACRA2 against the
   uncompressed thumbnail, encode and decode time (one thread, files in RAM). Then the bytes each format needs for
   the same quality (interpolated at fixed scores) and the projected size of 70M thumbnails.
3. Re-use: SSCD embedding of the thumbnail vs of the original (cosine): could a later model run on thumbnails
   instead of re-reading the originals (PLAN open decision 1)?

SSIMULACRA2 (-inf..100; ~90 visually lossless, 70 high, 50 medium quality) is libjxl's reference tool, and
cjpegli is run as a program, so its encode speed is its own report. Both come from libjxl 0.11.1, the last
release that ships cjpegli (jpegli moved out of libjxl afterwards; 0.12's ssimulacra2 scores identically),
under PG_TOOLS_DIR (default D:\\PhotoGallery\\tools). Timings run in one
process after the parallel quality pass; other load on the machine skews them.
Writes WORK_DIR/bench_thumbs.json.
"""

import argparse
import io
import json
import os
import random
import re
import shutil
import subprocess
import tempfile
import time
from collections.abc import Callable
from functools import cache
from multiprocessing import Pool
from pathlib import Path

import imagecodecs
import numpy as np
from PIL import AvifImagePlugin, Image, ImageDraw

from bench_decode import _vips
from common import CACHE_DIR, DATA_ROOT, WORK_DIR, image_list, load_rgb

TOOLS_DIR = Path(os.environ.get("PG_TOOLS_DIR", r"D:\PhotoGallery\tools"))
SSIMULACRA2 = TOOLS_DIR / "libjxl-0.11.1" / "ssimulacra2.exe"
CJPEGLI = TOOLS_DIR / "libjxl-0.11.1" / "cjpegli.exe"

MAX_ASPECT = 3.0  # panoramas: width capped at this x the row height
LIBRARY_SIZE = 70e6
SCORE_TARGETS = (60, 70, 80, 85)
RESIZE_METHODS = ("full_lanczos", "full_bilinear", "draft_lanczos", "draft2_lanczos", "draft4_lanczos", "vips")
LANCZOS = Image.Resampling.LANCZOS
AvifImagePlugin.DEFAULT_MAX_THREADS = 1  # one thread per image, like the encoders: the UI decodes many at once


def thumb_size(width: int, height: int, row: int) -> tuple[int, int]:
    th = min(row, height)
    tw = max(1, round(width * th / height))
    if tw > MAX_ASPECT * row:
        tw = round(MAX_ASPECT * row)
        th = max(1, round(height * tw / width))
    return tw, th


def oriented_size(data: bytes) -> tuple[int, int]:
    with Image.open(io.BytesIO(data)) as im:
        swap = im.getexif().get(0x0112, 1) in (5, 6, 7, 8)
        return (im.height, im.width) if swap else im.size


def resize(method: str, data: bytes, size: tuple[int, int]) -> np.ndarray:
    if method == "vips":
        return _vips(data, *size)
    margin = {"draft_lanczos": 1, "draft2_lanczos": 2, "draft4_lanczos": 4}.get(method)
    draft = (margin * size[0], margin * size[1]) if margin else None
    im = load_rgb(io.BytesIO(data), draft=draft)
    flt = Image.Resampling.BILINEAR if method == "full_bilinear" else LANCZOS
    return np.asarray(im.resize(size, flt))


def float_lanczos(im: Image.Image, size: tuple[int, int]) -> np.ndarray:
    """The reference: Lanczos3 on float channels, rounded once at the end."""
    planes = [np.asarray(c.convert("F").resize(size, LANCZOS)) for c in im.split()]
    return np.clip(np.round(np.stack(planes, axis=-1)), 0, 255).astype(np.uint8)


# --- Encoders: (settings, encode(rgb uint8 array, setting) -> bytes, decode(bytes) -> rgb array) ---


def _pil_encode(fmt: str, **kwargs) -> Callable[[np.ndarray, float], bytes]:
    def encode(a: np.ndarray, q: float) -> bytes:
        buf = io.BytesIO()
        Image.fromarray(a).save(buf, fmt, quality=int(q), **kwargs)
        return buf.getvalue()

    return encode


def _pil_decode(data: bytes) -> np.ndarray:
    with Image.open(io.BytesIO(data)) as im:
        return np.asarray(im.convert("RGB"))


_JPEGLI_MPS = [float("nan")]  # encode speed of the last cjpegli call, as the tool reports it


def _jpegli(*options: str) -> Callable[[np.ndarray, float], bytes]:
    def encode(a: np.ndarray, q: float) -> bytes:
        src, dst = _tmp() / "jpegli.ppm", _tmp() / "jpegli.jpg"
        Image.fromarray(a).save(src)
        out = subprocess.run(
            [str(CJPEGLI), str(src), str(dst), "-q", str(int(q)), *options], capture_output=True, text=True, check=True
        )
        m = re.search(r"([\d.]+) MP/s", out.stdout + out.stderr)
        _JPEGLI_MPS[0] = float(m[1]) if m else float("nan")
        return dst.read_bytes()

    return encode


def _jxl_encode(a: np.ndarray, d: float) -> bytes:
    return imagecodecs.jpegxl_encode(a, distance=d, effort=7, numthreads=1)


def _jxl_decode(data: bytes) -> np.ndarray:
    return imagecodecs.jpegxl_decode(data, numthreads=1)


def _mozjpeg_encode(a: np.ndarray, q: float) -> bytes:
    return imagecodecs.mozjpeg_encode(a, level=int(q))  # mozjpeg defaults: 4:2:0, progressive, trellis


Q8 = (40, 50, 60, 70, 75, 80, 85, 90, 95)
Q4 = (50, 60, 70, 80, 90)
CODECS: dict[str, tuple[tuple[float, ...], Callable[[np.ndarray, float], bytes], Callable[[bytes], np.ndarray]]] = {
    "jpeg": (Q8, _pil_encode("JPEG", optimize=True, subsampling="4:2:0"), _pil_decode),
    "jpeg_progressive": (Q4, _pil_encode("JPEG", optimize=True, subsampling="4:2:0", progressive=True), _pil_decode),
    "mozjpeg": (Q8, _mozjpeg_encode, _pil_decode),
    "jpegli": (Q8, _jpegli(), _pil_decode),  # its defaults: adaptive quantization, 4:4:4, progressive
    "jpegli_420_seq": (Q4, _jpegli("--chroma_subsampling=420", "-p", "0"), _pil_decode),
    "webp": (Q8, _pil_encode("WEBP", method=4), _pil_decode),
    "webp_m6": ((60, 75, 90), _pil_encode("WEBP", method=6), _pil_decode),
    "avif": ((30, 40, 50, 60, 70, 80, 90), _pil_encode("AVIF", speed=6, max_threads=1), _pil_decode),
    "jxl": ((0.5, 0.75, 1, 1.5, 2, 3, 4, 6), _jxl_encode, _jxl_decode),  # butteraugli distance (lower = better)
}


# --- SSIMULACRA2 via libjxl's tool ---


@cache
def _tmp() -> Path:
    """This process's scratch folder (pool workers each get their own)."""
    (CACHE_DIR / "tmp").mkdir(parents=True, exist_ok=True)
    return Path(tempfile.mkdtemp(prefix="thumbs_", dir=CACHE_DIR / "tmp"))


def ssimulacra2(reference: Path, a: np.ndarray) -> float:
    path = _tmp() / "distorted.ppm"
    Image.fromarray(a).save(path)
    out = subprocess.run([str(SSIMULACRA2), str(reference), str(path)], capture_output=True, text=True, check=True)
    return float(out.stdout.strip())


def save_reference(a: np.ndarray, name: str = "reference.ppm") -> Path:
    path = _tmp() / name
    Image.fromarray(a).save(path)
    return path


# --- Quality pass (one image per task, in parallel) ---


def quality(job: tuple[str, tuple[int, ...]]) -> dict | None:
    rel, heights = job
    try:
        data = (DATA_ROOT / rel).read_bytes()
        full = load_rgb(io.BytesIO(data))
    except Exception:
        return None
    out: dict = {"size": full.size, "resize": {}, "encode": {}}
    for row in heights:
        size = thumb_size(*full.size, row)
        ref = save_reference(float_lanczos(full, size))
        out["resize"][row] = {m: ssimulacra2(ref, resize(m, data, size)) for m in RESIZE_METHODS}
        thumb = np.asarray(full.resize(size, LANCZOS))
        ref = save_reference(thumb)
        per_codec: dict = {}
        for codec, (settings, encode, decode) in CODECS.items():
            per_codec[codec] = {}
            for s in settings:
                blob = encode(thumb, s)
                per_codec[codec][s] = (len(blob), ssimulacra2(ref, decode(blob)))
        out["encode"][row] = per_codec
    return out


# --- Timing pass (one process) ---


def timed(fn: Callable, *args) -> tuple:
    """(fn(*args), milliseconds)."""
    start = time.perf_counter()
    result = fn(*args)
    return result, (time.perf_counter() - start) * 1000


def timing(files: list[str], heights: tuple[int, ...]) -> dict:
    out: dict = {"resize": {}, "encode": {}, "decode": {}}
    blobs = [(DATA_ROOT / rel).read_bytes() for rel in files]
    for row in heights:
        resize_ms: dict[str, list[float]] = {m: [] for m in RESIZE_METHODS}
        enc: dict[str, dict] = {c: {s: [] for s in CODECS[c][0]} for c in CODECS}
        dec: dict[str, dict] = {c: {s: [] for s in CODECS[c][0]} for c in CODECS}
        for data in blobs:
            try:
                size = thumb_size(*oriented_size(data), row)
                thumb = resize("full_lanczos", data, size)
            except Exception:
                continue
            for m in RESIZE_METHODS:
                resize_ms[m].append(timed(resize, m, data, size)[1])
            for codec, (settings, encode, decode) in CODECS.items():
                for s in settings:
                    blob, enc_ms = timed(encode, thumb, s)
                    if codec.startswith("jpegli"):  # a process per call: use the tool's own speed report
                        enc_ms = thumb.shape[0] * thumb.shape[1] / 1e6 / _JPEGLI_MPS[0] * 1000
                    enc[codec][s].append(enc_ms)
                    dec[codec][s].append(timed(decode, blob)[1])
        out["resize"][row] = {m: float(np.median(v)) for m, v in resize_ms.items()}
        out["encode"][row] = {c: {str(s): float(np.nanmedian(v)) for s, v in d.items()} for c, d in enc.items()}
        out["decode"][row] = {c: {str(s): float(np.median(v)) for s, v in d.items()} for c, d in dec.items()}
    return out


# --- Zone plate (aliasing) ---


def zone_plate(n: int = 2048) -> np.ndarray:
    """cos(pi r^2 / n): the local frequency rises from 0 at the center to Nyquist at the edge midpoints."""
    y, x = np.mgrid[:n, :n] - n / 2
    v = 0.5 + 0.5 * np.cos(np.pi * (x * x + y * y) / n)
    return np.repeat((v * 255).round().astype(np.uint8)[..., None], 3, axis=2)


def zone_plate_test(side: int = 256) -> dict:
    plate = zone_plate()
    buf = io.BytesIO()
    Image.fromarray(plate).save(buf, "JPEG", quality=95, subsampling="4:4:4")
    data = buf.getvalue()
    size = (side, side)
    ref_arr = float_lanczos(Image.fromarray(plate), size)
    ref = save_reference(ref_arr, "zone_reference.ppm")
    tiles = [("reference", ref_arr)] + [(m, resize(m, data, size)) for m in RESIZE_METHODS]
    scores = {m: ssimulacra2(ref, a) for m, a in tiles[1:]}
    sheet = Image.new("RGB", (side * len(tiles), side + 20), "white")
    draw = ImageDraw.Draw(sheet)
    for k, (name, a) in enumerate(tiles):
        sheet.paste(Image.fromarray(a), (k * side, 20))
        draw.text((k * side + 4, 4), name, fill="black")
    sheet.save(WORK_DIR / "bench_thumbs_zoneplate.png")
    return scores


# --- Re-use for embeddings ---


def sscd_reuse(files: list[str], heights: tuple[int, ...]) -> dict:
    """Cosine between SSCD fp16 on the original and on the thumbnail (uncompressed, and as JPEG q85)."""
    import torch

    from models import SPECS, build_transform, load_model, run

    spec = SPECS["sscd_fp16"]
    model = load_model(spec, torch.device("cuda"))
    transform = build_transform(spec)

    def embed(images: list[Image.Image]) -> np.ndarray:
        with torch.inference_mode():
            x = torch.stack([transform(im) for im in images]).cuda()
            return torch.nn.functional.normalize(run(model, spec, x), dim=1).cpu().numpy()

    to_jpeg = _pil_encode("JPEG", optimize=True, subsampling="4:2:0")
    cos: dict[tuple[int, str], list[np.ndarray]] = {(row, k): [] for row in heights for k in ("raw", "jpeg85")}
    for s in range(0, len(files), 32):  # in chunks: full-size originals are large
        originals = []
        for rel in files[s : s + 32]:
            try:
                originals.append(load_rgb(DATA_ROOT / rel))
            except Exception:
                continue
        ref = embed(originals)
        for row in heights:
            raw = [im.resize(thumb_size(im.width, im.height, row), LANCZOS) for im in originals]
            jpeg = [Image.open(io.BytesIO(to_jpeg(np.asarray(t), 85))).convert("RGB") for t in raw]
            cos[(row, "raw")].append((embed(raw) * ref).sum(axis=1))
            cos[(row, "jpeg85")].append((embed(jpeg) * ref).sum(axis=1))
    out: dict = {}
    for (row, kind), parts in cos.items():
        c = np.concatenate(parts)
        out.setdefault(str(row), {})[kind] = {"min": float(c.min()), "p1": float(np.percentile(c, 1)), "p50": float(np.median(c))}
    return out


# --- Summaries ---


def iso_quality(curve: list[tuple[float, float]], target: float) -> float | None:
    """Mean bytes at a mean score, interpolated along a codec's (score, bytes) sweep."""
    pts = sorted(curve)
    scores = [s for s, _ in pts]
    if not scores or target < scores[0] or target > scores[-1]:
        return None
    return float(np.interp(target, scores, [b for _, b in pts]))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--images", type=int, default=300)
    parser.add_argument("--heights", type=int, nargs="+", default=[256, 384, 512])
    parser.add_argument("--workers", type=int, default=10)
    parser.add_argument("--timing", type=int, default=60, help="images in the single-process timing pass")
    parser.add_argument("--no-sscd", action="store_true")
    args = parser.parse_args()
    heights = tuple(args.heights)

    random.seed(0)
    files = random.sample(image_list(), args.images)
    start = time.perf_counter()
    with Pool(args.workers) as pool:
        results = [r for r in pool.imap(quality, [(f, heights) for f in files], chunksize=2) if r]
    print(f"quality pass: {len(results)}/{len(files)} images, {time.perf_counter() - start:.0f} s")

    report: dict = {"images": len(results), "heights": heights, "resize": {}, "encode": {}, "iso_quality": {}}
    for row in heights:
        report["resize"][row] = {}
        for m in RESIZE_METHODS:
            v = np.array([r["resize"][row][m] for r in results])
            report["resize"][row][m] = {"mean": float(v.mean()), "p10": float(np.percentile(v, 10))}
        report["encode"][row] = {}
        for codec, (settings, _, _) in CODECS.items():
            report["encode"][row][codec] = {}
            for s in settings:
                b = np.array([r["encode"][row][codec][s][0] for r in results])
                q = np.array([r["encode"][row][codec][s][1] for r in results])
                report["encode"][row][codec][str(s)] = {
                    "bytes_mean": float(b.mean()),
                    "bytes_median": float(np.median(b)),
                    "score_mean": float(q.mean()),
                    "score_p10": float(np.percentile(q, 10)),
                }
        report["iso_quality"][row] = {
            str(t): {
                c: iso_quality([(v["score_mean"], v["bytes_mean"]) for v in report["encode"][row][c].values()], t) for c in CODECS
            }
            for t in SCORE_TARGETS
        }

    report["zone_plate"] = zone_plate_test()
    report["timing_ms"] = timing(files[: args.timing], heights)
    if not args.no_sscd:
        report["sscd_cosine"] = sscd_reuse(files, heights)
    (WORK_DIR / "bench_thumbs.json").write_text(json.dumps(report, indent=1), encoding="utf-8")
    for d in (CACHE_DIR / "tmp").glob("thumbs_*"):  # this process's and the workers'
        shutil.rmtree(d, ignore_errors=True)

    t = report["timing_ms"]
    print("\n## Resize (SSIMULACRA2 vs float Lanczos3: mean / p10; ms)")
    print("| method | " + " | ".join(f"{h}px" for h in heights) + " | zone plate |")
    print("|---|" + "---|" * (len(heights) + 1))
    for m in RESIZE_METHODS:
        cells = [
            f"{report['resize'][h][m]['mean']:.1f} / {report['resize'][h][m]['p10']:.1f}; {t['resize'][h][m]:.1f} ms"
            for h in heights
        ]
        print(f"| {m} | " + " | ".join(cells) + f" | {report['zone_plate'][m]:.1f} |")
    for row in heights:
        print(f"\n## Encode at {row}px (mean bytes, SSIMULACRA2 mean / p10, encode / decode ms)")
        print("| codec | setting | bytes | score | enc ms | dec ms |")
        print("|---|---|---|---|---|---|")
        for codec, per in report["encode"][row].items():
            for s, v in per.items():
                print(
                    f"| {codec} | {s} | {v['bytes_mean']:.0f} | {v['score_mean']:.1f} / {v['score_p10']:.1f} "
                    f"| {t['encode'][row][codec][s]:.2f} | {t['decode'][row][codec][s]:.2f} |"
                )
        print(f"\n## Same quality at {row}px: mean bytes (70M thumbnails, GB)")
        print("| codec | " + " | ".join(f"score {s}" for s in SCORE_TARGETS) + " |")
        print("|---|" + "---|" * len(SCORE_TARGETS))
        for codec in CODECS:
            cells = []
            for s in SCORE_TARGETS:
                b = report["iso_quality"][row][str(s)][codec]
                cells.append("-" if b is None else f"{b:.0f} ({b * LIBRARY_SIZE / 1e9:.0f} GB)")
            print(f"| {codec} | " + " | ".join(cells) + " |")
    if "sscd_cosine" in report:
        print("\n## SSCD on the thumbnail vs on the original (cosine min / p1 / p50)")
        for row, kinds in report["sscd_cosine"].items():
            print(f"  {row}px: " + "; ".join(f"{k} {v['min']:.3f} / {v['p1']:.3f} / {v['p50']:.3f}" for k, v in kinds.items()))


if __name__ == "__main__":
    main()
