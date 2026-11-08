"""Planted hard duplicates for the ANN bake-off (eval/ann): edited copies of real test-set images, saved as files.

Usage: PG_DATA_ROOT=D:\\PhotoGallery\\datasets PG_WORK_DIR=D:\\PhotoGallery\\eval_ann python plant.py [--per-edit 1000]

For each edit in edits.TRANSFORMS, --per-edit distinct source images are decoded in full, edited and saved as JPEG
(quality 95; the jpeg_q30 edit is the source saved at quality 30) at DATA_ROOT/planted/<edit>/<source row>.jpg.
Each source gets one edit only, so a source's easy copies can't push its hard ones out of the top 10. The copies
are then embedded like any other file (embed.py), through the same decode as the rest of the base.

Writes the run folder: WORK_DIR/files.txt (the test set as pinterest/..., in its files.txt order, then the planted
copies as planted/...) and WORK_DIR/planted.json (each copy's row, its source's row and the edit). Existing copies
are kept, so a rerun only makes the missing ones.
"""

import argparse
import json
import random
from multiprocessing import Pool
from pathlib import Path

from common import DATA_ROOT, WORK_DIR, load_rgb
from edits import TRANSFORMS, edit_seed

TEST_LIST = Path(r"D:\PhotoGallery\eval\files.txt")  # the test set's canonical list (common.image_list)
TEST_DIR = "pinterest"  # the test set's folder under DATA_ROOT
PLANTED_DIR = "planted"


def make_copy(job: tuple[str, int, str, int]) -> tuple[str, bool]:
    """Writes one edited copy; returns (its path relative to DATA_ROOT, ok)."""
    edit, source_row, source_path, seed = job
    rel = f"{PLANTED_DIR}/{edit}/{source_row:06d}.jpg"
    out = DATA_ROOT / rel
    if out.exists():
        return rel, True
    try:
        im = load_rgb(DATA_ROOT / TEST_DIR / source_path)
        out.parent.mkdir(parents=True, exist_ok=True)
        tmp = out.with_suffix(".tmp")
        if edit == "jpeg_q30":
            im.save(tmp, "JPEG", quality=30)
        else:
            TRANSFORMS[edit](im, random.Random(seed)).save(tmp, "JPEG", quality=95)
        tmp.replace(out)
        return rel, True
    except Exception:
        return rel, False


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--per-edit", type=int, default=1000)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--workers", type=int, default=12)
    args = parser.parse_args()
    if not (DATA_ROOT / TEST_DIR).is_dir():
        raise SystemExit(f"{DATA_ROOT / TEST_DIR} not found: set PG_DATA_ROOT to the folder that holds {TEST_DIR}/")

    test = TEST_LIST.read_text(encoding="utf-8").splitlines()
    edits = list(TRANSFORMS)
    sources = random.Random(args.seed).sample(range(len(test)), args.per_edit * len(edits))
    jobs = []
    for t, edit in enumerate(edits):
        for item, row in enumerate(sorted(sources[t * args.per_edit : (t + 1) * args.per_edit])):
            jobs.append((edit, row, test[row], edit_seed(args.seed + t, item)))

    with Pool(args.workers) as pool:
        results = pool.map(make_copy, jobs, chunksize=16)

    files = [f"{TEST_DIR}/{f}" for f in test]
    planted = []
    failed = 0
    for (edit, row, _, _), (rel, ok) in zip(jobs, results, strict=True):
        if not ok:
            failed += 1
            continue
        planted.append({"row": len(files), "source": row, "edit": edit})
        files.append(rel)
    WORK_DIR.mkdir(parents=True, exist_ok=True)
    (WORK_DIR / "files.txt").write_text("\n".join(files), encoding="utf-8")
    (WORK_DIR / "planted.json").write_text(json.dumps(planted), encoding="utf-8")
    print(f"{len(planted)} planted copies ({failed} sources failed to decode), {len(files)} rows in {WORK_DIR}")


if __name__ == "__main__":
    main()
