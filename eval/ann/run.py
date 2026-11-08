"""ANN bake-off on real embeddings: exact GPU self-join vs tuned approximate methods (PLAN §9 decision 5).

Usage (from eval/, with PG_DATA_ROOT=D:\\PhotoGallery\\datasets PG_WORK_DIR=D:\\PhotoGallery\\eval_ann):
    python -m ann.run validate charact exact tune pca finalists verify report

Stages, each cached in CACHE_DIR/ann (results_*.jsonl lines are skipped when present):
- validate: the harness checks itself; tune refuses to run until every gate passes (validate.json):
  1. SIFT1M: FAISS reproduces its published recall (benchs/bench_hnsw.py settings), USearch is in range;
  2. ceiling: at 25k (VALIDATE), every non-quantized method's widest setting finds >= 0.999 of the required pairs;
  3. the GPU exact join equals float64 on 1,000 random and the 1,000 densest rows.
- charact: data structure, base vs a Gaussian control (charact.py).
- exact: ground truth for every nested size, and the fp16 exact join's speed.
- tune: every method's grid on the main base (MAIN = 50k nested subset); every setting is scored on both pair halves.
- pca: PCA-256 in front of each family's cheapest setting that reaches 0.999 on the tuning half.
- finalists: per family and target (0.99 / 0.995 / 0.999 on the tuning half), the cheapest setting, rebuilt 3 times
  (other insertion orders and seeds), and its trend over the smaller nested sizes (SIZES).
- verify: writes WORK_DIR/pairs_knn.csv (a stratified sample of the exact candidates, and the 0.999 finalists'
  misses) and runs verify.py on it, which writes verify_knn_<model>.csv (the viewer's k-NN tab).
- report: tables (report.md, report.json).
"""

import json
import subprocess
import sys
import tarfile
import time
import urllib.request
from pathlib import Path

import numpy as np

from ann import charact, exact, methods, metrics
from ann.data import KMAX, MAIN, MODEL, OUT, SIZES, VALIDATE, Base, load_base, size_tag
from common import WORK_DIR

KPRIMES = (10, 20, 40, 80)
EFS = (16, 32, 64, 128, 256, 512, 1024, 2048, 4096)
TARGETS = (0.99, 0.995, 0.999)
SIFT_URL = "ftp://ftp.irisa.fr/local/texmex/corpus/sift.tar.gz"
# FAISS wiki "Indexing 1M vectors", benchs/bench_hnsw.py on SIFT1M: R@1 per efSearch / nprobe.
SIFT_REF = {
    "HNSW32,Flat": {16: 0.8740, 32: 0.9492, 64: 0.9779, 128: 0.9887, 256: 0.9920},
    "IVF16384,Flat": {1: 0.4085, 4: 0.6331, 16: 0.8263, 64: 0.9470, 256: 0.9861},
}
SIFT_TOL = 0.015


def powers(limit: int) -> list[int]:
    return [p for p in (1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096) if p <= limit]


def nlists(n: int) -> list[int]:
    """sqrt(N), 4 sqrt(N), 16 sqrt(N), as powers of two."""
    root = int(2 ** round(np.log2(np.sqrt(n))))
    return [root, root * 4, root * 16]


# --- method grid -----------------------------------------------------------------------------------------------


def grid(n: int) -> list[dict]:
    """Every configuration: family, config (build settings), params (search settings swept on one build)."""
    out = [{"family": "exact", "config": {}, "params": [{}]}]
    for nl in nlists(n):
        out.extend(
            {"family": "gpu_ivf", "config": {"nlist": nl, "r": r}, "params": [{"nprobe": p} for p in powers(min(nl, 256))]}
            for r in (1, 2, 4)
        )
    for m in (16, 32, 64):
        for ef_add in (128, 256, 512):
            for dtype in ("f32", "f16", "i8"):
                cfg = {"M": m, "ef_add": ef_add, "dtype": dtype}
                out.append({"family": "usearch", "config": cfg, "params": [{"ef": e} for e in EFS]})
    for m in (32, 64):
        out.extend(
            {
                "family": "faiss_hnsw",
                "config": {"spec": f"HNSW{m},Flat", "efC": efc},
                "params": [{"efSearch": e} for e in EFS],
            }
            for efc in (256, 512)
        )
    for nl in nlists(n):
        out.extend(
            {
                "family": "faiss_ivf",
                "config": {"spec": f"IVF{nl},{enc}", "centroids": True},
                "params": [{"nprobe": p} for p in powers(nl)],
            }
            for enc in ("Flat", "SQ8", "PQ64", "PQ32x4fs", "PQ64x4fs")
        )
        out.append(
            {"family": "faiss_ivf", "config": {"spec": f"OPQ64,IVF{nl},PQ64"}, "params": [{"nprobe": p} for p in powers(nl)]}
        )
    nl = nlists(n)[-1]
    out.append(
        {
            "family": "faiss_ivf",
            "config": {"spec": f"IVF{nl}_HNSW32,Flat", "centroids": True},
            "params": [{"nprobe": p, "quantizer_efSearch": 128} for p in powers(nl)],
        }
    )
    for k in (32, 64):
        out.extend({"family": "nndescent", "config": {"K": k, "iter": it}, "params": [{}]} for it in (10, 20))
    return out


