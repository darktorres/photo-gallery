"""Verifier on synthetic edits: does the second stage accept an edited copy of the same picture?

Usage: python verify_synth.py [--model sscd_fp16] [--count 300] [--seed 0] [verify.py decision options]

Same sample and edits as synth.py (same --count, --seed and model => identical edited copies), so the
results line up with the retrieval test there. Each original is verified against each of its edited
copies; every pair should be a match. Writes verify_synth_<model>.csv / .json (label = the edit) and
one diagnostic per pair in verify_synth_<model>/<edit>_<i>.jpg.
"""

import argparse
import random
import time
from functools import partial
from types import SimpleNamespace

import cv2
import numpy as np

from common import DATA_ROOT, WORK_DIR, image_list, load_rgb
from edits import TRANSFORMS, edit_seed
from verify import Checkpoint, Features, PairRunner, add_decision_args, params_from, prepare_image, write_outputs


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", default="sscd_fp16", help="only selects the sample (its ok_<model>.npy mask)")
    parser.add_argument("--count", type=int, default=300)
    parser.add_argument("--seed", type=int, default=0)
    add_decision_args(parser)
    args = parser.parse_args()
    params = params_from(args)

    files = image_list()
    ok = np.load(WORK_DIR / f"ok_{args.model}.npy")
    # Exactly synth.py's sample and per-image edit seeds.
    sample = sorted(random.Random(args.seed).sample(list(np.flatnonzero(ok)), args.count))
    edits = list(TRANSFORMS)
    print(f"{len(sample)} images x {len(edits)} edits = {len(sample) * len(edits)} pairs", flush=True)

    features = Features(args.features)
    diag_dir = WORK_DIR / f"verify_synth_{args.model}"
    diag_dir.mkdir(exist_ok=True)
    total = len(sample) * len(edits)
    results: list = [None] * total
    checkpoint = Checkpoint("verify_synth", args.model)
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

    def edited_copy(im, t_index: int, i: int):
        return prepare_image(features, TRANSFORMS[edits[t_index]](im, random.Random(edit_seed(args.seed + t_index, i))))

    runner = PairRunner(features, params, args.workers, done)
    for i, f in enumerate(sample):
        finished = [checkpoint.done(f"{name}_{i}") for name in edits]
        todo = [t for t, r in enumerate(finished) if r is None]
        for t, r in enumerate(finished):
            if r is not None:
                results[i * len(edits) + t] = r
        if not todo:
            continue
        im = load_rgb(DATA_ROOT / files[f])
        original = prepare_image(features, im)
        for t_index in todo:
            name = edits[t_index]
            row = {"id": f"{name}_{i}", "label": name, "path_a": files[f], "path_b": f"{name}:{files[f]}",
                   "slot": i * len(edits) + t_index}
            runner.submit(row, lambda o=original: o, partial(edited_copy, im, t_index, i))
    runner.close()
    run = SimpleNamespace(pairs="synth", model=args.model, floor=None, features=args.features)
    write_outputs("verify_synth", run, params, tuple(edits), results, round(time.time() - start, 1))
    checkpoint.finish()


if __name__ == "__main__":
    main()
