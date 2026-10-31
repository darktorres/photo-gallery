"""ANN bake-off (PLAN M0): USearch HNSW (int8) vs FAISS IVF-PQ / IVF-SQ8, on 5M synthetic SSCD-like vectors.

No full-library embeddings exist yet, so the vectors are synthetic, fitted to the subset's real SSCD fp16 embeddings:
- background: a Gaussian with the subset's mean and covariance, L2-normalized (checked against the real random-pair
  and nearest-neighbour cosines at the subset's size);
- planted duplicates: 10,000 queries (not in the index), each with one partner in the index at a cosine uniform in
  0.3-0.99 (the labeled pairs are mostly near-identical, median 0.988; heavy crops sit at 0.4-0.88), reported by band.

Measures, per index: build (insert) speed, RAM, recall@10 vs exact search, the share of partners found in the top 10
(the k-NN candidate rule: k = 10, floor 0.4), and queries per second. Exact search (GPU brute force, fp16 with fp32
accumulation) is the ground truth and is timed too, to size the all-pairs exact option for 70M.

Stages are cached in CACHE_DIR/ann; run all, or name stages: gen check exact usearch faiss.
"""

import argparse
import gc
import json
import os
import time

import numpy as np

from common import CACHE_DIR, WORK_DIR, ground_truth, image_list

DIM = 512
OUT = CACHE_DIR / "ann"
THREADS = os.cpu_count() or 8
FLOOR, K = 0.4, 10
CHUNK = 262_144


def normalize(x: np.ndarray) -> np.ndarray:
    return x / np.linalg.norm(x, axis=1, keepdims=True)


def real_embeddings() -> tuple[np.ndarray, np.ndarray]:
    """Subset SSCD fp16 embeddings (normalized, valid rows) and the cosines of the labeled duplicate pairs."""
    emb = np.load(WORK_DIR / "emb_sscd_fp16.npy").astype(np.float32)
    ok = np.load(WORK_DIR / "ok_sscd_fp16.npy")
    emb = normalize(emb)
    gt = ground_truth(image_list())
    pos = [(a, b) for a, b in gt.positives if ok[a] and ok[b]]
    cos = np.array([emb[a] @ emb[b] for a, b in pos], dtype=np.float32)
    return emb[ok.astype(bool)], cos


class Gen:
    """Background vectors from a Gaussian fitted to the real embeddings."""

    def __init__(self, real: np.ndarray, seed: int):
        self.mean = real.mean(0)
        cov = np.cov(real - self.mean, rowvar=False)
        w, v = np.linalg.eigh(cov)
        self.proj = (v * np.sqrt(np.clip(w, 0, None))).T  # z @ proj ~ N(0, cov)
        self.rng = np.random.default_rng(seed)

    def __call__(self, n: int) -> np.ndarray:
        z = self.rng.standard_normal((n, DIM), dtype=np.float32)
        return normalize(self.mean + z @ self.proj).astype(np.float32)


def paths(n: int) -> dict:
    tag = f"{n // 1_000_000}M" if n % 1_000_000 == 0 else str(n)
    return {k: OUT / f"{tag}_{k}" for k in ("base.f16", "queries.npy", "partners.npy", "meta.json", "exact.npz")}


def stage_gen(n: int, nq: int, p: dict) -> None:
    if p["base.f16"].exists():
        return
    real, dup_cos = real_embeddings()
    gen = Gen(real, seed=1)
    rng = np.random.default_rng(2)
    base = np.lib.format.open_memmap(p["base.f16"], mode="w+", dtype=np.float16, shape=(n, DIM))
    t = time.perf_counter()
    for s in range(0, n, CHUNK):
        base[s : s + CHUNK] = gen(min(CHUNK, n - s))
    queries = gen(nq)
    # Each query's partner: cosine c drawn from the real duplicate pairs, along a fresh direction orthogonal to it.
    c = rng.uniform(0.3, 0.99, nq).astype(np.float32)
    u = gen(nq)
    u = normalize(u - (u * queries).sum(1, keepdims=True) * queries)
    partners = normalize(c[:, None] * queries + np.sqrt(1 - c[:, None] ** 2) * u)
    where = rng.choice(n, nq, replace=False)
    base[where] = partners.astype(np.float16)
    base.flush()
    del base
    np.save(p["queries.npy"], queries.astype(np.float16))
    np.save(p["partners.npy"], where)
    meta = {
        "n": n,
        "nq": nq,
        "partner_cos": c.tolist(),
        "gen_s": time.perf_counter() - t,
        "real_dup_cos_pcts": np.percentile(dup_cos, [5, 25, 50, 75, 95]).round(3).tolist(),
        "real_dups": len(dup_cos),
    }
    p["meta.json"].write_text(json.dumps(meta), encoding="utf-8")
    print(
        f"gen: {n:,} vectors in {meta['gen_s']:.0f} s; real duplicate cosines p5/25/50/75/95 "
        f"{meta['real_dup_cos_pcts']} ({len(dup_cos)} pairs)",
        flush=True,
    )