def scale_config(c: dict, n_full: int, n: int) -> dict:
    """The same configuration for a smaller base: list counts keep their multiple of sqrt(N), search settings that
    exceed the new list count are dropped."""
    c = json.loads(json.dumps(c))
    old, new = nlists(n_full), nlists(n)
    cfg = c["config"]
    if "nlist" in cfg:
        cfg["nlist"] = new[old.index(cfg["nlist"])]
        limit = cfg["nlist"]
    elif "IVF" in cfg.get("spec", ""):
        head = cfg["spec"].split("IVF")[1]
        nl = int(head.split(",")[0].split("_")[0])
        cfg["spec"] = cfg["spec"].replace(f"IVF{nl}", f"IVF{new[old.index(nl)]}")
        limit = new[old.index(nl)]
    else:
        return c
    c["params"] = [p for p in c["params"] if p.get("nprobe", 0) <= limit]
    return c


def config_name(c: dict, tag: str) -> str:
    return f"{tag}|{c['family']}|{json.dumps(c['config'], sort_keys=True)}"


def base_file(base: Base, size: int | None, pca_dim: int = 0) -> Path:
    path = OUT / f"base_{size_tag(size)}{f'_pca{pca_dim}' if pca_dim else ''}.npy"
    if not path.exists():
        x = base.x[base.subset(size)]
        np.save(path, methods.pca(x, pca_dim) if pca_dim else x.astype(np.float16))
    return path


def centroids_file(base: Base, size: int | None, nlist: int) -> Path:
    path = OUT / f"centroids_{size_tag(size)}_{nlist}.npy"
    if not path.exists():
        np.save(path, methods.kmeans(base.x[base.subset(size)], nlist))
    return path


def run_config(base: Base, size: int | None, c: dict, pca_dim: int = 0, **job) -> tuple[dict, list[dict]]:
    """Builds one configuration and runs its searches: (build info, [{param, search_s, ids}])."""
    x = base.x[base.subset(size)]
    fam, cfg = c["family"], c["config"]
    if pca_dim and fam in ("gpu_ivf", "exact"):
        raise ValueError("PCA applies to the CPU indexes")
    if fam == "exact":
        _, ids, seconds = exact.topk(x, KMAX, fp16=True)
        return {"build_s": 0.0, "index_bytes": 0}, [{"param": {}, "search_s": seconds, "ids": ids}]
    if fam == "gpu_ivf":
        return methods.gpu_ivf(x, cfg["nlist"], cfg["r"], [p["nprobe"] for p in c["params"]], seed=job.get("seed", 0))
    bf = base_file(base, size, pca_dim)
    worker_cfg = {k: v for k, v in cfg.items() if k != "centroids"}
    if fam in ("faiss_hnsw", "faiss_ivf"):
        if cfg.get("centroids") and not pca_dim:
            job["centroids"] = centroids_file(base, size, int(cfg["spec"].split(",")[0].split("_")[0][3:]))
        family = "faiss"
    else:
        family = fam
    tag = f"{size_tag(size)}_{fam}_{abs(hash(json.dumps(cfg, sort_keys=True))) % 10**8}"
    return methods.run_cpu(tag, bf, family, worker_cfg, c["params"], **job)


