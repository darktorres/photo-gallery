"""Builds one CPU index and searches it; runs in WSL2 (methods.run_cpu), whose pip wheels of FAISS (AVX2) and
USearch (SimSIMD) have the SIMD kernels the Windows wheels lack. Only numpy, faiss and usearch: no torch, no eval
imports.

Usage: python cpu_worker.py <job.json>

Job keys:
- base: .npy of vectors (float16 or float32); queries: .npy or null for a self-join (every base row queries)
- metric: "ip" (unit vectors: cosine) or "l2"
- family: "usearch", "faiss" or "nndescent"; config: that family's build settings (see build_*)
- params: the search settings to run, one search each (ef / efSearch / nprobe); k: results per query
- order: "file" (rows added in file order, as an import would) or "shuffled" (seeded by "seed")
- train: .npy of training vectors, or null to train on a sample of the base; centroids: .npy of IVF centroids to
  use instead of training the coarse quantizer
- out: folder for ids_<param>.npy (int32, -1 = no result) and result.json (times, memory, peak RSS)
- converged (default 1e-4), max_search_s (default none): stop widening the search once fewer than this share of
  rows change their top 10, or once the sweep's total search time passes this
"""

import json
import resource
import sys
import time
from pathlib import Path

import numpy as np

THREADS = 12


def load(path: str) -> np.ndarray:
    return np.ascontiguousarray(np.load(path), dtype=np.float32)


def build_usearch(x: np.ndarray, metric: str, config: dict, order: np.ndarray):
    from usearch.index import Index

    index = Index(
        ndim=x.shape[1],
        metric={"ip": "ip", "l2": "l2sq"}[metric],
        dtype=config["dtype"],
        connectivity=config["M"],
        expansion_add=config["ef_add"],
    )
    index.add(order, x[order], threads=THREADS)
    return index


def search_usearch(index, q: np.ndarray, k: int, ef: int) -> np.ndarray:
    index.expansion_search = ef
    m = index.search(q, k, threads=THREADS)
    keys = np.asarray(m.keys, dtype=np.int64)
    valid = np.arange(keys.shape[1])[None, :] < np.asarray(m.counts)[:, None]
    return np.where(valid, keys, -1).astype(np.int32)


def build_faiss(x: np.ndarray, metric: str, config: dict, order: np.ndarray, train, centroids):
    import faiss

    faiss.omp_set_num_threads(THREADS)
    mt = faiss.METRIC_INNER_PRODUCT if metric == "ip" else faiss.METRIC_L2
    index = faiss.index_factory(x.shape[1], config["spec"], mt)
    base = faiss.downcast_index(index.index) if isinstance(index, faiss.IndexPreTransform) else index
    if isinstance(base, faiss.IndexHNSW):
        base.hnsw.efConstruction = config.get("efC", 40)
    if centroids is not None:
        ivf = faiss.extract_index_ivf(index)
        ivf.quantizer.add(centroids)  # a trained quantizer with nlist entries: train() leaves it alone
    if not index.is_trained:
        index.train(train if train is not None else x[np.random.default_rng(0).permutation(len(x))[:300_000]])
    # FAISS assigns ids in insertion order, so add in the requested order and map back.
    index.add(x[order])
    return index, order


def search_faiss(built, q: np.ndarray, k: int, param: dict) -> np.ndarray:
    import faiss

    index, order = built
    ps = faiss.ParameterSpace()
    for name, value in param.items():
        if name == "quantizer_efSearch":
            faiss.downcast_index(faiss.extract_index_ivf(index).quantizer).hnsw.efSearch = value
        elif name == "bounded_queue":
            faiss.downcast_index(index).hnsw.search_bounded_queue = bool(value)
        else:
            ps.set_index_parameter(index, name, value)
    _, ids = index.search(q, k)
    return np.where(ids >= 0, order[np.clip(ids, 0, None)], -1).astype(np.int32)


def nndescent_graph(x: np.ndarray, metric: str, config: dict, order: np.ndarray) -> np.ndarray:
    import faiss

    faiss.omp_set_num_threads(THREADS)
    mt = faiss.METRIC_INNER_PRODUCT if metric == "ip" else faiss.METRIC_L2
    index = faiss.IndexNNDescentFlat(x.shape[1], config["K"], mt)
    index.nndescent.iter = config.get("iter", 10)
    index.nndescent.S = config.get("S", 10)
    index.nndescent.R = config.get("R", 100)
    index.nndescent.L = config.get("L", config["K"] + 50)
    index.add(x[order])
    graph = faiss.vector_to_array(index.nndescent.final_graph).reshape(len(x), -1)[:, : config["K"]]
    return np.where(graph >= 0, order[np.clip(graph, 0, None)], -1)[np.argsort(order)].astype(np.int32)


def main() -> None:
    job = json.loads(Path(sys.argv[1]).read_text())
    out = Path(job["out"])
    out.mkdir(parents=True, exist_ok=True)
    x = load(job["base"])
    q = load(job["queries"]) if job.get("queries") else x
    order = np.arange(len(x))
    if job.get("order") == "shuffled":
        order = np.random.default_rng(job.get("seed", 0)).permutation(len(x))
    train = load(job["train"]) if job.get("train") else None
    centroids = load(job["centroids"]) if job.get("centroids") else None
    family, config, k = job["family"], job["config"], job["k"]
    result: dict = {"family": family, "config": config, "searches": []}

    t = time.perf_counter()
    built = None
    if family == "usearch":
        built = build_usearch(x, job["metric"], config, order)
        result["index_bytes"] = int(built.memory_usage)
    elif family == "faiss":
        import faiss

        built = build_faiss(x, job["metric"], config, order, train, centroids)
        result["index_bytes"] = int(faiss.serialize_index(built[0]).size)
    elif family == "nndescent":
        graph = nndescent_graph(x, job["metric"], config, order)
        result["build_s"] = time.perf_counter() - t
        np.save(out / "ids_graph.npy", graph)
        result["searches"].append({"param": {}, "search_s": 0.0, "file": "ids_graph.npy"})
    else:
        raise SystemExit(f"unknown family {family}")
    result.setdefault("build_s", time.perf_counter() - t)

    previous, spent = None, 0.0
    for i, param in enumerate(job.get("params", []) if built is not None else []):
        t = time.perf_counter()
        if family == "usearch":
            ids = search_usearch(built, q, min(k, param["ef"]), param["ef"])
        else:
            kk = min(k, param["efSearch"]) if "efSearch" in param else k
            ids = search_faiss(built, q, kk, param)
        seconds = time.perf_counter() - t
        name = f"ids_{i}.npy"
        result["searches"].append({"param": param, "search_s": seconds, "file": name})
        np.save(out / name, ids)
        # The params widen the search in order. Stop when it has converged (the top 10 of almost no row changed
        # from the previous width) or when the sweep's total search time passes the cap: wider settings only cost
        # more. Neither rule looks at the ground truth.
        spent += seconds
        top = np.sort(ids[:, :10], axis=1)
        if previous is not None and (top != previous).any(axis=1).mean() < job.get("converged", 1e-4):
            result["stopped"] = f"converged after {param}"
            break
        if spent > job.get("max_search_s", float("inf")):
            result["stopped"] = f"time cap after {param} ({spent:.0f} s of search)"
            break
        previous = top
    result["peak_rss_bytes"] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * 1024
    result["base_bytes"] = int(x.nbytes)
    (out / "result.json").write_text(json.dumps(result))


if __name__ == "__main__":
    main()