def stage_check() -> dict:
    """Is the synthetic background like the real one? Random-pair and nearest-neighbour cosines at the subset's size."""
    real, _ = real_embeddings()
    gt = ground_truth(image_list())
    syn = Gen(real, seed=3)(len(real))
    rng = np.random.default_rng(4)
    out = {}
    # Drop labeled duplicates' extra members so real nearest neighbours are mostly unrelated images.
    keep = np.ones(len(image_list()), bool)
    for g in gt.groups:
        keep[g[1:]] = False
    ok = np.load(WORK_DIR / "ok_sscd_fp16.npy").astype(bool)
    real_u = normalize(np.load(WORK_DIR / "emb_sscd_fp16.npy").astype(np.float32)[keep & ok])
    for name, e in (("real", real_u), ("synthetic", syn[: len(real_u)])):
        i, j = rng.integers(0, len(e), (2, 200_000))
        rand = (e[i] * e[j]).sum(1)
        sim = e @ e.T
        np.fill_diagonal(sim, -2)
        nn = sim.max(1)
        out[name] = {
            "random_pair_p50_p99_p9999": np.percentile(rand, [50, 99, 99.99]).round(3).tolist(),
            "nn_cos_p5_p50_p95": np.percentile(nn, [5, 50, 95]).round(3).tolist(),
            "n": len(e),
        }
        print(
            f"check {name:<9}: random-pair cosine p50/p99/p99.99 {out[name]['random_pair_p50_p99_p9999']}, "
            f"nearest-neighbour cosine p5/p50/p95 {out[name]['nn_cos_p5_p50_p95']} (n={len(e):,})",
            flush=True,
        )
    return out


QBLOCK = 1024  # queries per GEMM: the (QBLOCK x CHUNK) similarity block must fit the 6 GB GPU (1 GB in fp32)


