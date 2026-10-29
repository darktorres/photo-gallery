"""Synthetic robustness: how far can an image be edited before the model stops finding it?

Usage: python synth.py --model sscd [--count 1000]

For a fixed random sample of images, apply each transform, embed the edited copy, and
check whether the untouched original comes back from the full 190k index. R@1 slightly
understates retrieval when the original has a real duplicate elsewhere in the library
(that copy can win rank 1), so R@10 is reported too.
"""

import argparse
import json
import random
from pathlib import Path
from typing import Sequence

import numpy as np
import torch
from PIL import Image

from common import DATA_ROOT, WORK_DIR, image_list, load_rgb
from edits import TRANSFORMS, edit_seed
from imgload import ImageDataset, embed
from models import SPECS, build_transform, load_model


class TransformSource:
    """Picklable source: loads sample image i and applies one transform deterministically."""

    def __init__(self, root: Path, paths: Sequence[str], transform: str, seed: int):
        self.root = root
        self.paths = paths
        self.transform = transform
        self.seed = seed  # run seed + transform index

    def __len__(self) -> int:
        return len(self.paths)

    def __call__(self, i: int) -> Image.Image:
        im = load_rgb(self.root / self.paths[i])
        return TRANSFORMS[self.transform](im, random.Random(edit_seed(self.seed, i)))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True, choices=sorted(SPECS))
    parser.add_argument("--count", type=int, default=1000)
    parser.add_argument("--batch", type=int, default=64)
    parser.add_argument("--workers", type=int, default=10)
    parser.add_argument("--seed", type=int, default=0)
    args = parser.parse_args()

    device = torch.device("cuda")
    files = image_list()
    ok = np.load(WORK_DIR / f"ok_{args.model}.npy")
    db = torch.from_numpy(np.load(WORK_DIR / f"emb_{args.model}.npy")).to(device)

    # Same sample for every model, so results are comparable.
    sample = sorted(random.Random(args.seed).sample(list(np.flatnonzero(ok)), args.count))
    sample_paths = [files[i] for i in sample]
    originals = torch.tensor(sample, device=device)

    thresholds = {}
    results_path = WORK_DIR / f"results_{args.model}.json"
    if results_path.exists():
        for p in json.loads(results_path.read_text(encoding="utf-8"))["operating_points"]:
            thresholds[p["target_recall"]] = p["threshold"]

    spec = SPECS[args.model]
    model = load_model(spec, device)
    transform = build_transform(spec)

    rows = []
    for t_index, name in enumerate(TRANSFORMS):
        dataset = ImageDataset(TransformSource(DATA_ROOT, sample_paths, name, args.seed + t_index), transform, spec.size)
        q, q_ok = embed(model, spec, dataset, args.batch, args.workers, device, f"{args.model}/{name}")
        q = torch.from_numpy(q).to(device)
        keep = torch.from_numpy(q_ok).to(device)
        cos = (q.float() * db[originals].float()).sum(1)[keep].cpu().numpy()
        top = (q @ db.T).topk(10, dim=1).indices[keep]
        hits = top == originals[keep].unsqueeze(1)
        rows.append({
            "transform": name,
            "r1": hits[:, 0].float().mean().item(),
            "r10": hits.any(1).float().mean().item(),
            "cos_p10": float(np.quantile(cos, 0.10)),
            "cos_p50": float(np.quantile(cos, 0.50)),
            **{f"above_t{int(r * 100)}": float((cos >= t).mean()) for r, t in thresholds.items() if r in (0.90, 0.95)},
        })

    (WORK_DIR / f"synth_{args.model}.json").write_text(json.dumps(rows, indent=2), encoding="utf-8")
    threshold_cols = [k for k in rows[0] if k.startswith("above_t")]
    print(f"## {args.model}: synthetic robustness, {args.count} images, retrieved from {len(files):,}")
    header = ["transform", "R@1", "R@10", "cos p10", "cos p50"] + [
        f"≥ thr@{c[7:]}%recall" for c in threshold_cols]
    print("| " + " | ".join(header) + " |")
    print("|" + "---|" * len(header))
    for r in rows:
        cells = [r["transform"], f"{r['r1']:.3f}", f"{r['r10']:.3f}", f"{r['cos_p10']:.3f}", f"{r['cos_p50']:.3f}"]
        cells += [f"{r[c]:.3f}" for c in threshold_cols]
        print("| " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
