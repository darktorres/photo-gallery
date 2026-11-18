"""The benches' pure helpers: bench_hdd's `--dense` blocks and bench_ann's `rerank` with its `-1` padding. Stdlib unittest,
as test_viewer.py:

    python -m unittest discover -s eval -p 'test_*.py'

common.py resolves the work folders when it loads, so the environment is set first, to temp folders.
"""

import os
import sys
import tempfile
import unittest
from pathlib import Path

_ROOT = Path(tempfile.mkdtemp(prefix="bench-test-"))
os.environ["PG_ROOT"] = str(_ROOT)
os.environ["PG_DATA_ROOT"] = str(_ROOT / "data")
os.environ["PG_WORK_DIR"] = str(_ROOT / "runs" / "eval")
os.environ["PG_CACHE_DIR"] = str(_ROOT / "cache")

import numpy as np  # noqa: E402 (after the environment above)

import bench_ann  # noqa: E402
import bench_hdd  # noqa: E402

# common.py keeps the folders this module set. Drop it from the import cache, so a test module that imports it next
# (test_viewer, which sets its own folders) gets a fresh one. The benches above keep the module they were given.
sys.modules.pop("common", None)

ORDERS = ["path", "mft", "lcn", "random", "path_qd8", "lcn_qd8"]


class DenseSamples(unittest.TestCase):
    def test_blocks_are_consecutive_and_disjoint(self):
        files = [f"f{i:02}" for i in range(14)]  # 14 // 6 = 2 per step, two files to spare
        samples = bench_hdd.dense_samples(files, ORDERS, 2)
        self.assertEqual(list(samples), ORDERS)
        blocks = list(samples.values())
        self.assertTrue(all(len(b) == 2 for b in blocks))
        self.assertEqual(len({p for b in blocks for p in b}), 12, "the blocks overlap")
        self.assertEqual(blocks[0], ["f00", "f01"])
        self.assertEqual(blocks[1], ["f02", "f03"])
        self.assertEqual(blocks[5], ["f10", "f11"])

    def test_too_few_files_is_refused_with_the_count(self):
        files = [f"f{i}" for i in range(11)]
        with self.assertRaises(ValueError) as cm:
            bench_hdd.dense_samples(files, ORDERS, 2)
        self.assertIn("--dense 2 x 6 orders needs 12 files; 11 found", str(cm.exception))

    def test_exactly_enough_files_is_accepted(self):
        files = [f"f{i}" for i in range(12)]
        samples = bench_hdd.dense_samples(files, ORDERS, 2)
        self.assertEqual(sum(len(b) for b in samples.values()), 12)


class Rerank(unittest.TestCase):
    def setUp(self):
        # Four unit-ish vectors; the query is [1, 0].
        self.base = np.array([[1, 0], [0, 1], [0.6, 0.8], [-1, 0]], dtype=np.float16)
        self.q = np.array([[1, 0]], dtype=np.float32)

    def test_candidates_are_ranked_by_exact_cosine_and_padded(self):
        out = bench_ann.rerank(self.base, self.q, np.array([[0, 1, 2, -1]]))
        self.assertEqual(out.shape, (1, bench_ann.K))
        self.assertEqual(out[0, :3].tolist(), [0, 2, 1])
        self.assertEqual(out[0, 3:].tolist(), [-1] * (bench_ann.K - 3))

    def test_no_candidates_is_all_padding(self):
        out = bench_ann.rerank(self.base, self.q, np.array([[-1, -1]]))
        self.assertEqual(out.tolist(), [[-1] * bench_ann.K])

    def test_the_top_k_are_kept_when_there_are_more(self):
        base = np.random.default_rng(0).standard_normal((40, 8)).astype(np.float16)
        q = np.random.default_rng(1).standard_normal((1, 8)).astype(np.float32)
        out = bench_ann.rerank(base, q, np.arange(40).reshape(1, 40))
        scores = base.astype(np.float32) @ q[0]
        self.assertEqual(out[0].tolist(), np.argsort(-scores)[: bench_ann.K].tolist())


if __name__ == "__main__":
    unittest.main()