def score_runs(t: metrics.Truth, x: np.ndarray, name: str, c: dict, build: dict, runs: list[dict], extra: dict) -> list[dict]:
    lines = []
    for r in runs:
        ids, sims = metrics.rescore(x, r["ids"])
        for kp in KPRIMES:
            if kp > r["ids"].shape[1]:
                continue
            s = metrics.score(t, ids, sims, kp)
            lines.append(
                {
                    "name": name,
                    "family": c["family"],
                    "config": c["config"],
                    "param": r["param"],
                    "build_s": build.get("build_s"),
                    "search_s": r["search_s"],
                    "cost_s": (build.get("build_s") or 0) + r["search_s"],
                    "index_bytes": build.get("index_bytes"),
                    "peak_rss_bytes": build.get("peak_rss_bytes"),
                    "stopped": build.get("stopped"),
                    **extra,
                    **s,
                }
            )
    return lines


def results_path(stage: str) -> Path:
    return OUT / f"results_{stage}.jsonl"


def done_names(stage: str) -> set[str]:
    p = results_path(stage)
    return {json.loads(line)["name"] for line in p.read_text(encoding="utf-8").splitlines()} if p.exists() else set()


def append(stage: str, lines: list[dict]) -> None:
    with results_path(stage).open("a", encoding="utf-8") as f:
        f.writelines(json.dumps(line) + "\n" for line in lines)


def load_results(stage: str) -> list[dict]:
    p = results_path(stage)
    return [json.loads(line) for line in p.read_text(encoding="utf-8").splitlines()] if p.exists() else []


# --- stages --------------------------------------------------------------------------------------------------


def fvecs(path: Path) -> np.ndarray:
    raw = np.fromfile(path, dtype=np.int32)
    d = raw[0]
    return raw.reshape(-1, d + 1)[:, 1:].copy().view(np.float32)


def stage_validate(base: Base) -> None:
    out: dict = {}
    # Gate 1: SIFT1M.
    sift = OUT / "sift"
    if not (sift / "sift_base.npy").exists():
        sift.mkdir(parents=True, exist_ok=True)
        tgz = sift / "sift.tar.gz"
        if not tgz.exists():
            print(f"downloading {SIFT_URL}", flush=True)
            urllib.request.urlretrieve(SIFT_URL, tgz)  # noqa: S310 (SIFT_URL is a fixed ftp constant)
        with tarfile.open(tgz) as tf:
            tf.extractall(sift, filter="data")
        for name in ("base", "query", "learn"):
            np.save(sift / f"sift_{name}.npy", fvecs(sift / "sift" / f"sift_{name}.fvecs"))
        np.save(
            sift / "sift_gt.npy", np.fromfile(sift / "sift" / "sift_groundtruth.ivecs", dtype=np.int32).reshape(-1, 101)[:, 1:]
        )
    gt1 = np.load(sift / "sift_gt.npy")[:, 0]
    common = {"queries": sift / "sift_query.npy", "metric": "l2"}
    gate1 = {}
    for spec, ref in SIFT_REF.items():
        hnsw = spec.startswith("HNSW")
        params = [{"efSearch": e} for e in ref] if hnsw else [{"nprobe": p} for p in ref]
        cfg = {"spec": spec, "efC": 40} if hnsw else {"spec": spec}
        job = dict(common, **({} if hnsw else {"train": sift / "sift_learn.npy"}))
        _, runs = methods.run_cpu(f"sift_{spec.split(',')[0]}", sift / "sift_base.npy", "faiss", cfg, params, **job)
        got = {next(iter(r["param"].values())): float((r["ids"][:, 0] == gt1).mean()) for r in runs}
        gate1[spec] = {str(k): [round(got[k], 4), v] for k, v in ref.items()}
        gate1[spec + " pass"] = all(abs(got[k] - v) <= SIFT_TOL for k, v in ref.items())
    _, runs = methods.run_cpu(
        "sift_usearch",
        sift / "sift_base.npy",
        "usearch",
        {"M": 32, "ef_add": 40, "dtype": "f32"},
        [{"ef": e} for e in SIFT_REF["HNSW32,Flat"]],
        **common,
    )
    gate1["usearch M32 ef_add 40 (informational)"] = {
        str(r["param"]["ef"]): round(float((r["ids"][:, 0] == gt1).mean()), 4) for r in runs
    }
    out["gate1_sift1m"] = gate1
    print(json.dumps(gate1, indent=1), flush=True)

    # Gate 2: the ceiling at 25k.
    size = VALIDATE
    sub = base.subset(size)
    t = metrics.truth(base, sub, exact.ground_truth(base, size))
    x = base.x[sub]
    nl = nlists(len(sub))[0]
    widest = [
        {"family": "gpu_ivf", "config": {"nlist": nl, "r": 1}, "params": [{"nprobe": nl}]},
        {"family": "usearch", "config": {"M": 32, "ef_add": 256, "dtype": "f32"}, "params": [{"ef": 4096}]},
        {"family": "faiss_hnsw", "config": {"spec": "HNSW32,Flat", "efC": 256}, "params": [{"efSearch": 4096}]},
        {"family": "faiss_ivf", "config": {"spec": f"IVF{nl},Flat", "centroids": True}, "params": [{"nprobe": nl}]},
        {"family": "nndescent", "config": {"K": 64, "iter": 20}, "params": [{}]},
    ]
    gate2 = {}
    for c in widest:
        _, runs = run_config(base, size, c)
        ids, sims = metrics.rescore(x, runs[0]["ids"])
        s = metrics.score(t, ids, sims, min(KMAX, runs[0]["ids"].shape[1]))
        both = (s["recall_tune"] + s["recall_eval"]) / 2
        gate2[f"{c['family']} {json.dumps(c['config'])}"] = round(both, 5)
    out["gate2_ceiling_25k"] = gate2
    out["gate2_pass"] = all(v >= 0.999 for v in gate2.values())
    print(json.dumps(gate2, indent=1), flush=True)

    # Gate 3: exact vs float64.
    gt = exact.ground_truth(base, MAIN)
    xa = base.x[base.subset(MAIN)]
    rng = np.random.default_rng(0)
    rows = np.unique(np.concatenate([rng.choice(len(xa), 1000, replace=False), np.argsort(-gt["sims"][:, -1])[:1000]]))
    out["gate3_fp64"] = exact.check_fp64(xa, gt["sims"], gt["ids"], rows)
    out["gate3_pass"] = not out["gate3_fp64"]["mismatched_rows"]
    out["pass"] = bool(all(v for k, v in gate1.items() if k.endswith(" pass")) and out["gate2_pass"] and out["gate3_pass"])
    (OUT / "validate.json").write_text(json.dumps(out, indent=1), encoding="utf-8")
    print(f"validation {'PASSED' if out['pass'] else 'FAILED'}: {OUT / 'validate.json'}", flush=True)


