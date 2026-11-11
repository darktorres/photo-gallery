"""A small synthetic photo library for profiling: the same kinds of files as a real one, far fewer than the pinterest set.

    python tools/profiling/synthetic_library.py OUT [--folders 80] [--per-folder 50] [--seed 1] [--jobs N]
    python tools/profiling/synthetic_library.py OUT --profile large     # 200,000 files, 50,000 of them broken

Writes OUT/folder-NN/ with --per-folder images each: mostly JPEGs (landscape, portrait and 12 MP, some with an EXIF
orientation and a capture date), plus PNGs (some with alpha), animated GIFs and WebPs. The pixels are smooth gradients
with a little noise, so the files compress like photographs rather than to nothing.

--broken FRACTION makes that share of the files broken JPEGs (`broken_jpeg`: a valid SOI and JFIF APP0, then bytes
that are no JPEG), which the import fails, each with the same error every time. The broken files are drawn from their
own generator, after the master draw, so the files that are not broken are the ones the same arguments write without
--broken (only the broken ones are replaced, each by a `.jpg` of the same stem).

A profile is a set of defaults that the options given override:
- `large`: the catalog benches' large dataset (`bench_data.py`, a layout named `...-large`): 400 folders of 500 files,
  a quarter of them broken, and small pictures (320x240 JPEGs, PNGs and WebPs, 160x120 GIF frames), so the library is
  a few GB, not hundreds. The scan, the import and the ledger see the file count, not the pixels.

Every file's kind, date and pixels come from its own seed, drawn in a fixed order from --seed before any file is
written, so the same arguments always write the same files, however many worker processes (--jobs, default: all
cores) write them.
"""

import argparse
import os
import random
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from multiprocessing import Pool
from pathlib import Path

import numpy as np
from PIL import Image

# (width, height, weight) of the JPEG sizes.
JPEG_SIZES = [(1920, 1080, 60), (1080, 1920, 20), (4032, 3024, 20)]
# (width, height) of the other kinds' pictures (a GIF's frames).
OTHER_SIZES = {"png": (800, 600), "gif": (320, 240), "webp": (1024, 768)}


@dataclass(frozen=True)
class Profile:
    folders: int = 80
    per_folder: int = 50
    broken: float = 0.0
    jpeg_size: str | None = None
    other_sizes: dict[str, tuple[int, int]] = field(default_factory=lambda: dict(OTHER_SIZES))


PROFILES = {
    "large": Profile(
        folders=400,
        per_folder=500,
        broken=0.25,
        jpeg_size="320x240",
        other_sizes={"png": (320, 240), "gif": (160, 120), "webp": (320, 240)},
    ),
}

# The JFIF APP0 segment after the SOI: version 1.1, no density units, 1:1, no thumbnail.
JFIF_HEADER = bytes.fromhex("FFD8FFE000104A46494600010100000100010000")

Job = tuple[Path, str, datetime, int, list[tuple[int, int, int]], dict[str, tuple[int, int]]]


def broken_jpeg(seed: int, length: int) -> bytes:
    """A JPEG's valid start (SOI, JFIF APP0) and then `length` bytes from `seed` with no 0xFF in them, so no marker
    follows the header: a decoder stops at the same place with the same error, every time."""
    rng = random.Random(seed)
    return JFIF_HEADER + bytes(rng.randrange(0xFF) for _ in range(length))


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


def write_file(job: Job) -> Path:
    path, kind, when, seed, sizes, other = job
    rng = random.Random(seed)
    if kind == "broken":
        path.write_bytes(broken_jpeg(seed, 1024 + rng.randrange(3072)))
    elif kind == "jpg":
        width, height, _ = rng.choices(sizes, weights=[w for *_, w in sizes])[0]
        img = picture(seed, width, height)
        orientation = rng.choice([6, 8]) if rng.random() < 0.1 else None
        img.save(path, "JPEG", quality=85, exif=exif_with(orientation, when))
    elif kind == "png":
        img = picture(seed, *other["png"], alpha=rng.random() < 0.3)
        img.save(path, "PNG", optimize=False)
    elif kind == "gif":
        frames = [picture(seed + k, *other["gif"]).convert("P") for k in range(3)]
        frames[0].save(path, "GIF", save_all=True, append_images=frames[1:], duration=120, loop=0)
    else:
        picture(seed, *other["webp"]).save(path, "WEBP", quality=80)
    return path


