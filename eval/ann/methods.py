"""The methods' self-joins. Each returns, per search setting, (candidate ids per row in local ids, -1 = none) plus
times and memory; metrics.py rescores and scores them.

- GPU (here, Windows torch): k-means, and the IVF self-join (each row probes its nearest lists and is compared
  exactly, in fp16, with their members).
- CPU indexes (USearch, FAISS) run in the Linux container through cpu_worker.py (`tools/linux.py ann-worker`), whose
  Linux wheels have SIMD kernels; the Windows pip wheels of both have none (FINDINGS).
"""

import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import torch

from ann.data import KMAX, OUT

# The launcher that runs cpu_worker.py in the container, with the job's folders mounted (the worker's environment, the
# BLAS and OpenMP thread settings, is the launcher's too).
LAUNCHER = Path(__file__).resolve().parents[2] / "tools" / "linux.py"
# A sweep (one build, every search width) stops once its searches have taken this long in total: a self-join
# over the main base that needs more than 10 minutes of search is not a practical setting, and wider ones cost more.
MAX_SEARCH_S = 600


def kmeans(x: np.ndarray, nlist: int, iters: int = 25, seed: int = 0) -> np.ndarray:
    """Spherical k-means on the GPU (unit centroids, cosine assignment); an empty list is reseeded with a random
    point."""
    g = torch.Generator(device="cuda").manual_seed(seed)
    xt = torch.from_numpy(x).cuda()
    c = xt[torch.randperm(len(xt), generator=g, device="cuda")[:nlist]].clone()
    for _ in range(iters):
        assign = torch.cat([(xt[s : s + 8192] @ c.T).argmax(1) for s in range(0, len(xt), 8192)])
        sums = torch.zeros_like(c).index_add_(0, assign, xt)
        counts = torch.bincount(assign, minlength=nlist)
        empty = counts == 0
        sums[empty] = xt[torch.randint(len(xt), (int(empty.sum()),), generator=g, device="cuda")]
        c = torch.nn.functional.normalize(sums, dim=1)
    return c.cpu().numpy()


def nearest_lists(xt: torch.Tensor, c: torch.Tensor, r: int) -> torch.Tensor:
    return torch.cat([(xt[s : s + 8192] @ c.T).topk(r, dim=1).indices for s in range(0, len(xt), 8192)])


