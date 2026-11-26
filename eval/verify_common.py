"""The torch-free parts of the verifier, shared by verify.py and the tools that run it (verify_synth.py): the image preparation
the loaders run, the thresholds' command-line options, the parameters they make, and the summary a run writes. Importing
this module loads no torch, so a tool that needs only these helpers does not pay for it."""

import argparse
import csv
import json
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass

import numpy as np
from PIL import Image

from common import WORK_DIR, WORK_SIDE
from verify_types import COLUMNS, ROTATIONS, csv_row

MIN_WORK_SIDE = 512  # ... smaller ones upscaled to this (25%-downscaled copies: 0/40 aligned as is, 40/40 at 512)

FEATURES = ("disk", "aliked", "sift")  # local features (each with its own LightGlue weights)

DISK_RUNTIMES = ("trt", "torch")

# The right decision per label; labels without one (similar_folder) aren't scored. Planted copies (plant.py) are
# edits of their source, so they must match. verify.py scores with it, and so does the viewer's verifier tables.
EXPECTED = {"duplicate": "match", "cross_group": "unrelated", "planted": "match"}


@dataclass
class Params:
    min_inliers: int  # fewer: don't even try to align
    abs_threshold: float  # residual (RGB distance, 0..sqrt(3)) that always counts as changed
    noise_k: float  # ... or this many robust sigmas above the pair's own noise level
    max_noise: float  # match if the median residual after alignment is at most this
    strong_inliers: int  # ... or if at least this many inliers
    strong_ratio: float  # ... make up at least this fraction of all matches
    max_moved: float = 0.0  # unrelated if more than this share of the overlap moved (0 = off)
    max_interior_change: float = 0.0  # unrelated if more than this share of the interior changed (0 = off)


def work_image(im) -> tuple[np.ndarray, float]:
    """`im`: an upright RGB PIL image -> (uint8 RGB array at the working resolution, its scale). CPU only."""
    side = max(im.size)
    scale = min(WORK_SIDE, max(MIN_WORK_SIDE, side)) / side
    if scale != 1.0:
        im = im.resize((max(1, round(im.width * scale)), max(1, round(im.height * scale))), resample=Image.Resampling.LANCZOS)
    return np.asarray(im), scale


class Prefetcher:
    """Runs `load` (the CPU part of preparing images: decode, edit, resize) on its own threads for `keys`, in the
    order they will be asked for and up to `ahead` of them in advance, so the main thread only feeds the GPU.
    A key that isn't pending (asked for again after the caller's cache dropped it) loads in the caller."""

    def __init__(self, load, keys, threads: int, ahead: int = 32) -> None:
        self.load, self.keys, self.ahead = load, iter(keys), ahead
        self.pool = ThreadPoolExecutor(max(1, threads), thread_name_prefix="load")
        self.pending: dict = {}
        self._fill()

    def _fill(self) -> None:
        while len(self.pending) < self.ahead and (key := next(self.keys, None)) is not None:
            self.pending.setdefault(key, self.pool.submit(self.load, key))

    def get(self, key):
        fut = self.pending.pop(key, None)
        self._fill()
        return fut.result() if fut is not None else self.load(key)

    def close(self) -> None:
        self.pool.shutdown(cancel_futures=True)


def add_decision_args(parser: argparse.ArgumentParser) -> None:
    """The verifier's thresholds as command-line options, and --workers (shared with verify_synth.py)."""
    parser.add_argument("--workers", type=int, default=8, help="CPU threads for the dense comparison")
    parser.add_argument("--loaders", type=int, default=4, help="CPU threads decoding and resizing images ahead")
    parser.add_argument("--features", choices=FEATURES, default="disk", help="local features for the alignment")
    parser.add_argument(
        "--disk-runtime",
        choices=DISK_RUNTIMES,
        default="trt",
        help="DISK's U-Net on TensorRT fp16 (3x faster, same features) or torch fp32",
    )
    parser.add_argument(
        "--rotations",
        choices=tuple(ROTATIONS),
        default="none",
        help="for a pair that doesn't match as is, also try the other image rotated in these steps",
    )
    parser.add_argument("--min-inliers", type=int, default=20)
    parser.add_argument("--abs-threshold", type=float, default=0.08)
    parser.add_argument("--noise-k", type=float, default=6.0)
    parser.add_argument("--max-noise", type=float, default=0.06)
    # Off by default since 2026-10-29: the only labeled duplicate that needed it (dups2/090) turned out
    # to be two different frames of one shoot, which must not match.
    parser.add_argument("--strong-inliers", type=int, default=0, help="0 = off")
    parser.add_argument("--strong-ratio", type=float, default=0.5)
    # Off until tuned on hand marks (viewer: pair_marks.json); try them with --rescore.
    parser.add_argument("--max-moved", type=float, default=0.0, help="0 = off")
    parser.add_argument("--max-interior-change", type=float, default=0.0, help="0 = off")


def params_from(args: argparse.Namespace) -> Params:
    return Params(
        args.min_inliers,
        args.abs_threshold,
        args.noise_k,
        args.max_noise,
        args.strong_inliers,
        args.strong_ratio,
        args.max_moved,
        args.max_interior_change,
    )


def score(row: dict, out: dict, expected: str | None) -> None:
    """Adds a pair's result `out` to its row, and whether its decision is `expected`. An error is never the expected
    decision; a row with no expected decision (None) gets no `correct`."""
    row.update(out)
    if expected is not None:
        row["correct"] = row["decision"] == expected


def write_outputs(prefix: str, args, params: Params, labels: tuple, results: list[dict], seconds: float) -> None:
    with (WORK_DIR / f"{prefix}_{args.model}.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, COLUMNS, extrasaction="ignore")
        w.writeheader()
        for r in results:
            w.writerow(csv_row(r))

    decisions = ["match", "unrelated", "error"]
    confusion = {lab: {d: sum(r["label"] == lab and r["decision"] == d for r in results) for d in decisions} for lab in labels}
    summary = {
        "pairs_set": args.pairs,
        "model": args.model,
        "floor": args.floor,
        "features": getattr(args, "features", "disk"),
        "params": vars(params),
        "pairs": len(results),
        "seconds": seconds,
        "confusion": confusion,
    }
    (WORK_DIR / f"{prefix}_{args.model}.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")

    print(f"\n{len(results)} pairs ({seconds} s of image work)")
    print("| label | " + " | ".join(decisions) + " |")
    print("|---|" + "---|" * len(decisions))
    for lab, counts in confusion.items():
        print(f"| {lab} | " + " | ".join(str(counts[d]) for d in decisions) + " |")

    def quantiles(rows: list[dict], key: str) -> str:
        return " / ".join(f"{v:.4f}" for v in np.quantile([float(r[key]) for r in rows], [0.1, 0.5, 0.9, 0.99]))

    for lab in labels:
        aligned = [r for r in results if r["label"] == lab and "blob_frac" in r]
        if aligned:
            print(
                f"{lab} (aligned: {len(aligned)}): blob_frac p10/50/90/99 {quantiles(aligned, 'blob_frac')}; "
                f"noise {quantiles(aligned, 'noise')}"
            )
