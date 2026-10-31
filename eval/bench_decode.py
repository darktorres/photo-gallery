"""Decode + resize benchmark: which pipeline should feed the embedder?

Usage: python bench_decode.py [--count 1000] [--processes 12]

Every pipeline turns a JPEG file into the 320x320 RGB input SSCD sees (whole image squashed to a
square, EXIF orientation applied):
  pil_full   PIL full-resolution decode + bilinear resize (the current harness, the reference)
  pil_draft  PIL draft(): libjpeg-turbo decodes at 1/2, 1/4 or 1/8 scale in the DCT, then resize
  vips       libvips thumbnail: shrink-on-load in the decoder + Lanczos3 reduce
  nvjpeg     batched GPU decode (torchvision.io.decode_jpeg on CUDA) + antialiased GPU resize

Reports img/s in one process and across --processes processes (CPU pipelines; nvJPEG runs batched
in one process), and how far each pipeline moves the SSCD fp16 embedding away from the reference
(cosine). Files are read once before timing, so this measures decode, not disk. Run it on an idle
machine: other jobs skew the timings.
"""

import argparse
import io
import json
import random
import time
from multiprocessing import Pool

import numpy as np
from PIL import Image, ImageOps

from common import DATA_ROOT, WORK_DIR, image_list

SIDE = 320
CPU_PIPELINES = ("pil_full", "pil_draft", "vips")


def _pil(data: bytes, draft: bool) -> np.ndarray:
    with Image.open(io.BytesIO(data)) as im:
        if draft:
            im.draft("RGB", (SIDE, SIDE))
        im = ImageOps.exif_transpose(im).convert("RGB")
        return np.asarray(im.resize((SIDE, SIDE), Image.Resampling.BILINEAR))


def _vips(data: bytes, width: int = SIDE, height: int = SIDE) -> np.ndarray:
    import pyvips

    im = pyvips.Image.thumbnail_buffer(data, width, height=height, size="force")
    if im.interpretation not in ("srgb", "rgb", "b-w") or im.bands not in (1, 3):
        im = im.colourspace("srgb")
    if im.hasalpha():
        im = im.flatten(background=[255, 255, 255])
    if im.bands == 1:
        im = im.bandjoin([im, im])
    return np.ndarray(buffer=im.write_to_memory(), dtype=np.uint8, shape=(im.height, im.width, 3))


def decode(job: tuple[str, str]) -> np.ndarray | None:
    pipeline, rel = job
    data = (DATA_ROOT / rel).read_bytes()
    try:
        if pipeline == "vips":
            return _vips(data)
        return _pil(data, draft=pipeline == "pil_draft")
    except Exception:
        return None


def exif_orientation(data: bytes) -> int:
    """EXIF orientation from the header only (no pixel decode)."""
    with Image.open(io.BytesIO(data)) as im:
        return int(im.getexif().get(0x0112, 1))


def orient(t, orientation: int):
    """Apply an EXIF orientation to a CHW tensor (same result as ImageOps.exif_transpose)."""
    import torch

    if orientation in (2, 4, 5, 7):  # mirrored variants
        t = torch.flip(t, dims=[2])
    rotations = {3: 2, 4: 2, 5: 1, 6: 3, 7: 3, 8: 1}  # quarter turns counter-clockwise
    return torch.rot90(t, k=rotations.get(orientation, 0), dims=[1, 2])


def nvjpeg_run(files: list[str], batch: int) -> tuple[list, float, int]:
    """Batched GPU decode. Returns (HWC uint8 arrays or None, seconds, CPU fallbacks)."""
    import torch
    import torch.nn.functional as F
    from torchvision.io import ImageReadMode, decode_jpeg

    out: list = [None] * len(files)
    fallbacks = 0
    torch.cuda.synchronize()
    start = time.perf_counter()
    for s in range(0, len(files), batch):
        chunk = files[s : s + batch]
        blobs = [(DATA_ROOT / rel).read_bytes() for rel in chunk]
        tensors = [torch.frombuffer(bytearray(b), dtype=torch.uint8) for b in blobs]
        try:
            decoded = decode_jpeg(tensors, mode=ImageReadMode.RGB, device="cuda")
        except RuntimeError:  # one unsupported file fails the batch: decode one by one
            decoded = []
            for t in tensors:
                try:
                    decoded.append(decode_jpeg(t, mode=ImageReadMode.RGB, device="cuda"))
                except RuntimeError:
                    decoded.append(None)
        for k, (img, data) in enumerate(zip(decoded, blobs, strict=True)):
            if img is None:  # CMYK and other formats nvJPEG rejects: CPU path
                fallbacks += 1
                out[s + k] = _pil(data, draft=False)
                continue
            img = orient(img, exif_orientation(data))
            img = F.interpolate(img[None].float(), size=(SIDE, SIDE), mode="bilinear", antialias=True, align_corners=False)[0]
            out[s + k] = img.round().clamp(0, 255).byte().permute(1, 2, 0)
        torch.cuda.synchronize()
    seconds = time.perf_counter() - start
    return [None if o is None else (o if isinstance(o, np.ndarray) else o.cpu().numpy()) for o in out], seconds, fallbacks


