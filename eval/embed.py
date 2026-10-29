"""Embed every image in the dataset with one candidate model.

Usage: python embed.py --model sscd
Writes WORK_DIR/emb_<model>.npy (float16, row i = files.txt line i) and ok_<model>.npy.
"""

import argparse
import time

import numpy as np
import torch

from common import DATA_ROOT, WORK_DIR, image_list
from imgload import FileSource, ImageDataset, embed, first_failures
from models import SPECS, build_transform, load_model


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True, choices=sorted(SPECS))
    parser.add_argument("--batch", type=int, default=64)
    parser.add_argument("--workers", type=int, default=10)
    args = parser.parse_args()

    device = torch.device("cuda")
    files = image_list()
    spec = SPECS[args.model]
    model = load_model(spec, device)

    dataset = ImageDataset(FileSource(DATA_ROOT, files), build_transform(spec), spec.size)
    start = time.perf_counter()
    emb, ok = embed(model, spec, dataset, args.batch, args.workers, device, args.model)
    elapsed = time.perf_counter() - start

    np.save(WORK_DIR / f"emb_{args.model}.npy", emb)
    np.save(WORK_DIR / f"ok_{args.model}.npy", ok)
    print(f"{args.model}: {len(files)} images, dim {emb.shape[1]}, {elapsed:.0f} s "
          f"({len(files) / elapsed:.0f} img/s), {int((~ok).sum())} failed")
    for path in first_failures(files, ok):
        print(f"  failed: {path}")


if __name__ == "__main__":
    main()
