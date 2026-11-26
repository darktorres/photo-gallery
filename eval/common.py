"""Shared paths, the canonical image list, and ground truth for the M0 model bake-off.

Row i of every embedding file corresponds to line i of files.txt, so all models are
compared on exactly the same images in the same order.
"""

import os
from collections import defaultdict
from collections.abc import Callable
from dataclasses import dataclass
from itertools import combinations
from pathlib import Path
from typing import BinaryIO

import numpy as np
from PIL import Image, ImageOps

REPO_ROOT = Path(__file__).resolve().parent.parent


def _load_dotenv() -> None:
    """Load <repo>/.env (KEY=VALUE lines, # comments) into os.environ. Variables already set win."""
    env_file = REPO_ROOT / ".env"
    if not env_file.is_file():
        return
    for raw_line in env_file.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        os.environ.setdefault(key.strip(), value.strip())


_load_dotenv()


def pg_path(var: str | None, *parts: str) -> Path:
    """The value of the environment variable `var` if it is set, else PG_ROOT joined with `parts`.

    Fails with a message that names both variables when neither is set. There is no drive-letter default."""
    if var and os.environ.get(var):
        return Path(os.environ[var])
    root = os.environ.get("PG_ROOT")
    if root:
        return Path(root, *parts)
    names = f"{var} or PG_ROOT" if var else "PG_ROOT"
    raise SystemExit(f"{names} must be set (put PG_ROOT=<base folder> in {REPO_ROOT / '.env'}, or in the environment)")


def exe_name(name: str) -> str:
    """A program's file name on this OS: `name.exe` on Windows, `name` elsewhere."""
    return name + (".exe" if os.name == "nt" else "")


# A venv's interpreter, as (folder, file) under the venv: Scripts\python.exe on Windows, bin/python elsewhere.
VENV_PYTHON = ("Scripts" if os.name == "nt" else "bin", exe_name("python"))

DATA_ROOT = pg_path("PG_DATA_ROOT", "datasets")
WORK_DIR = pg_path("PG_WORK_DIR", "eval")
CACHE_DIR = pg_path("PG_CACHE_DIR", "cache")

# Keep model downloads off C: (nearly full). Must run before timm/open_clip are imported.
os.environ.setdefault("HF_HOME", str(CACHE_DIR / "hf"))
os.environ.setdefault("TORCH_HOME", str(CACHE_DIR / "torch"))

IMAGE_EXTS = {".jpg", ".jpeg", ".png", ".webp", ".bmp", ".gif"}

# The verifier's feature extraction resolution (long side); the DISK engine's largest input is the same size.
WORK_SIDE = 1024
# The library the projections are sized for: 70 million images.
LIBRARY_SIZE = 70e6
# The similarity of a row with nothing (itself, a padded slot): below every cosine (at least -1), so a top-k never picks it.
NO_SIM = float("-inf")
# The decode rule of the Rust import (crates/model/src/analysis.rs, DECODE_MIN_LONG and DECODE_MIN_SHORT): the long side
# at least 512 and the short side at least 320, after orientation. Python cannot import the Rust constant, so this copy
# is by hand.
ANALYSIS_DRAFT_SIDES = (512, 320)

# The model the eval scripts use when none is named: the PyTorch SSCD baseline (models.py's SPECS).
DEFAULT_MODEL = "sscd_fp16"


def embedding_name(model: str, trim: bool = False, draft: bool = False) -> str:
    """The stem of a model's embedding files (emb_<stem>.npy, ok_<stem>.npy): the model, then _trim when its borders were
    cut first and _draft when its JPEGs were decoded at the import's draft scale. embed.py writes these names, and every
    reader builds them here too."""
    return model + ("_trim" if trim else "") + ("_draft" if draft else "")


