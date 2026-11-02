"""A small synthetic photo library for profiling: the same kinds of files as a real one, far fewer than the pinterest set.

    python tools/profiling/synthetic_library.py OUT [--folders 80] [--per-folder 50] [--seed 1] [--jobs N]

Writes OUT/folder-NN/ with --per-folder images each: mostly JPEGs (landscape, portrait and 12 MP, some with an EXIF
orientation and a capture date), plus PNGs (some with alpha), animated GIFs and WebPs. The pixels are smooth gradients
with a little noise, so the files compress like photographs rather than to nothing.

Every file's kind, date and pixels come from its own seed, drawn in a fixed order from --seed before any file is
written, so the same arguments always write the same files, however many worker processes (--jobs, default: all
cores) write them.
"""

import argparse
import os
import random
from datetime import datetime, timedelta
from multiprocessing import Pool
from pathlib import Path

import numpy as np
from PIL import Image

# (width, height, weight) of the JPEG sizes.
JPEG_SIZES = [(1920, 1080, 60), (1080, 1920, 20), (4032, 3024, 20)]


def picture(seed: int, width: int, height: int, alpha: bool = False) -> Image.Image:
    gen = np.random.default_rng(seed)
    y, x = np.indices((height, width), dtype=np.float32)
    phase = gen.uniform(0, 6.28, size=3).astype(np.float32)
    channels = [
        128 + 110 * np.sin(x / (width / gen.uniform(2, 6)) + phase[c]) * np.cos(y / (height / gen.uniform(2, 6)))
        for c in range(3)
    ]
    rgb = np.stack(channels, axis=-1).astype(np.float32)
    rgb += gen.normal(0, 3, size=(height, width, 3)).astype(np.float32)
    rgb = np.clip(rgb, 0, 255).astype(np.uint8)
    if alpha:
        a = np.clip(255 * (x / width), 0, 255).astype(np.uint8)[..., None]
        return Image.fromarray(np.concatenate([rgb, a], axis=-1), "RGBA")
    return Image.fromarray(rgb, "RGB")


def exif_with(orientation: int | None, when: datetime) -> bytes:
    exif = Image.Exif()
    exif[0x0132] = when.strftime("%Y:%m:%d %H:%M:%S")  # DateTime
    if orientation is not None:
        exif[0x0112] = orientation  # Orientation
    return exif.tobytes()


def write_file(job: tuple[Path, str, datetime, int]) -> Path:
    path, kind, when, seed = job
    rng = random.Random(seed)
    if kind == "jpg":
        width, height, _ = rng.choices(JPEG_SIZES, weights=[w for *_, w in JPEG_SIZES])[0]
        img = picture(seed, width, height)
        orientation = rng.choice([6, 8]) if rng.random() < 0.1 else None
        img.save(path, "JPEG", quality=85, exif=exif_with(orientation, when))
    elif kind == "png":
        img = picture(seed, 800, 600, alpha=rng.random() < 0.3)
        img.save(path, "PNG", optimize=False)
    elif kind == "gif":
        frames = [picture(seed + k, 320, 240).convert("P") for k in range(3)]
        frames[0].save(path, "GIF", save_all=True, append_images=frames[1:], duration=120, loop=0)
    else:
        picture(seed, 1024, 768).save(path, "WEBP", quality=80)
    return path


def plan(folders: int, per_folder: int, seed: int, out: Path) -> list[tuple[Path, str, datetime, int]]:
    """Every file's job, drawn in a fixed order from the master seed."""
    master = random.Random(seed)
    start = datetime(2024, 1, 1)
    jobs = []
    for f in range(folders):
        folder = out / f"folder-{f:02d}"
        for i in range(per_folder):
            when = start + timedelta(days=master.randrange(900), seconds=master.randrange(86400))
            kind = master.choices(["jpg", "png", "gif", "webp"], weights=[70, 15, 5, 5])[0]
            jobs.append((folder / f"IMG_{f:02d}_{i:03d}.{kind}", kind, when, master.randrange(2**32)))
    return jobs


def main() -> None:
    parser = argparse.ArgumentParser(description="Write a small synthetic photo library for profiling.")
    parser.add_argument("out", type=Path)
    parser.add_argument("--folders", type=int, default=80)
    parser.add_argument("--per-folder", type=int, default=50)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    args = parser.parse_args()
    jobs = plan(args.folders, args.per_folder, args.seed, args.out)
    for folder in {job[0].parent for job in jobs}:
        folder.mkdir(parents=True, exist_ok=True)
    with Pool(args.jobs) as pool:
        written = sum(1 for _ in pool.imap_unordered(write_file, jobs, chunksize=4))
    print(f"wrote {written} files in {args.folders} folders under {args.out}")


if __name__ == "__main__":
    main()
