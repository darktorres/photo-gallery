"""Shared paths, the canonical image list, and ground truth for the M0 model bake-off.

Row i of every embedding file corresponds to line i of files.txt, so all models are
compared on exactly the same images in the same order.
"""

import os
from collections import defaultdict
from dataclasses import dataclass
from itertools import combinations
from pathlib import Path

from PIL import Image, ImageOps

DATA_ROOT = Path(os.environ.get("PG_DATA_ROOT", r"D:\PhotoGallery\datasets\pinterest"))
WORK_DIR = Path(os.environ.get("PG_WORK_DIR", r"D:\PhotoGallery\eval"))
CACHE_DIR = Path(os.environ.get("PG_CACHE_DIR", r"D:\PhotoGallery\cache"))

# Keep model downloads off C: (nearly full). Must run before timm/open_clip are imported.
os.environ.setdefault("HF_HOME", str(CACHE_DIR / "hf"))
os.environ.setdefault("TORCH_HOME", str(CACHE_DIR / "torch"))

IMAGE_EXTS = {".jpg", ".jpeg", ".png", ".webp", ".bmp", ".gif"}

# Local, trusted files; some Pinterest originals are huge.
Image.MAX_IMAGE_PIXELS = None


def load_rgb(path: Path) -> Image.Image:
    """Decode, apply EXIF orientation, and flatten transparency onto white."""
    with Image.open(path) as im:
        im = ImageOps.exif_transpose(im)
        if im.mode in ("RGBA", "LA") or (im.mode == "P" and "transparency" in im.info):
            rgba = im.convert("RGBA")
            flat = Image.new("RGB", rgba.size, (255, 255, 255))
            flat.paste(rgba, mask=rgba.getchannel("A"))
            return flat
        return im.convert("RGB")

# dups2/<NNN>/<files>: matched duplicate groups.
# dups2/<file>: each file has a duplicate among the other loose files; not yet paired.
GROUPS_DIR = "dups2"
# Images that duplicate tools flagged as duplicates, but which aren't.
NEGATIVES_DIR = "similiar_but_not_dups"


def image_list() -> list[str]:
    """Relative POSIX paths of all images under DATA_ROOT, in a stable order (cached)."""
    cache = WORK_DIR / "files.txt"
    if cache.exists():
        return cache.read_text(encoding="utf-8").splitlines()
    files = sorted(
        p.relative_to(DATA_ROOT).as_posix()
        for p in DATA_ROOT.rglob("*")
        if p.is_file() and p.suffix.lower() in IMAGE_EXTS
    )
    WORK_DIR.mkdir(parents=True, exist_ok=True)
    cache.write_text("\n".join(files), encoding="utf-8")
    return files


@dataclass
class GroundTruth:
    """Row indices into the file list."""

    groups: list[list[int]]
    # Every pair inside a matched group.
    positives: list[tuple[int, int]]
    # Every pair inside the negatives folder is a non-duplicate.
    negative_pool: list[int]
    # Loose dups2 files: each has a duplicate somewhere else in this pool.
    unpaired_pool: list[int]


def ground_truth(files: list[str]) -> GroundTruth:
    by_group: dict[str, list[int]] = defaultdict(list)
    negative_pool: list[int] = []
    unpaired_pool: list[int] = []
    for i, f in enumerate(files):
        parts = f.split("/")
        if len(parts) == 3 and parts[0] == GROUPS_DIR:
            by_group[parts[1]].append(i)
        elif len(parts) == 2 and parts[0] == GROUPS_DIR:
            unpaired_pool.append(i)
        elif len(parts) == 2 and parts[0] == NEGATIVES_DIR:
            negative_pool.append(i)
    groups = [sorted(g) for g in by_group.values() if len(g) >= 2]
    positives = [pair for g in groups for pair in combinations(g, 2)]
    return GroundTruth(groups, positives, negative_pool, unpaired_pool)
