"""Build the quick-test subset: every labeled file plus N random others.

Usage: python make_subset.py [--out D:\\PhotoGallery\\eval_smoke] [--others 3000]

Writes <out>/files.txt from the dataset as it is now; run the other scripts with
PG_WORK_DIR=<out> to work on the subset. Rerun after editing the labeled folders.
"""

import argparse
import random
from pathlib import Path

from common import DATA_ROOT, GROUPS_DIR, IMAGE_EXTS, NEGATIVES_DIR


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, default=Path(r"D:\PhotoGallery\eval_smoke"))
    parser.add_argument("--others", type=int, default=3000)
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()

    files = sorted(
        p.relative_to(DATA_ROOT).as_posix() for p in DATA_ROOT.rglob("*") if p.is_file() and p.suffix.lower() in IMAGE_EXTS
    )
    labeled = [f for f in files if f.startswith((f"{GROUPS_DIR}/", f"{NEGATIVES_DIR}/"))]
    rest = sorted(set(files) - set(labeled))
    others = random.Random(args.seed).sample(rest, min(args.others, len(rest)))

    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "files.txt").write_text("\n".join(sorted(labeled + others)), encoding="utf-8")
    print(f"{args.out / 'files.txt'}: {len(labeled)} labeled + {len(others)} others (of {len(files)} images)")


if __name__ == "__main__":
    main()
