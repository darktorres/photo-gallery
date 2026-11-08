"""The bake-off's base: the test set plus the planted copies (plant.py), embedded with the production pipeline
(embed.py --model sscd_trt_fp16 --trim --draft), with what is known about each row.

Run with PG_DATA_ROOT=D:\\PhotoGallery\\datasets and PG_WORK_DIR=D:\\PhotoGallery\\eval_ann (plant.py's run folder).
Methods work on compact ids: the rows that embedded (ok), in files.txt order.
"""

import json
from dataclasses import dataclass, field

import numpy as np

from common import CACHE_DIR, WORK_DIR, ground_truth, image_list
from edits import TRANSFORMS

MODEL = "sscd_trt_fp16_trim_draft"
TEST_PREFIX = "pinterest/"
OUT = CACHE_DIR / "ann"
FLOOR, K = 0.4, 10  # the k-NN candidate rule (verify.py --pairs knn)
KMAX = 80  # most candidates any method returns per row (the largest rerank depth)
TIE = 1e-6  # a pair counts as required only if it beats the row's (K+1)th neighbour by this much
# Nested subsets of the test set (rank order; planted copies come with their sources). Every method is tuned,
# selected and reported at MAIN; the smaller sizes give the trend. VALIDATE is the size the gates ran at.
SIZES = (12_500, 25_000, 50_000)
MAIN = 50_000
VALIDATE = 25_000
EDITS = list(TRANSFORMS)


@dataclass
class Base:
    files: list[str]
    rows: np.ndarray  # compact id -> row in files.txt
    x: np.ndarray  # (n, d) float32, unit length
    planted_edit: np.ndarray  # index into EDITS, or -1
    planted_source: np.ndarray  # compact id of the copy's source, or -1
    group: np.ndarray  # dups2 group, or -1
    rank: np.ndarray  # nested-subset rank: a random permutation of the test rows; copies take their source's
    sub_cache: dict = field(default_factory=dict)

    def subset(self, size: int) -> np.ndarray:
        """Compact ids of a nested subset: test rows ranked below `size` and their planted copies (sorted)."""
        if size not in self.sub_cache:
            test = self.planted_edit < 0
            keep = test & (self.rank < size)
            copies = ~test & (self.planted_source >= 0)
            keep[copies] = keep[self.planted_source[copies]]
            self.sub_cache[size] = np.flatnonzero(keep)
        return self.sub_cache[size]


def size_tag(size: int) -> str:
    return f"{size / 1000:g}k"


def load_base() -> Base:
    files = image_list()
    emb = np.load(WORK_DIR / f"emb_{MODEL}.npy")
    ok = np.load(WORK_DIR / f"ok_{MODEL}.npy")
    rows = np.flatnonzero(ok)
    compact = np.full(len(files), -1)
    compact[rows] = np.arange(len(rows))
    x = emb[rows].astype(np.float32)
    x /= np.linalg.norm(x, axis=1, keepdims=True)

    n = len(rows)
    planted_edit = np.full(n, -1)
    planted_source = np.full(n, -1)
    for p in json.loads((WORK_DIR / "planted.json").read_text(encoding="utf-8")):
        c = compact[p["row"]]
        if c >= 0:
            planted_edit[c] = EDITS.index(p["edit"])
            planted_source[c] = compact[p["source"]]  # -1 if the source itself failed to embed

    gt = ground_truth([f[len(TEST_PREFIX) :] if f.startswith(TEST_PREFIX) else "" for f in files])
    group = np.full(n, -1)
    for g, members in enumerate(gt.groups):
        for m in members:
            if compact[m] >= 0:
                group[compact[m]] = g

    rank = np.zeros(n, dtype=np.int64)
    test = np.flatnonzero(planted_edit < 0)
    rank[test] = np.random.default_rng(0).permutation(len(test))
    copies = planted_source >= 0
    rank[copies] = rank[planted_source[copies]]
    return Base(files, rows, x, planted_edit, planted_source, group, rank)


def pair_keys(a: np.ndarray, b: np.ndarray, n: int) -> np.ndarray:
    """Unordered pair -> int64 key."""
    lo, hi = np.minimum(a, b).astype(np.int64), np.maximum(a, b).astype(np.int64)
    return lo * n + hi


def pair_half(keys: np.ndarray) -> np.ndarray:
    """0 = tuning half, 1 = evaluation half: a fixed hash of the pair, so a pair is always in the same half."""
    z = keys.astype(np.uint64) + np.uint64(0x9E3779B97F4A7C15)
    z = (z ^ (z >> np.uint64(30))) * np.uint64(0xBF58476D1CE4E5B9)
    z = (z ^ (z >> np.uint64(27))) * np.uint64(0x94D049BB133111EB)
    return ((z ^ (z >> np.uint64(31))) & np.uint64(1)).astype(np.int8)
