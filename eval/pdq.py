"""PDQ perceptual hash (Meta's tier-1 near-duplicate hash), scored like the embedding models.

Usage: python pdq.py [--workers 6]          # hash every image -> emb_pdq.npy, then: evaluate.py --model pdq
       python pdq.py --synth [--count 300]  # synthetic robustness, same edits and sample as synth.py

PDQ is a 256-bit hash compared by Hamming distance d. Each hash is stored as a +-1/16 vector, so
cosine similarity = 1 - d/128 and every embedding tool (evaluate.py, verify.py, the viewer) works on
it unchanged. Meta's recommended match threshold is d <= 31 (cosine >= 0.758).

Images are hashed upright (EXIF applied) after downscaling to <= 512 px, as Meta's reference
hasher recommends; the hash itself downsamples to 64x64 internally.
"""

import argparse
import json
import random
import time
from multiprocessing import Pool

import numpy as np
import pdqhash
from PIL import Image

from common import DATA_ROOT, WORK_DIR, image_list, load_rgb
from edits import TRANSFORMS, edit_seed

PREP_SIDE = 512
MEANINGFUL_D = 31  # Meta's recommended PDQ match threshold


def prepare(im: Image.Image) -> np.ndarray:
    if max(im.size) > PREP_SIDE:
        im = im.copy()
        im.thumbnail((PREP_SIDE, PREP_SIDE), Image.Resampling.LANCZOS)
    return np.asarray(im)


def to_vector(bits: np.ndarray) -> np.ndarray:
    """0/1 bits (..., 256) -> +-1/16 float16 vectors: dot product = 1 - hamming/128."""
    return ((bits.astype(np.float32) * 2 - 1) / 16).astype(np.float16)


def hash_file(rel: str) -> tuple[np.ndarray, int] | None:
    try:
        bits, quality = pdqhash.compute(prepare(load_rgb(DATA_ROOT / rel)))
        return bits, int(quality)
    except Exception:
        return None


def edited_hashes(job: tuple[str, str, int]) -> np.ndarray | None:
    """The 8 dihedral hashes of one edited sample image (row 0 = as is)."""
    rel, transform, seed = job
    try:
        im = TRANSFORMS[transform](load_rgb(DATA_ROOT / rel), random.Random(seed))
        hashes, _ = pdqhash.compute_dihedral(prepare(im))
        return np.stack(hashes)
    except Exception:
        return None


def hash_all(workers: int) -> None:
    files = image_list()
    start = time.perf_counter()
    with Pool(workers) as pool:
        results = pool.map(hash_file, files, chunksize=32)
    elapsed = time.perf_counter() - start
    ok = np.array([r is not None for r in results])
    bits = np.zeros((len(files), 256), np.uint8)
    quality = np.zeros(len(files), np.int16)
    for i, r in enumerate(results):
        if r is not None:
            bits[i], quality[i] = r
    emb = to_vector(bits)
    emb[~ok] = 0
    np.save(WORK_DIR / "emb_pdq.npy", emb)
    np.save(WORK_DIR / "ok_pdq.npy", ok)
    np.save(WORK_DIR / "quality_pdq.npy", quality)
    print(
        f"pdq: {len(files)} images, {elapsed:.0f} s ({len(files) / elapsed:.0f} img/s, {workers} processes), "
        f"{int((~ok).sum())} failed; quality < 50 (unreliable hash): {int(((quality < 50) & ok).sum())}"
    )


def synth(count: int, seed: int, workers: int) -> None:
    files = image_list()
    ok = np.load(WORK_DIR / "ok_pdq.npy")
    db = np.load(WORK_DIR / "emb_pdq.npy").astype(np.float32)
    sample = sorted(random.Random(seed).sample(list(np.flatnonzero(ok)), count))
    originals = np.array(sample)

    t95 = None
    results_path = WORK_DIR / "results_pdq.json"
    if results_path.exists():
        points = json.loads(results_path.read_text(encoding="utf-8"))["operating_points"]
        t95 = next(p["threshold"] for p in points if p["target_recall"] == 0.95)

    rows = []
    with Pool(workers) as pool:
        for t_index, name in enumerate(TRANSFORMS):
            # Same per-image seeds as synth.py, so the edits are identical.
            jobs = [(files[i], name, edit_seed(seed + t_index, k)) for k, i in enumerate(sample)]
            hashes = pool.map(edited_hashes, jobs, chunksize=8)
            keep = np.array([h is not None for h in hashes])
            q = to_vector(np.stack([h for h in hashes if h is not None])).astype(np.float32)  # (m, 8, 256)
            orig = originals[keep]
            # Plain-query metrics at the top level (same fields as synth.py), dihedral nested.
            row: dict = {"transform": name}
            for variant, sims in (("plain", q[:, 0] @ db.T), ("dihedral", (q @ db.T).max(axis=1))):
                top = np.argsort(-sims, axis=1)[:, :10]
                hits = top == orig[:, None]
                d = np.rint((1 - sims[np.arange(len(orig)), orig]) * 128)
                metrics = {
                    "r1": float(hits[:, 0].mean()),
                    "r10": float(hits.any(1).mean()),
                    "d_p50": float(np.quantile(d, 0.5)),
                    "d_p90": float(np.quantile(d, 0.9)),
                    "within_31": float((d <= MEANINGFUL_D).mean()),
                    **({"above_t95": float((1 - d / 128 >= t95).mean())} if t95 is not None else {}),
                }
                if variant == "plain":
                    row.update(metrics)
                else:
                    row["dihedral"] = metrics
            rows.append(row)
            print(f"  {name}: done", flush=True)

    (WORK_DIR / "synth_pdq.json").write_text(json.dumps(rows, indent=2), encoding="utf-8")
    print(f"## pdq: synthetic robustness, {count} images, retrieved from {len(files):,} (d = Hamming distance)")
    for variant in ("plain", "dihedral"):
        print(f"\n{variant} query{' (best of the 8 rotations/mirrors)' if variant == 'dihedral' else ''}:")
        extra = " | >= thr@95%recall" if t95 is not None else ""
        print(f"| transform | R@1 | R@10 | d p50 | d p90 | d <= 31{extra} |")
        print("|---|---|---|---|---|---|" + ("---|" if t95 is not None else ""))
        for r in rows:
            v = r["dihedral"] if variant == "dihedral" else r
            cells = [
                r["transform"],
                f"{v['r1']:.3f}",
                f"{v['r10']:.3f}",
                f"{v['d_p50']:.0f}",
                f"{v['d_p90']:.0f}",
                f"{v['within_31']:.3f}",
            ] + ([f"{v['above_t95']:.3f}"] if t95 is not None else [])
            print("| " + " | ".join(cells) + " |")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--synth", action="store_true")
    parser.add_argument("--count", type=int, default=300)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--workers", type=int, default=6)
    args = parser.parse_args()
    if args.synth:
        synth(args.count, args.seed, args.workers)
    else:
        hash_all(args.workers)


if __name__ == "__main__":
    main()
