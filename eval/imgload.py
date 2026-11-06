"""Batched embedding shared by embed.py, synth.py and check_groups.py."""

import os
import struct
import subprocess
from collections.abc import Sequence
from pathlib import Path
from typing import Protocol

import numpy as np
import torch
from PIL import Image
from torch.utils.data import DataLoader, Dataset
from tqdm import tqdm

from common import load_rgb
from models import ModelSpec, run


class ImageSource(Protocol):
    """Produces the PIL image for item i. Must be picklable: DataLoader workers on
    Windows are spawned processes, so implementations are module-level classes."""

    def __len__(self) -> int: ...

    def __call__(self, i: int) -> Image.Image: ...


class FileSource:
    def __init__(self, root: Path, files: Sequence[str]):
        self.root = root
        self.files = files

    def __len__(self) -> int:
        return len(self.files)

    def __call__(self, i: int) -> Image.Image:
        return load_rgb(self.root / self.files[i])


# The gallery's border detector (crates/analysis/src/borders.rs), run on piped pixels by `gallery borders --stdin`.
GALLERY = Path(os.environ.get("PG_GALLERY", Path(__file__).resolve().parents[1] / "target" / "release" / "gallery.exe"))


class _Detector:
    """Its process in this worker: started on first use, and again if it ended."""

    process: subprocess.Popen | None = None


def content_box(im: Image.Image) -> tuple[int, int, int, int]:
    """What is left of `im` without its solid or blurred padding borders, as (left, top, right, bottom): the
    gallery's detector, one process per worker, kept running."""
    if _Detector.process is None or _Detector.process.poll() is not None:
        args = [str(GALLERY), "borders", "--stdin"]
        _Detector.process = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    process = _Detector.process
    assert process.stdin is not None and process.stdout is not None
    rgb = im.convert("RGB")
    process.stdin.write(struct.pack("<II", *rgb.size) + rgb.tobytes())
    process.stdin.flush()
    line = process.stdout.readline()
    if not line:
        raise RuntimeError(f"{GALLERY} borders --stdin ended (build it: cargo build --release)")
    left, top, right, bottom = map(int, line.split())
    return left, top, right, bottom


def trim_borders(im: Image.Image) -> Image.Image:
    """`im` without solid or blurred padding borders, as the gallery finds them before embedding."""
    box = content_box(im)
    return im if box == (0, 0, *im.size) else im.crop(box)


class TrimmedSource:
    """Another source's images with their borders trimmed (picklable, like the sources it wraps)."""

    def __init__(self, source: ImageSource):
        self.source = source

    def __len__(self) -> int:
        return len(self.source)

    def __call__(self, i: int) -> Image.Image:
        return trim_borders(self.source(i))


class ImageDataset(Dataset):
    """Yields (tensor, ok); an item that fails to load yields zeros and ok=False."""

    def __init__(self, source: ImageSource, transform, size: int):
        self.source = source
        self.transform = transform
        self.size = size

    def __len__(self) -> int:
        return len(self.source)

    def __getitem__(self, i: int):
        try:
            return self.transform(self.source(i)), True
        except Exception:  # noqa: BLE001 (a bad file yields a blank tensor, flagged False, not a crash)
            return torch.zeros(3, self.size, self.size), False


def embed(
    model: torch.nn.Module, spec: ModelSpec, dataset: ImageDataset, batch_size: int, workers: int, device: torch.device, desc: str
) -> tuple[np.ndarray, np.ndarray]:
    """Returns (L2-normalized float16 embeddings, ok mask). Failed items get zero vectors."""
    loader = DataLoader(dataset, batch_size=batch_size, num_workers=workers, pin_memory=True)
    out: np.ndarray | None = None
    ok = np.zeros(len(dataset), dtype=bool)
    pos = 0
    with torch.inference_mode():
        for x, ok_batch in tqdm(loader, desc=desc, unit="batch"):
            y = run(model, spec, x.to(device, non_blocking=True))
            y = torch.nn.functional.normalize(y, dim=1)
            y[~ok_batch.to(device)] = 0
            if out is None:
                out = np.zeros((len(dataset), y.shape[1]), dtype=np.float16)
            out[pos : pos + len(y)] = y.cpu().numpy()
            ok[pos : pos + len(y)] = ok_batch.numpy()
            pos += len(y)
    assert out is not None
    return out, ok


def first_failures(files: Sequence[str], ok: np.ndarray, limit: int = 20) -> list[str]:
    return [files[int(i)] for i in np.flatnonzero(~ok)[:limit]]
