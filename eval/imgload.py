"""Batched embedding shared by embed.py, synth.py and check_groups.py."""

import sys
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


def trim_borders(im: Image.Image) -> Image.Image:
    """`im` without solid or blurred padding borders: tools/borders.py's detector at its default settings, as the
    ingest pipeline would run it before embedding."""
    tools = str(Path(__file__).resolve().parents[1] / "tools")
    if tools not in sys.path:
        sys.path.append(tools)
    import borders

    box, accepted, _ = borders.detect(np.asarray(im), borders.Params())
    return im.crop(box) if accepted else im


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
        except Exception:
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