def gpu_ivf(x: np.ndarray, nlist: int, r: int, nprobes: list[int], seed: int = 0) -> tuple[dict, list[dict]]:
    """IVF self-join on the GPU. Build: k-means, then each row goes into its r nearest lists. Join: each row probes
    its nprobe nearest lists and is compared exactly (fp16 GEMM, fp32 accumulation) with their members, keeping
    its KMAX best. Returns (build info, one result per nprobe: ids + seconds)."""
    torch.backends.cuda.matmul.allow_fp16_reduced_precision_reduction = False
    torch.cuda.synchronize()
    t = time.perf_counter()
    c = torch.from_numpy(kmeans(x, nlist, seed=seed)).cuda()
    xt = torch.from_numpy(x).cuda()
    x16 = xt.half()
    member_list = nearest_lists(xt, c, r).flatten()
    member_row = torch.arange(len(xt), device="cuda").repeat_interleave(r)
    order = torch.argsort(member_list, stable=True)
    member_row = member_row[order]
    bounds = torch.searchsorted(member_list[order], torch.arange(nlist + 1, device="cuda")).cpu().tolist()
    torch.cuda.synchronize()
    build = {"build_s": time.perf_counter() - t, "index_bytes": int(len(member_row) * 4 + c.numel() * 4)}

    results = []
    probes_all = nearest_lists(xt, c, max(nprobes))
    for nprobe in nprobes:
        torch.cuda.synchronize()
        t = time.perf_counter()
        probe_list = probes_all[:, :nprobe].flatten()
        probe_row = torch.arange(len(xt), device="cuda").repeat_interleave(nprobe)
        po = torch.argsort(probe_list, stable=True)
        probe_row = probe_row[po]
        pbounds = torch.searchsorted(probe_list[po], torch.arange(nlist + 1, device="cuda")).cpu().tolist()
        best_s = torch.full((len(xt), KMAX), -float("inf"), device="cuda")
        best_i = torch.full((len(xt), KMAX), -1, dtype=torch.int64, device="cuda")
        for li in range(nlist):
            q = probe_row[pbounds[li] : pbounds[li + 1]]
            m = member_row[bounds[li] : bounds[li + 1]]
            if len(q) == 0 or len(m) == 0:
                continue
            s = (x16[q] @ x16[m].T).float()
            s[q[:, None] == m[None, :]] = -float("inf")
            v, i = s.topk(min(KMAX, len(m)), dim=1)
            ids = torch.cat([best_i[q], m[i]], 1)
            sims = torch.cat([best_s[q], v], 1)
            # A row reached through two of its probed lists (r > 1) must count once.
            ids_sorted, o = ids.sort(dim=1)
            sims = sims.gather(1, o)
            dup = torch.zeros_like(ids_sorted, dtype=torch.bool)
            dup[:, 1:] = (ids_sorted[:, 1:] == ids_sorted[:, :-1]) & (ids_sorted[:, 1:] >= 0)
            sims[dup] = -float("inf")
            v, o = sims.topk(KMAX, dim=1)
            best_s[q], best_i[q] = v, ids_sorted.gather(1, o)
        best_i[best_s == -float("inf")] = -1
        torch.cuda.synchronize()
        results.append({"param": {"nprobe": nprobe}, "search_s": time.perf_counter() - t, "ids": best_i.int().cpu().numpy()})
    return build, results


def run_cpu(tag: str, base_file: Path, family: str, config: dict, params: list[dict], **job) -> tuple[dict, list[dict]]:
    """Builds and searches one CPU index in the container's worker (self-join over base_file). Returns (build info, one
    result per param: ids + seconds). The job names this machine's paths; the launcher mounts their folders and gives the
    worker its own paths for them."""
    work = OUT / "jobs" / tag
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    spec = {
        "base": str(base_file.resolve()),
        "queries": None,
        "metric": "ip",
        "family": family,
        "config": config,
        "params": params,
        "k": KMAX,
        "out": str(work.resolve()),
        "order": "file",
        "max_search_s": MAX_SEARCH_S,
        **{k: (str(v.resolve()) if isinstance(v, Path) else v) for k, v in job.items()},
    }
    (work / "job.json").write_text(json.dumps(spec), encoding="utf-8")
    proc = subprocess.run(
        [sys.executable, str(LAUNCHER), "ann-worker", str(work / "job.json")],
        capture_output=True,
        check=False,
        text=True,
    )
    if proc.returncode != 0 or not (work / "result.json").exists():
        raise RuntimeError(f"worker failed ({tag}): {proc.stdout[-2000:]} {proc.stderr[-2000:]}")
    res = json.loads((work / "result.json").read_text(encoding="utf-8"))
    out = [{"param": s["param"], "search_s": s["search_s"], "ids": np.load(work / s["file"])} for s in res["searches"]]
    build = {k: v for k, v in res.items() if k != "searches"}
    shutil.rmtree(work)
    return build, out


def pca(x: np.ndarray, dim: int) -> np.ndarray:
    """x projected on its top `dim` principal axes, not centred and not renormalized, so inner products of the
    projections approximate the cosines (centring would add mean terms to every product)."""
    xt = torch.from_numpy(x).cuda().double()
    mean = xt.mean(0)
    cov = (xt - mean).T @ (xt - mean) / len(xt)
    _, v = torch.linalg.eigh(cov)
    return (xt @ v[:, -dim:]).float().cpu().numpy()