def embed_inputs(arrays: list[np.ndarray | None]) -> np.ndarray:
    """SSCD fp16 embeddings of ready 320x320 RGB inputs (zero vector where missing)."""
    import torch

    from models import SPECS, load_model, run

    spec = SPECS["sscd_fp16"]
    model = load_model(spec, torch.device("cuda"))
    mean = torch.tensor(spec.mean, device="cuda").view(1, 3, 1, 1)
    std = torch.tensor(spec.std, device="cuda").view(1, 3, 1, 1)
    out = np.zeros((len(arrays), 512), np.float32)
    with torch.inference_mode():
        for s in range(0, len(arrays), 64):
            chunk = [a if a is not None else np.zeros((SIDE, SIDE, 3), np.uint8) for a in arrays[s : s + 64]]
            x = torch.from_numpy(np.stack(chunk)).cuda().permute(0, 3, 1, 2).float() / 255
            y = torch.nn.functional.normalize(run(model, spec, (x - mean) / std), dim=1)
            out[s : s + len(chunk)] = y.cpu().numpy()
    for i, a in enumerate(arrays):
        if a is None:
            out[i] = 0
    return out


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--count", type=int, default=1000)
    parser.add_argument("--processes", type=int, default=12)
    parser.add_argument("--batch", type=int, default=32)
    parser.add_argument("--seed", type=int, default=0)
    args = parser.parse_args()

    jpegs = [f for f in image_list() if f.lower().endswith((".jpg", ".jpeg"))]
    files = sorted(random.Random(args.seed).sample(jpegs, args.count))
    sizes = []
    for rel in files:  # warm the OS file cache and record sizes
        data = (DATA_ROOT / rel).read_bytes()
        with Image.open(io.BytesIO(data)) as im:
            sizes.append((len(data), im.width * im.height))
    mb = np.array([s[0] for s in sizes]) / 1e6
    mp = np.array([s[1] for s in sizes]) / 1e6
    print(
        f"{len(files)} JPEGs: median {np.median(mp):.2f} MP / {np.median(mb):.2f} MB, p90 {np.quantile(mp, 0.9):.2f} MP",
        flush=True,
    )

    outputs: dict[str, list] = {}
    rows = []
    for name in CPU_PIPELINES:
        jobs = [(name, rel) for rel in files]
        start = time.perf_counter()
        outputs[name] = [decode(j) for j in jobs]
        single = len(files) / (time.perf_counter() - start)
        with Pool(args.processes) as pool:
            pool.map(decode, jobs[: args.processes * 4])  # spawn + import outside the timing
            start = time.perf_counter()
            pool.map(decode, jobs, chunksize=8)
            multi = len(files) / (time.perf_counter() - start)
        rows.append(
            {
                "pipeline": name,
                "img_s_1proc": single,
                f"img_s_{args.processes}proc": multi,
                "failed": sum(o is None for o in outputs[name]),
            }
        )
        print(f"  {name}: {single:.0f} img/s (1 process), {multi:.0f} img/s ({args.processes} processes)", flush=True)

    nvjpeg_run(files[: args.batch * 2], args.batch)  # CUDA init + warm-up outside the timing
    outputs["nvjpeg"], seconds, fallbacks = nvjpeg_run(files, args.batch)
    rows.append(
        {
            "pipeline": "nvjpeg",
            "img_s_1proc": len(files) / seconds,
            f"img_s_{args.processes}proc": None,
            "failed": sum(o is None for o in outputs["nvjpeg"]),
            "cpu_fallbacks": fallbacks,
        }
    )
    print(f"  nvjpeg: {len(files) / seconds:.0f} img/s (1 process, batch {args.batch}), {fallbacks} CPU fallbacks", flush=True)

    emb = {name: embed_inputs(outs) for name, outs in outputs.items()}
    ref = emb["pil_full"]
    for row in rows:
        cos = (emb[row["pipeline"]] * ref).sum(1)
        valid = np.linalg.norm(emb[row["pipeline"]], axis=1) > 0
        cos = cos[valid & (np.linalg.norm(ref, axis=1) > 0)]
        row.update({"cos_min": float(cos.min()), "cos_p1": float(np.quantile(cos, 0.01)), "cos_p50": float(np.median(cos))})

    summary = {"count": len(files), "median_mp": float(np.median(mp)), "processes": args.processes, "rows": rows}
    (WORK_DIR / "bench_decode.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    multi_key = f"img_s_{args.processes}proc"
    print(
        f"\n| pipeline | img/s, 1 process | img/s, {args.processes} processes | failed | "
        "SSCD cosine vs pil_full: min / p1 / p50 |"
    )
    print("|---|---|---|---|---|")
    for r in rows:
        multi = "–" if r[multi_key] is None else f"{r[multi_key]:.0f}"
        print(
            f"| {r['pipeline']} | {r['img_s_1proc']:.0f} | {multi} | {r['failed']} | "
            f"{r['cos_min']:.4f} / {r['cos_p1']:.4f} / {r['cos_p50']:.4f} |"
        )


if __name__ == "__main__":
    main()