def require_validation() -> None:
    p = OUT / "validate.json"
    if not p.exists() or not json.loads(p.read_text(encoding="utf-8"))["pass"]:
        raise SystemExit("validation has not passed (python -m ann.run validate)")


def stage_charact(base: Base) -> None:
    out = charact.characterize(base, exact.ground_truth)
    (OUT / "charact.json").write_text(json.dumps(out, indent=1), encoding="utf-8")
    print(json.dumps(out, indent=1), flush=True)


def stage_exact(base: Base) -> None:
    for size in SIZES:
        gt = exact.ground_truth(base, size)
        print(
            f"exact {size_tag(size)}: {len(gt['ids']):,} rows in {float(gt['seconds']):.1f} s; {len(gt['hub_rows'])} hubs",
            flush=True,
        )
    x = base.x[base.subset(MAIN)]
    rate = exact.fp16_rate(x)
    (OUT / "exact_speed.json").write_text(json.dumps({"fp16_pairs_per_s": rate}), encoding="utf-8")
    print(f"exact fp16 join: {rate / 1e9:.1f} G pairs/s", flush=True)


def stage_tune(base: Base, configs: list[dict] | None = None, stage: str = "tune", pca_dim: int = 0) -> None:
    require_validation()
    sub = base.subset(MAIN)
    t = metrics.truth(base, sub, exact.ground_truth(base, MAIN))
    x = base.x[sub]
    done = done_names(stage)
    configs = configs if configs is not None else grid(len(sub))
    for i, c in enumerate(configs):
        name = config_name(c, f"{size_tag(MAIN)}{f'_pca{pca_dim}' if pca_dim else ''}")
        if name in done:
            continue
        start = time.perf_counter()
        try:
            build, runs = run_config(base, MAIN, c, pca_dim=pca_dim)
        except Exception as e:  # noqa: BLE001 (one failing configuration is recorded, not fatal)
            append(stage, [{"name": name, "family": c["family"], "config": c["config"], "error": str(e)[:500]}])
            print(f"[{i + 1}/{len(configs)}] {name}: FAILED {str(e)[:200]}", flush=True)
            continue
        lines = score_runs(t, x, name, c, build, runs, {"pca": pca_dim})
        append(stage, lines)
        best = max((ln["recall_tune"] for ln in lines), default=float("nan"))
        print(f"[{i + 1}/{len(configs)}] {name}: {time.perf_counter() - start:.0f} s, best tune recall {best:.5f}", flush=True)


