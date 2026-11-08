"""Exact k-NN self-join on the GPU: the ground truth, and the exact method's own timing.

The ground truth is fp32 (no fp16 rounding at this size), self excluded by id. Each row keeps its TOP most similar
rows; a row whose TOP-th neighbour is still >= FLOOR (a hub) also gets every neighbour >= FLOOR (radius).
"""

import time

import numpy as np
import torch

from ann.data import FLOOR, OUT, Base, size_tag

TOP = 64
BLOCK = 1024  # query rows per GEMM: a (BLOCK x n) fp32 block is ~0.8 GB at 206k rows


def topk(x: np.ndarray, k: int, fp16: bool = False) -> tuple[np.ndarray, np.ndarray, float]:
    """(sims, ids, seconds): each row's k most similar other rows. fp16: the GEMM in fp16 (fp32 accumulation),
    as a production exact join would run; the returned sims are then fp16-rounded."""
    torch.backends.cuda.matmul.allow_fp16_reduced_precision_reduction = False
    dt = torch.float16 if fp16 else torch.float32
    xt = torch.from_numpy(x).cuda().to(dt)
    n = len(xt)
    sims = torch.empty(n, k, device="cuda")
    ids = torch.empty(n, k, dtype=torch.int32, device="cuda")
    torch.cuda.synchronize()
    t = time.perf_counter()
    for s in range(0, n, BLOCK):
        block = xt[s : s + BLOCK] @ xt.T
        r = torch.arange(len(block), device="cuda")
        block[r, r + s] = -float("inf")
        v, i = block.float().topk(k, dim=1)
        sims[s : s + BLOCK], ids[s : s + BLOCK] = v, i.int()
    torch.cuda.synchronize()
    seconds = time.perf_counter() - t
    return sims.cpu().numpy(), ids.cpu().numpy(), seconds


def radius(x: np.ndarray, rows: np.ndarray, floor: float) -> list[tuple[np.ndarray, np.ndarray]]:
    """For each of `rows`: (ids, sims) of every other row with similarity >= floor, most similar first."""
    xt = torch.from_numpy(x).cuda()
    out = []
    for r in rows:
        s = xt @ xt[int(r)]
        s[int(r)] = -float("inf")
        i = torch.nonzero(s >= floor).flatten()
        o = torch.argsort(s[i], descending=True)
        out.append((i[o].int().cpu().numpy(), s[i[o]].cpu().numpy()))
    return out


def check_fp64(x: np.ndarray, sims: np.ndarray, ids: np.ndarray, rows: np.ndarray) -> dict:
    """Compares the GPU top lists of `rows` with float64 CPU results; a differing id must be a tie within 1e-6."""
    xd = x.astype(np.float64)
    bad = []
    for r in rows:
        s = xd @ xd[r]
        s[r] = -np.inf
        ref = np.argsort(-s, kind="stable")[: ids.shape[1]]
        if set(ref) != set(ids[r]):
            kth = s[ref[-1]]
            # ids that differ must sit at the boundary (ties with the k-th value)
            extra = set(ids[r]) ^ set(ref)
            if any(abs(s[i] - kth) > 1e-6 for i in extra):
                bad.append(int(r))
        if np.abs(np.sort(s[ids[r]])[::-1] - sims[r]).max() > 1e-5:
            bad.append(int(r))
    return {"rows": len(rows), "mismatched_rows": sorted(set(bad))}


def fp16_rate(x: np.ndarray) -> float:
    """Pairs/s of the fp16 join's step (GEMM + top-K) with every operand on the GPU."""
    _, _, seconds = topk(x, 10, fp16=True)
    return len(x) * (len(x) - 1) / seconds


def ground_truth(base: Base, size: int | None) -> dict:
    """Cached exact results for a nested subset, in its local ids (positions in base.subset(size))."""
    path = OUT / f"exact_{size_tag(size)}.npz"
    if path.exists():
        d = np.load(path)
        return {k: d[k] for k in d.files}
    sub = base.subset(size)
    x = base.x[sub]
    sims, ids, seconds = topk(x, TOP)
    hub_rows = np.flatnonzero(sims[:, -1] >= FLOOR)
    found = radius(x, hub_rows, FLOOR)
    hub_ptr = np.cumsum([0] + [len(i) for i, _ in found])
    hub_ids = np.concatenate([i for i, _ in found]) if found else np.zeros(0, np.int32)
    hub_sims = np.concatenate([s for _, s in found]) if found else np.zeros(0, np.float32)
    d = {
        "sims": sims,
        "ids": ids,
        "seconds": np.float64(seconds),
        "hub_rows": hub_rows,
        "hub_ptr": hub_ptr,
        "hub_ids": hub_ids,
        "hub_sims": hub_sims,
    }
    OUT.mkdir(parents=True, exist_ok=True)
    np.savez(path, **d)
    return d