def plan(
    folders: int,
    per_folder: int,
    seed: int,
    out: Path,
    sizes: list[tuple[int, int, int]] = JPEG_SIZES,
    broken: float = 0.0,
    other: dict[str, tuple[int, int]] = OTHER_SIZES,
) -> list[Job]:
    """Every file's job, drawn in a fixed order from the master seed; then the broken ones, from their own generator."""
    master = random.Random(seed)
    start = datetime(2024, 1, 1)  # noqa: DTZ001 (EXIF DateTime is wall-clock time with no zone)
    jobs: list[Job] = []
    for f in range(folders):
        folder = out / f"folder-{f:02d}"
        for i in range(per_folder):
            when = start + timedelta(days=master.randrange(900), seconds=master.randrange(86400))
            kind = master.choices(["jpg", "png", "gif", "webp"], weights=[70, 15, 5, 5])[0]
            jobs.append((folder / f"IMG_{f:02d}_{i:03d}.{kind}", kind, when, master.randrange(2**32), sizes, other))
    count = round(broken * len(jobs))
    if count:
        for index in random.Random(seed * 1_000_003 + 7).sample(range(len(jobs)), count):
            path, _, when, file_seed, _, _ = jobs[index]
            jobs[index] = (path.with_suffix(".jpg"), "broken", when, file_seed, sizes, other)
    return jobs


def parse_sizes(text: str) -> list[tuple[int, int, int]]:
    """`WxH,WxH…`: those JPEG sizes, equally likely."""
    sizes = []
    for part in text.split(","):
        width, height = part.lower().split("x")
        sizes.append((int(width), int(height), 1))
    return sizes


def main() -> None:
    parser = argparse.ArgumentParser(description="Write a small synthetic photo library for profiling.")
    parser.add_argument("out", type=Path)
    parser.add_argument("--profile", choices=sorted(PROFILES), help="a set of defaults (see the module docstring)")
    parser.add_argument("--folders", type=int)
    parser.add_argument("--per-folder", type=int)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    parser.add_argument(
        "--jpeg-size",
        help="JPEG sizes as WxH[,WxH…], equally likely (default: the camera-like mix above, up to 12 MP)",
    )
    parser.add_argument("--broken", type=float, help="the share of the files written as broken JPEGs (default 0)")
    args = parser.parse_args()
    # The options given, else the profile's, else the defaults.
    profile = PROFILES[args.profile] if args.profile else Profile()
    folders = args.folders if args.folders is not None else profile.folders
    per_folder = args.per_folder if args.per_folder is not None else profile.per_folder
    broken = args.broken if args.broken is not None else profile.broken
    if not 0.0 <= broken <= 1.0:
        parser.error("--broken is a share, from 0 to 1")
    jpeg_size = args.jpeg_size or profile.jpeg_size
    sizes = parse_sizes(jpeg_size) if jpeg_size else JPEG_SIZES
    other = profile.other_sizes
    jobs = plan(folders, per_folder, args.seed, args.out, sizes, broken, other)
    for folder in {job[0].parent for job in jobs}:
        folder.mkdir(parents=True, exist_ok=True)
    with Pool(args.jobs) as pool:
        written = sum(1 for _ in pool.imap_unordered(write_file, jobs, chunksize=4))
    damaged = sum(1 for job in jobs if job[1] == "broken")
    print(f"wrote {written} files ({damaged} broken) in {folders} folders under {args.out}")


if __name__ == "__main__":
    main()