def ok_lines(stage: str) -> list[dict]:
    return [ln for ln in load_results(stage) if "error" not in ln]


def cheapest(lines: list[dict], family: str, target: float) -> dict | None:
    hits = [ln for ln in lines if ln["family"] == family and ln["recall_tune"] >= target]
    return min(hits, key=lambda ln: ln["cost_s"]) if hits else None


def stage_pca(base: Base) -> None:
    lines = ok_lines("tune")
    configs = []
    for fam in ("usearch", "faiss_hnsw", "faiss_ivf", "nndescent"):
        best = cheapest(lines, fam, TARGETS[-1]) or max(
            (ln for ln in lines if ln["family"] == fam), key=lambda ln: ln["recall_tune"], default=None
        )
        if best:
            cfg = dict(best["config"])
            configs.append(
                {
                    "family": fam,
                    "config": cfg,
                    "params": [p["param"] for p in lines if p["name"] == best["name"] and p["kprime"] == best["kprime"]],
                }
            )
    stage_tune(base, configs, stage="pca", pca_dim=256)


def stage_finalists(base: Base) -> None:
    lines = ok_lines("tune")
    done = done_names("finalists")
    families = sorted({ln["family"] for ln in lines} - {"exact"})
    for fam in families:
        for target in TARGETS:
            best = cheapest(lines, fam, target)
            if best is None:
                print(f"{fam}: nothing reaches {target}", flush=True)
                continue
            c = {"family": fam, "config": best["config"], "params": [best["param"]]}
            # Rebuilds on the full base: other insertion orders (CPU graphs) / seeds (k-means).
            for rep in (1, 2, 3):
                name = f"rep{rep}|{target}|" + config_name(c, size_tag(MAIN))
                if name in done:
                    continue
                job: dict = {"seed": rep}
                if fam in ("usearch", "faiss_hnsw", "nndescent"):
                    job["order"] = "shuffled"
                build, runs = run_config(base, MAIN, c, **job)
                sub = base.subset(MAIN)
                t = metrics.truth(base, sub, exact.ground_truth(base, MAIN))
                ls = [
                    ln
                    for ln in score_runs(
                        t, base.x[sub], name, c, build, runs, {"target": target, "rep": rep, "size": size_tag(MAIN)}
                    )
                    if ln["kprime"] == best["kprime"]
                ]
                append("finalists", ls)
                print(f"{name}: eval recall {ls[0]['recall_eval']:.5f}", flush=True)
            # Trend over the nested sizes, the search setting swept again at each size.
            n_full = len(base.subset(MAIN))
            sweep = next((g for g in grid(n_full) if g["family"] == fam and g["config"] == best["config"]), c)
            for size in (s for s in SIZES if s != MAIN):
                name = f"size|{target}|{size_tag(size)}|" + config_name(c, size_tag(MAIN))
                if name in done:
                    continue
                sub = base.subset(size)
                t = metrics.truth(base, sub, exact.ground_truth(base, size))
                scaled = scale_config(sweep, n_full, len(sub))
                build, runs = run_config(base, size, scaled)
                extra = {"target": target, "size": size_tag(size)}
                ls = [ln for ln in score_runs(t, base.x[sub], name, scaled, build, runs, extra) if ln["kprime"] == best["kprime"]]
                append("finalists", ls)
                print(f"{name}: done", flush=True)


VERIFY_PER_STRATUM = 250
VERIFY_MISSES = 400


def stage_verify(base: Base) -> None:
    """The pair list is made once: the 0.999 settings' misses come from rebuilt indexes, and multithreaded graph
    builds aren't deterministic, so a rerun would list other pairs, while verify.py's checkpoint resumes by position
    in the list. pairs_knn.csv is written last (atomically), so it marks a complete list."""
    if not (WORK_DIR / "pairs_knn.csv").exists() or not (OUT / "verify_meta.json").exists():
        prepare_verify(base)
    else:
        print(f"reusing the pair list {WORK_DIR / 'pairs_knn.csv'}", flush=True)
    subprocess.run(
        [
            sys.executable,
            str(Path(__file__).parents[1] / "verify.py"),
            "--pairs",
            "listed",
            "--list",
            "knn",
            "--model",
            MODEL,
            "--rotations",
            "45",
        ],
        check=True,
    )


