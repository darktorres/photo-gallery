"""Verifier on synthetic edits: does the second stage accept an edited copy of the same picture?

Usage: python verify_synth.py [--model sscd_fp16] [--count 300] [--seed 0] [--edits rot_45 ...] [--prefix verify_synth]
                              [verify.py decision options]

Same sample and edits as synth.py (same --count, --seed and model => identical edited copies), so the
results line up with the retrieval test there. Each original is verified against each of its edited
copies; every pair should be a match. Writes verify_synth_<model>.csv / .json (label = the edit) and
one diagnostic per pair in verify_synth_<model>/<edit>_<i>.jpg.
"""

import argparse
import random
import time
from functools import lru_cache, partial
from types import SimpleNamespace

import cv2
import numpy as np

from common import DATA_ROOT, WORK_DIR, image_list, load_rgb
from edits import TRANSFORMS, edit_seed
from verify import (
    ROTATIONS,
    Checkpoint,
    Features,
    PairRunner,
    Prefetcher,
    Prepared,
    add_decision_args,
    params_from,
    prepared,
    work_image,
    write_outputs,
)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", default="sscd_fp16", help="only selects the sample (its ok_<model>.npy mask)")
    parser.add_argument("--count", type=int, default=300)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--edits", nargs="+", choices=list(TRANSFORMS), help="only these edits (default: all)")
    parser.add_argument("--prefix", default="verify_synth", help="output name (keeps runs of edit subsets apart)")
    add_decision_args(parser)
    args = parser.parse_args()
    params = params_from(args)

    files = image_list()
    ok = np.load(WORK_DIR / f"ok_{args.model}.npy")
    # Exactly synth.py's sample and per-image edit seeds.
    sample = sorted(random.Random(args.seed).sample(list(np.flatnonzero(ok)), args.count))
    edits = args.edits or list(TRANSFORMS)
    seed_of = {name: args.seed + k for k, name in enumerate(TRANSFORMS)}  # an edit's seed doesn't depend on --edits
    print(f"{len(sample)} images x {len(edits)} edits = {len(sample) * len(edits)} pairs", flush=True)

    features = Features(args.features, args.disk_runtime, ROTATIONS[args.rotations])
    diag_dir = WORK_DIR / f"{args.prefix}_{args.model}"
    diag_dir.mkdir(exist_ok=True)
    total = len(sample) * len(edits)
    results: list = [None] * total
    checkpoint = Checkpoint(args.prefix, args.model)
    start, verified = time.time(), 0

    def done(row: dict, out: dict, diag) -> None:
        nonlocal verified
        row.update(out)
        row["correct"] = row["decision"] == "match"
        if diag is not None:
            cv2.imwrite(str(diag_dir / f"{row['id']}.jpg"), diag, [cv2.IMWRITE_JPEG_QUALITY, 85])
        results[row.pop("slot")] = row
        checkpoint.add(row)
        verified += 1
        if verified % 260 == 0:
            print(f"  {verified}/{total}  {verified / (time.time() - start):.1f} pairs/s", flush=True)

    todo: dict[int, list[int]] = {}  # sample position -> the edits (indices into `edits`) still to verify
    for i in range(len(sample)):
        for t, name in enumerate(edits):
            if (r := checkpoint.done(f"{name}_{i}")) is None:
                todo.setdefault(i, []).append(t)
            else:
                results[i * len(edits) + t] = r

    def load(i: int) -> tuple:
        """On the prefetcher's threads: the original and its edited copies, at working resolution."""
        im = load_rgb(DATA_ROOT / files[sample[i]])
        copies = {t: work_image(TRANSFORMS[edits[t]](im, random.Random(edit_seed(seed_of[edits[t]], i)))) for t in todo[i]}
        return work_image(im), copies

    prefetch = Prefetcher(load, todo, args.loaders, ahead=8)

    # Everything that can fail runs inside PairRunner.submit's try (its getters), so one bad file is that pair's error.
    # Prefetcher.get pops its key, so each image is loaded once and kept here while its pairs are submitted.
    @lru_cache(maxsize=8)
    def loaded(i: int) -> tuple:
        return prefetch.get(i)

    @lru_cache(maxsize=8)
    def original(i: int) -> Prepared:
        return prepared(features, loaded(i)[0])

    def edited(i: int, t_index: int) -> Prepared:
        return prepared(features, loaded(i)[1][t_index])

    runner = PairRunner(features, params, args.workers, done)
    for i, ts in todo.items():
        rel = files[sample[i]]
        for t_index in ts:
            name = edits[t_index]
            row = {"id": f"{name}_{i}", "label": name, "path_a": rel, "path_b": f"{name}:{rel}", "slot": i * len(edits) + t_index}
            runner.submit(row, partial(original, i), partial(edited, i, t_index))
    runner.close()
    prefetch.close()
    run = SimpleNamespace(pairs="synth", model=args.model, floor=None, features=args.features)
    write_outputs(args.prefix, run, params, tuple(edits), results, round(time.time() - start, 1))
    checkpoint.finish()


if __name__ == "__main__":
    main()
