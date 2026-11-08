"""The structure an index relies on, measured on the base and on a structureless control (one Gaussian with the
base's mean and covariance, as the invalid first bake-off used): nearest-neighbour cosines, local intrinsic
dimensionality, hubness, neighbours >= FLOOR per row (by nested size), and cluster sizes."""

import numpy as np
import torch

from ann.data import FLOOR, MAIN, SIZES, Base, size_tag
from ann.exact import TOP, topk
from ann.methods import kmeans


def gaussian_like(x: np.ndarray, seed: int = 0) -> np.ndarray:
    mean = x.mean(0)
    w, v = np.linalg.eigh(np.cov(x - mean, rowvar=False))
    z = np.random.default_rng(seed).standard_normal(x.shape).astype(np.float32)
    g = mean + z @ (v * np.sqrt(np.clip(w, 0, None))).T
    return (g / np.linalg.norm(g, axis=1, keepdims=True)).astype(np.float32)


def stats(x: np.ndarray, sims: np.ndarray, ids: np.ndarray) -> dict:
    nn = sims[:, 0]
    # Levina-Bickel MLE of the local intrinsic dimensionality on the 20 nearest (angular distances).
    d = np.sqrt(np.clip(2 - 2 * sims[:, :20].astype(np.float64), 1e-12, None))
    lid = -1 / np.mean(np.log(d[:, :-1] / d[:, -1:]), axis=1)
    occ = np.bincount(ids[:, :10].ravel(), minlength=len(x)).astype(np.float64)
    skew = float(((occ - occ.mean()) ** 3).mean() / occ.std() ** 3)
    above = (sims >= FLOOR).sum(1)
    nlist = int(np.sqrt(len(x)))
    c = torch.from_numpy(kmeans(x, nlist)).cuda()
    xt = torch.from_numpy(x).cuda()
    assign = torch.cat([(xt[s : s + 8192] @ c.T).argmax(1) for s in range(0, len(xt), 8192)])
    sizes = torch.bincount(assign, minlength=nlist).cpu().numpy()
    return {
        "n": len(x),
        "nn_cos_p5_p50_p95": np.percentile(nn, [5, 50, 95]).round(4).tolist(),
        "lid_p10_p50_p90": np.percentile(lid, [10, 50, 90]).round(2).tolist(),
        "hubness_skew_n10": round(skew, 3),
        "neighbours_ge_floor_mean_p50_p99": [
            round(float(above.mean()), 3),
            *np.percentile(above, [50, 99]).tolist(),
        ],
        f"rows_with_{TOP}_or_more": int((above >= TOP).sum()),
        "cluster_sizes_cv_max_over_mean": [
            round(float(sizes.std() / sizes.mean()), 3),
            round(float(sizes.max() / sizes.mean()), 2),
        ],
        "kmeans_nlist": nlist,
    }


def characterize(base: Base, ground_truth) -> dict:
    out = {}
    for size in SIZES:
        sub = base.subset(size)
        gt = ground_truth(base, size)
        out[size_tag(size)] = stats(base.x[sub], gt["sims"], gt["ids"])
    x = base.x[base.subset(MAIN)]
    g = gaussian_like(x)
    sims, ids, _ = topk(g, TOP)
    out["control_gaussian"] = stats(g, sims, ids)
    return out
