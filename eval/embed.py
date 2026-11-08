"""Embed every image in the dataset with one candidate model.

Usage: python embed.py --model sscd [--trim] [--draft]
Writes WORK_DIR/emb_<model>.npy (float16, row i = files.txt line i) and ok_<model>.npy. With --trim, each image's
borders are cut first (the gallery's detector) and the files are named <model>_trim. With --draft, JPEGs decode
DCT-scaled by the Rust import's rule (long side >= 512, short side >= 320) and the names gain _draft.
"""

import argparse
import time

import numpy as np
import torch

from common import DATA_ROOT, WORK_DIR, image_list
from imgload import FileSource, ImageDataset, TrimmedSource, embed, first_failures
from models import SPECS, build_transform, load_model

# The Rust import's decode rule (crates/core/src/analyze.rs): long side >= 512, short side >= 320.
DRAFT_SIDES = (512, 320)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True, choices=sorted(SPECS))
    parser.add_argument("--batch", type=int, default=64)
    parser.add_argument("--workers", type=int, default=10)
    parser.add_argument("--trim", action="store_true", help="cut borders before embedding")
    parser.add_argument("--draft", action="store_true", help="DCT-scaled JPEG decode, as the Rust import does")
    args = parser.parse_args()

    device = torch.device("cuda")
    files = image_list()
    spec = SPECS[args.model]
    model = load_model(spec, device)

    source = FileSource(DATA_ROOT, files, draft_sides=DRAFT_SIDES if args.draft else None)
    dataset = ImageDataset(TrimmedSource(source) if args.trim else source, build_transform(spec), spec.size)
    name = args.model + ("_trim" if args.trim else "") + ("_draft" if args.draft else "")
    start = time.perf_counter()
    emb, ok = embed(model, spec, dataset, args.batch, args.workers, device, args.model)
    elapsed = time.perf_counter() - start

    np.save(WORK_DIR / f"emb_{name}.npy", emb)
    np.save(WORK_DIR / f"ok_{name}.npy", ok)
    print(
        f"{name}: {len(files)} images, dim {emb.shape[1]}, {elapsed:.0f} s "
        f"({len(files) / elapsed:.0f} img/s), {int((~ok).sum())} failed"
    )
    for path in first_failures(files, ok):
        print(f"  failed: {path}")


if __name__ == "__main__":
    main()