def analysis_denom(w: int, h: int) -> int:
    """The DCT scale denominator (1, 2, 4 or 8) the analysis rule picks for a w x h JPEG: the smallest that keeps
    ANALYSIS_DRAFT_SIDES."""
    mw, mh = ANALYSIS_DRAFT_SIDES if w >= h else ANALYSIS_DRAFT_SIDES[::-1]
    return next((d for d in (8, 4, 2) if -(-w // d) >= mw and -(-h // d) >= mh), 1)


# Local, trusted files; some Pinterest originals are huge.
Image.MAX_IMAGE_PIXELS = None


def quarter_turn(im: Image.Image) -> bool:
    """True when the EXIF orientation turns the image a quarter turn, so its width and height swap (orientations 5-8)."""
    return im.getexif().get(0x0112, 1) in (5, 6, 7, 8)


def load_rgb(
    path: Path | BinaryIO, draft: tuple[int, int] | None = None, draft_sides: tuple[int, int] | None = None
) -> Image.Image:
    """Decode, apply EXIF orientation, and flatten transparency onto white.

    draft=(w, h): let a JPEG decode at 1/2, 1/4 or 1/8 scale in the DCT (libjpeg-turbo) while staying at least
    w x h after orientation; other formats decode in full.
    draft_sides=(long, short): the same, keeping the long side >= long and the short side >= short; this is the
    Rust import's decode rule (ANALYSIS_DRAFT_SIDES, from crates/model/src/analysis.rs)."""
    with Image.open(path) as im:
        if draft_sides and im.format == "JPEG":
            long, short = draft_sides  # orientation doesn't change which side is the long one
            im.draft("RGB" if im.mode == "RGB" else None, (long, short) if im.width >= im.height else (short, long))
        elif draft and im.format == "JPEG":
            im.draft("RGB" if im.mode == "RGB" else None, (draft[1], draft[0]) if quarter_turn(im) else draft)
        oriented = ImageOps.exif_transpose(im)
        if oriented.mode in ("RGBA", "LA") or (oriented.mode == "P" and "transparency" in oriented.info):
            rgba = oriented.convert("RGBA")
            flat = Image.new("RGB", rgba.size, (255, 255, 255))
            flat.paste(rgba, mask=rgba.getchannel("A"))
            return flat
        return oriented.convert("RGB")


# dups2/<NNN>/<files>: matched duplicate groups.
# dups2/<file>: each file has a duplicate among the other loose files; not yet paired.
GROUPS_DIR = "dups2"
# Images that duplicate tools flagged as duplicates, but which aren't.
NEGATIVES_DIR = "similiar_but_not_dups"


def vips_thumbnail(data: bytes, width: int, height: int) -> np.ndarray:
    """`data` fitted to width x height by libvips, as the 3-band sRGB array the other decoders give (the bench's `vips`
    pipeline: bench_decode.py and bench_thumbs.py both use this one)."""
    import pyvips  # the bench's only user of it: imported where it is used

    im = pyvips.Image.thumbnail_buffer(data, width, height=height, size="force")
    if im.interpretation not in ("srgb", "rgb", "b-w") or im.bands not in (1, 3):
        im = im.colourspace("srgb")
    if im.hasalpha():
        im = im.flatten(background=[255, 255, 255])
    if im.bands == 1:
        im = im.bandjoin([im, im])
    return np.ndarray(buffer=im.write_to_memory(), dtype=np.uint8, shape=(im.height, im.width, 3))


def partial_path(path: Path) -> Path:
    """The temporary name a file is written under before it is renamed into place."""
    return path.with_name(path.name + ".partial")


def save_atomic(path: Path, save: Callable[[BinaryIO], None]) -> None:
    """Writes `path` through a temp file and a rename, so an interrupted write never leaves a file that a rerun
    takes as finished."""
    tmp = partial_path(path)
    with tmp.open("wb") as f:
        save(f)
    tmp.replace(path)


def image_list() -> list[str]:
    """Relative POSIX paths of all images under DATA_ROOT, in a stable order (cached)."""
    cache = WORK_DIR / "files.txt"
    if cache.exists():
        return cache.read_text(encoding="utf-8").splitlines()
    files = sorted(
        p.relative_to(DATA_ROOT).as_posix() for p in DATA_ROOT.rglob("*") if p.is_file() and p.suffix.lower() in IMAGE_EXTS
    )
    WORK_DIR.mkdir(parents=True, exist_ok=True)
    save_atomic(cache, lambda f: f.write("\n".join(files).encode("utf-8")))
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