def prepare_verify(base: Base) -> None:
    sub = base.subset(MAIN)
    t = metrics.truth(base, sub, exact.ground_truth(base, MAIN))
    x = base.x[sub]
    rng = np.random.default_rng(0)
    kind = metrics.kind_of(t, t.required)
    band = np.digitize(t.required_sim, metrics.BANDS) - 1
    chosen: dict[int, list[str]] = {}
    for k in range(len(metrics.KINDS)):
        for b in range(len(metrics.BANDS) - 1):
            keys = t.required[(kind == k) & (band == b)]
            for key in rng.choice(keys, min(len(keys), VERIFY_PER_STRATUM), replace=False):
                chosen.setdefault(int(key), []).append("exact_sample")
    lines = ok_lines("tune")
    for fam in sorted({ln["family"] for ln in lines} - {"exact"}):
        best = cheapest(lines, fam, TARGETS[-1])
        if best is None:
            continue
        c = {"family": fam, "config": best["config"], "params": [best["param"]]}
        _, runs = run_config(base, MAIN, c)
        ids, sims = metrics.rescore(x, runs[0]["ids"])
        miss = metrics.missed(t, ids, sims, best["kprime"])
        np.save(OUT / f"misses_{fam}.npy", miss)  # all of them: the report scales the verified sample up
        for key in rng.choice(miss, min(len(miss), VERIFY_MISSES), replace=False):
            chosen.setdefault(int(key), []).append(f"miss:{fam}")
        print(f"{fam}: {len(miss)} required pairs missed at its 0.999 setting", flush=True)

    keys = np.array(sorted(chosen), dtype=np.int64)
    a, b = keys // t.n, keys % t.n
    pk = metrics.kind_of(t, keys)
    label = np.where(pk == 0, "planted", np.where(pk == 2, "duplicate", "unlabeled"))
    cross = (t.group[a] >= 0) & (t.group[b] >= 0) & (t.group[a] != t.group[b])
    label = np.where(cross, "cross_group", label)
    rows_a, rows_b = base.rows[sub[a]], base.rows[sub[b]]
    sims_ab = np.einsum("ij,ij->i", x[a], x[b])
    meta = [
        {"key": int(k), "sources": chosen[int(k)], "kind": metrics.KINDS[int(pk[i])], "sim": float(sims_ab[i])}
        for i, k in enumerate(keys)
    ]
    (OUT / "verify_meta.json").write_text(json.dumps(meta), encoding="utf-8")
    tmp = WORK_DIR / "pairs_knn.csv.tmp"
    with tmp.open("w", encoding="utf-8") as f:
        f.write("a,b,label\n")
        f.writelines(f"{ra},{rb},{lab}\n" for ra, rb, lab in zip(rows_a, rows_b, label, strict=True))
    tmp.replace(WORK_DIR / "pairs_knn.csv")
    print(f"{len(keys)} pairs to verify -> {WORK_DIR / 'pairs_knn.csv'}", flush=True)


def stage_report(base: Base) -> None:
    from ann import report

    report.write(base)


STAGES = {
    "validate": stage_validate,
    "charact": stage_charact,
    "exact": stage_exact,
    "tune": stage_tune,
    "pca": stage_pca,
    "finalists": stage_finalists,
    "verify": stage_verify,
    "report": stage_report,
}


def main() -> None:
    stages = sys.argv[1:] or list(STAGES)
    unknown = [s for s in stages if s not in STAGES]
    if unknown:
        raise SystemExit(f"unknown stages {unknown}; choose from {list(STAGES)}")
    OUT.mkdir(parents=True, exist_ok=True)
    base = load_base()
    print(f"base: {len(base.x):,} rows ({int((base.planted_edit >= 0).sum()):,} planted)", flush=True)
    for s in stages:
        print(f"== {time.strftime('%H:%M:%S')} {s}", flush=True)
        STAGES[s](base)


if __name__ == "__main__":
    main()