def all_pairs_rate() -> float:
    """Pairs/s of the step an exact all-pairs k-NN would repeat: an fp16 GEMM (tensor cores, fp32 accumulation) of a
    query block against a base chunk, then top-K, both resident on the GPU (streaming the base is not included)."""
    import torch

    g = torch.Generator(device="cuda").manual_seed(0)
    a = torch.randn(4096, DIM, device="cuda", dtype=torch.float16, generator=g)
    b = torch.randn(CHUNK // 2, DIM, device="cuda", dtype=torch.float16, generator=g)
    for _ in range(3):
        (a @ b.T).topk(K, dim=1)
    torch.cuda.synchronize()
    t = time.perf_counter()
    for _ in range(10):
        (a @ b.T).topk(K, dim=1)
    torch.cuda.synchronize()
    return 10 * len(a) * len(b) / (time.perf_counter() - t)


def stage_exact(p: dict) -> dict:
    """Top-K by brute force on the GPU, in fp32 so the ground truth carries no fp16 rounding."""
    import torch

    if p["exact.npz"].exists():
        d = np.load(p["exact.npz"])
        return {"ids": d["ids"], "sims": d["sims"], "rate": float(d["rate"])}
    base = np.load(p["base.f16"], mmap_mode="r")
    q = torch.from_numpy(np.load(p["queries.npy"])).cuda().float()
    n = len(base)
    best_s = torch.full((len(q), K), -2.0, device="cuda")
    best_i = torch.zeros((len(q), K), dtype=torch.long, device="cuda")
    t = time.perf_counter()
    for s in range(0, n, CHUNK):
        chunk = torch.from_numpy(np.ascontiguousarray(base[s : s + CHUNK])).cuda().float()
        for r in range(0, len(q), QBLOCK):
            v, i = (q[r : r + QBLOCK] @ chunk.T).topk(K, dim=1)
            allv = torch.cat([best_s[r : r + QBLOCK], v], 1)
            alli = torch.cat([best_i[r : r + QBLOCK], i + s], 1)
            best_s[r : r + QBLOCK], o = allv.topk(K, dim=1)
            best_i[r : r + QBLOCK] = alli.gather(1, o)
    torch.cuda.synchronize()
    wall = time.perf_counter() - t
    rate = all_pairs_rate()
    print(
        f"exact: {len(q):,} queries x {n:,} in {wall:.0f} s (fp32); fp16 GEMM + top-{K}: {rate / 1e9:.1f} G pairs/s, "
        f"70M all-pairs ≈ {70e6**2 / rate / 3600:.0f} GPU hours",
        flush=True,
    )
    ids, sims = best_i.cpu().numpy(), best_s.cpu().numpy()
    np.savez(p["exact.npz"], ids=ids, sims=sims, rate=rate)
    return {"ids": ids, "sims": sims, "rate": rate}


BANDS = ((0.3, 0.4), (0.4, 0.5), (0.5, 0.6), (0.6, 0.8), (0.8, 1.0))


def partner_cos(p: dict) -> np.ndarray:
    return np.array(json.loads(p["meta.json"].read_text(encoding="utf-8"))["partner_cos"])


def score(found: np.ndarray, exact: dict, partners: np.ndarray, cos: np.ndarray) -> dict:
    """recall@10 vs exact search, and the share of planted partners in the top 10 by cosine band (≥ FLOOR overall)."""
    ex = exact["ids"]
    recall = np.mean([len(set(found[i]) & set(ex[i])) / K for i in range(len(ex))])
    hit = np.array([partners[i] in set(found[i]) for i in range(len(ex))])
    bands = {f"{a:.1f}-{b:.1f}": float(hit[(cos >= a) & (cos < b)].mean()) for a, b in BANDS}
    return {"recall10": float(recall), "partners_found": float(hit[cos >= FLOOR].mean()), "bands": bands}


def stage_usearch(p: dict, exact: dict, dtype: str) -> list[dict]:
    from usearch.index import Index

    base = np.load(p["base.f16"], mmap_mode="r")
    q = np.load(p["queries.npy"]).astype(np.float32)
    partners = np.load(p["partners.npy"])
    cos = partner_cos(p)
    n = len(base)
    index = Index(ndim=DIM, metric="cos", dtype=dtype, connectivity=16, expansion_add=128)
    t = time.perf_counter()
    for s in range(0, n, CHUNK):
        x = np.asarray(base[s : s + CHUNK], dtype=np.float32)
        index.add(np.arange(s, s + len(x)), x, threads=THREADS)
    build = time.perf_counter() - t
    print(
        f"usearch {dtype}: built {n:,} in {build:.0f} s ({n / build:,.0f} vectors/s, {THREADS} threads); "
        f"memory_usage {index.memory_usage / 2**30:.2f} GB",
        flush=True,
    )
    rows = []
    for ef in (16, 32, 64, 128, 256):
        index.expansion_search = ef
        t = time.perf_counter()
        m = index.search(q, K, threads=THREADS)
        qs = time.perf_counter() - t
        sc = score(np.asarray(m.keys), exact, partners, cos)
        rows.append(
            {
                "index": f"usearch HNSW {dtype}",
                "param": f"ef={ef}",
                "qps": len(q) / qs,
                "build_s": build,
                "vectors_per_s": n / build,
                "ram_gb": index.memory_usage / 2**30,
                **sc,
            }
        )
        print(
            f"  ef {ef:>3}: recall@10 {sc['recall10']:.4f}, partners ≥ {FLOOR} {sc['partners_found']:.4f} "
            f"{sc['bands']}, {len(q) / qs:,.0f} q/s",
            flush=True,
        )
    del index
    gc.collect()
    return rows


def rerank(base, q: np.ndarray, cand: np.ndarray) -> np.ndarray:
    """Exact cosine on the stored fp16 vectors for each query's candidates; top K."""
    out = np.empty((len(q), K), dtype=np.int64)
    for i in range(len(q)):
        c = cand[i][cand[i] >= 0]
        order = np.sort(c)
        s = np.asarray(base[order], dtype=np.float32) @ q[i]
        out[i] = order[np.argsort(-s)[:K]]
    return out


def stage_faiss(p: dict, exact: dict) -> list[dict]:
    import faiss

    faiss.omp_set_num_threads(THREADS)
    base = np.load(p["base.f16"], mmap_mode="r")
    q = np.load(p["queries.npy"]).astype(np.float32)
    partners = np.load(p["partners.npy"])
    cos = partner_cos(p)
    n = len(base)
    nlist = 8192
    rows = []
    rng = np.random.default_rng(5)
    train = np.asarray(base[np.sort(rng.choice(n, 300_000, replace=False))], dtype=np.float32)
    for name, spec in (("IVF-PQ64", f"IVF{nlist},PQ64"), ("IVF-SQ8", f"IVF{nlist},SQ8")):
        index = faiss.index_factory(DIM, spec, faiss.METRIC_INNER_PRODUCT)
        t = time.perf_counter()
        index.train(train)
        train_s = time.perf_counter() - t
        t = time.perf_counter()
        for s in range(0, n, CHUNK):
            index.add(np.asarray(base[s : s + CHUNK], dtype=np.float32))
        add_s = time.perf_counter() - t
        ivf = faiss.extract_index_ivf(index)
        # Codes + 8-byte ids per vector, plus the coarse centroids (RSS would also count the mapped base file).
        ram = (n * (ivf.code_size + 8) + nlist * DIM * 4) / 2**30
        print(
            f"faiss {name}: train {train_s:.0f} s, add {n:,} in {add_s:.0f} s ({n / add_s:,.0f} vectors/s); RAM ≈ {ram:.2f} GB",
            flush=True,
        )
        for nprobe in (16, 64, 256):
            faiss.extract_index_ivf(index).nprobe = nprobe
            variants = [("", K)] + ([(" + rerank 100", 100)] if name == "IVF-PQ64" else [])
            for suffix, k in variants:
                t = time.perf_counter()
                _, ids = index.search(q, k)
                if k > K:
                    ids = rerank(base, q, ids)
                qs = time.perf_counter() - t
                sc = score(ids[:, :K], exact, partners, cos)
                rows.append(
                    {
                        "index": f"faiss {name}{suffix}",
                        "param": f"nprobe={nprobe}",
                        "qps": len(q) / qs,
                        "build_s": train_s + add_s,
                        "vectors_per_s": n / add_s,
                        "ram_gb": ram,
                        **sc,
                    }
                )
                print(
                    f"  nprobe {nprobe:>3}{suffix}: recall@10 {sc['recall10']:.4f}, partners "
                    f"≥ {FLOOR} {sc['partners_found']:.4f} {sc['bands']}, {len(q) / qs:,.0f} q/s",
                    flush=True,
                )
        del index
        gc.collect()
    return rows


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("stages", nargs="*", default=["gen", "check", "exact", "usearch", "faiss"])
    ap.add_argument("--n", type=int, default=5_000_000)
    ap.add_argument("--queries", type=int, default=10_000)
    ap.add_argument("--usearch-dtype", nargs="+", default=["i8"])
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    p = paths(args.n)
    results_path = OUT / f"bench_ann_{p['base.f16'].name.split('_')[0]}.json"
    results = json.loads(results_path.read_text(encoding="utf-8")) if results_path.exists() else {}
    if "gen" in args.stages:
        stage_gen(args.n, args.queries, p)
    if "check" in args.stages:
        results["check"] = stage_check()
    rows = [r for r in results.get("rows", []) if not any(r["index"].startswith(s) for s in args.stages)]
    if {"exact", "usearch", "faiss"} & set(args.stages):
        exact = stage_exact(p)
        partners = np.load(p["partners.npy"])
        sc = score(exact["ids"], exact, partners, partner_cos(p))
        results["exact"] = {"pairs_per_s": exact["rate"], "hours_70m_all_pairs": 70e6**2 / exact["rate"] / 3600, **sc}
        print(f"exact: partners ≥ {FLOOR} in the top {K}: {sc['partners_found']:.4f} {sc['bands']}", flush=True)
        if "usearch" in args.stages:
            for dt in args.usearch_dtype:
                rows += stage_usearch(p, exact, dt)
        if "faiss" in args.stages:
            rows += stage_faiss(p, exact)
    results["rows"] = rows
    results_path.write_text(json.dumps(results, indent=1), encoding="utf-8")
    print(f"wrote {results_path}")


if __name__ == "__main__":
    main()
