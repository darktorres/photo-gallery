"""Audit the hand-made duplicate groups (dups2/<NNN>/) for mislabeled folders.

Usage: python check_groups.py [--model sscd_fp16] [--link 0.35]

Embeds the current contents of every group folder (rescanned, so it reflects edits
made since files.txt was cached) and splits each folder into connected parts, linking
images whose similarity is >= --link. A folder that splits into 2+ parts contains
pictures unrelated to each other. The default link sits above the hard negatives'
99th percentile (~0.30), so separate parts are very unlikely to be the same picture.
"""

import argparse

import numpy as np
import torch

from common import DATA_ROOT, GROUPS_DIR, IMAGE_EXTS
from imgload import FileSource, ImageDataset, embed
from models import SPECS, build_transform, load_model


def components(sims: np.ndarray, link: float) -> list[list[int]]:
    parent = list(range(len(sims)))

    def find(i: int) -> int:
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i

    for i in range(len(sims)):
        for j in range(i + 1, len(sims)):
            if sims[i, j] >= link:
                parent[find(i)] = find(j)
    parts: dict[int, list[int]] = {}
    for i in range(len(sims)):
        parts.setdefault(find(i), []).append(i)
    return sorted(parts.values(), key=lambda p: p[0])


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", default="sscd_fp16", choices=sorted(SPECS))
    parser.add_argument("--link", type=float, default=0.35)
    args = parser.parse_args()

    root = DATA_ROOT / GROUPS_DIR
    folders = sorted(p for p in root.iterdir() if p.is_dir())
    members = {f.name: sorted(p.name for p in f.iterdir() if p.is_file() and p.suffix.lower() in IMAGE_EXTS) for f in folders}
    files = [f"{GROUPS_DIR}/{g}/{name}" for g, names in members.items() for name in names]

    device = torch.device("cuda")
    spec = SPECS[args.model]
    dataset = ImageDataset(FileSource(DATA_ROOT, files), build_transform(spec), spec.size)
    emb, ok = embed(load_model(spec, device), spec, dataset, 64, 4, device, args.model)
    emb = emb.astype(np.float32)

    mixed, singles, weak = [], [], []
    row = 0
    for group, names in members.items():
        rows = list(range(row, row + len(names)))
        row += len(names)
        if len(names) < 2:
            singles.append(group)
            continue
        sims = emb[rows] @ emb[rows].T
        parts = components(sims, args.link)
        if len(parts) > 1:
            cross = max(sims[a, b] for p in parts for q in parts if p is not q for a in p for b in q)
            mixed.append((group, names, parts, cross, [names[i] for i, r in enumerate(rows) if not ok[r]]))
        else:
            # Weakest similarity that still holds the group together (max spanning tree's min edge
            # is complex; the min over each member's best partner is a good-enough signal).
            np.fill_diagonal(sims, -2.0)
            weak.append((float(sims.max(1).min()), group))

    print(f"{len(folders)} group folders, {len(files)} files, linked at similarity >= {args.link}")
    print(f"\n## Folders with unrelated pictures: {len(mixed)}")
    for group, names, parts, cross, failed in mixed:
        print(f"\n{GROUPS_DIR}/{group}  ({len(parts)} separate parts; highest similarity between parts {cross:.2f})")
        for k, part in enumerate(parts, 1):
            for i in part:
                print(f"   part {k}: {names[i]}")
        for name in failed:
            print(f"   (failed to load: {name})")
    print(f"\n## Folders with a single file (partner missing): {len(singles)}")
    for group in singles:
        print(f"   {GROUPS_DIR}/{group}: {members[group][0]}")
    print("\n## Weakest consistent groups (lowest best-partner similarity), worth a glance:")
    for score, group in sorted(weak)[:10]:
        print(f"   {GROUPS_DIR}/{group}: {score:.2f}")


if __name__ == "__main__":
    main()
