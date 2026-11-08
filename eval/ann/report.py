"""The bake-off's tables (CACHE_DIR/ann/report.md and report.json), from the stages' cached results.

True-duplicate recall: the verifier's match rate on a stratified sample of the exact candidates (stratum = pair
kind x cosine band) estimates the true duplicates among the required pairs; its match rate on a sample of a
method's misses estimates the true duplicates it loses. Recall = 1 - lost / total, with a conservative 95% bound
from Clopper-Pearson intervals (the misses' rate at its upper bound, the candidates' at their lower).
"""

import csv
import json

import numpy as np

from ann import exact, metrics
from ann.data import MAIN, MODEL, OUT, Base
from common import WORK_DIR

TARGETS = (0.99, 0.995, 0.999)


def load(stage: str) -> list[dict]:
    p = OUT / f"results_{stage}.jsonl"
    if not p.exists():
        return []
    return [ln for ln in map(json.loads, p.read_text(encoding="utf-8").splitlines()) if "error" not in ln]


def describe(ln: dict) -> str:
    cfg = ", ".join(f"{k} {v}" for k, v in ln["config"].items() if k != "centroids")
    prm = ", ".join(f"{k} {v}" for k, v in ln["param"].items())
    return f"{cfg}{'; ' if cfg and prm else ''}{prm}; k' {ln['kprime']}" + (f"; PCA {ln['pca']}" if ln.get("pca") else "")


def frontier(lines: list[dict]) -> list[dict]:
    """The Pareto frontier on (cost, tuning-half recall)."""
    out, best = [], -1.0
    for ln in sorted(lines, key=lambda ln: (ln["cost_s"], -ln["recall_tune"])):
        if ln["recall_tune"] > best:
            out.append(ln)
            best = ln["recall_tune"]
    return out


def gb(b) -> str:
    return f"{b / 2**30:.2f}" if b else "–"


def true_duplicate_recall(base: Base) -> dict:
    """Per family (its 0.999 setting): estimated true-duplicate recall, and its conservative bound."""
    csv_path = WORK_DIR / f"verify_knn_{MODEL}.csv"
    meta_path = OUT / "verify_meta.json"
    if not csv_path.exists() or not meta_path.exists():
        return {}
    meta = json.loads(meta_path.read_text(encoding="utf-8"))
    with open(csv_path, newline="", encoding="utf-8") as f:
        decision = {int(r["id"]): r["decision"] for r in csv.DictReader(f)}
    sub = base.subset(MAIN)
    t = metrics.truth(base, sub, exact.ground_truth(base, MAIN))
    band_of = lambda s: int(np.digitize(s, metrics.BANDS) - 1)  # noqa: E731

    # Match rate per stratum among the exact candidates' sample.
    strata: dict[tuple, list[int]] = {}
    for pid, m in enumerate(meta):
        if "exact_sample" in m["sources"] and decision.get(pid) in ("match", "unrelated"):
            strata.setdefault((m["kind"], band_of(m["sim"])), []).append(decision[pid] == "match")
    kind = metrics.kind_of(t, t.required)
    band = np.digitize(t.required_sim, metrics.BANDS) - 1
    total = total_lo = 0.0
    for (k, b), v in strata.items():
        count = int(((kind == metrics.KINDS.index(k)) & (band == b)).sum())
        total += count * np.mean(v)
        total_lo += count * metrics.clopper_pearson(int(sum(v)), len(v))[0]

    out: dict = {"true_duplicates_estimated": round(total, 1)}
    for p in sorted(OUT.glob("misses_*.npy")):
        fam = p.stem[len("misses_") :]
        miss = np.load(p)
        sample = [
            decision[pid] == "match"
            for pid, m in enumerate(meta)
            if f"miss:{fam}" in m["sources"] and decision.get(pid) in ("match", "unrelated")
        ]
        if not sample:
            continue
        rate = float(np.mean(sample))
        hi = metrics.clopper_pearson(int(sum(sample)), len(sample))[1]
        lost, lost_hi = len(miss) * rate, len(miss) * hi
        out[fam] = {
            "missed_pairs": len(miss),
            "verified_sample": len(sample),
            "match_rate_of_misses": round(rate, 4),
            "true_duplicates_lost": round(lost, 1),
            "true_duplicate_recall": round(1 - lost / total, 5) if total else None,
            "conservative_bound": round(1 - lost_hi / total_lo, 5) if total_lo else None,
        }
    return out


def write(base: Base) -> None:
    tune, pca, fin = load("tune"), load("pca"), load("finalists")
    md = ["# ANN bake-off report", ""]
    for name in ("validate", "exact_speed", "charact"):
        p = OUT / f"{name}.json"
        if p.exists():
            md += [f"## {name}", "", "```json", p.read_text(encoding="utf-8"), "```", ""]

    md += ["## Pareto frontiers (full base; settings chosen on the tuning half, recall reported on both)", ""]
    md += [
        "| Family | Setting | Cost (s) | Memory (GB) | Recall tune | Recall eval | Radius | Recall@10 |",
        "|---|---|---|---|---|---|---|---|",
    ]
    report: dict = {"frontiers": {}}
    for fam in sorted({ln["family"] for ln in tune + pca}):
        f = frontier([ln for ln in tune + pca if ln["family"] == fam])
        report["frontiers"][fam] = f
        for ln in f:
            mem = ln.get("index_bytes") or ln.get("peak_rss_bytes")
            md.append(
                f"| {fam} | {describe(ln)} | {ln['cost_s']:.1f} | {gb(mem)} | {ln['recall_tune']:.5f} | "
                f"{ln['recall_eval']:.5f} | {ln['recall_radius']:.5f} | {ln['recall_at_10']:.4f} |"
            )
    md.append("")

    md += [
        "## Finalists: rebuilt 3 times (eval half)",
        "",
        "| Family | Target | Setting | Eval recall min / mean | Cost mean (s) |",
        "|---|---|---|---|---|",
    ]
    for key in sorted({(ln["family"], ln["target"]) for ln in fin if "rep" in ln}):
        reps = [ln for ln in fin if "rep" in ln and (ln["family"], ln["target"]) == key]
        r = [ln["recall_eval"] for ln in reps]
        cost = np.mean([ln["cost_s"] for ln in reps])
        md.append(f"| {key[0]} | {key[1]} | {describe(reps[0])} | {min(r):.5f} / {np.mean(r):.5f} | {cost:.1f} |")
    md += [
        "",
        "## Trend over nested sizes (same setting family, search swept; cheapest reaching the target on the tuning half)",
        "",
    ]
    md += ["| Family | Target | Size | Setting | Recall eval | Cost (s) |", "|---|---|---|---|---|---|"]
    for key in sorted({(ln["family"], ln["target"], ln["size"]) for ln in fin if "rep" not in ln}):
        ls = [ln for ln in fin if "rep" not in ln and (ln["family"], ln["target"], ln["size"]) == key]
        hits = [ln for ln in ls if ln["recall_tune"] >= key[1]]
        ln = min(hits, key=lambda ln: ln["cost_s"]) if hits else max(ls, key=lambda ln: ln["recall_tune"])
        setting = describe(ln) + ("" if hits else " (target not reached)")
        md.append(f"| {key[0]} | {key[1]} | {key[2]} | {setting} | {ln['recall_eval']:.5f} | {ln['cost_s']:.1f} |")

    md += ["", "## Planted copies found, per edit (each family's cheapest 0.999 setting; exact rule = ceiling)", ""]
    best = {}
    for fam in sorted({ln["family"] for ln in tune}):
        hits = [ln for ln in tune if ln["family"] == fam and ln["recall_tune"] >= TARGETS[-1]]
        if hits:
            best[fam] = min(hits, key=lambda ln: ln["cost_s"])
    if best:
        any_ln = next(iter(best.values()))
        md += ["| Edit | exact rule | " + " | ".join(best) + " |", "|---|---|" + "---|" * len(best)]
        for edit, (_, ceiling) in any_ln["planted"].items():
            md.append(f"| {edit} | {ceiling:.3f} | " + " | ".join(f"{b['planted'][edit][0]:.3f}" for b in best.values()) + " |")
        md += ["", "| dups2 pairs | exact rule | " + " | ".join(best) + " |", "|---|---|" + "---|" * len(best)]
        md.append(
            f"| {any_ln['dups2'][2]} | {any_ln['dups2'][1]:.4f} | "
            + " | ".join(f"{b['dups2'][0]:.4f}" for b in best.values())
            + " |"
        )

    tdr = true_duplicate_recall(base)
    report["true_duplicate_recall"] = tdr
    md += [
        "",
        "## True-duplicate recall (verifier on samples; each family's cheapest 0.999 setting)",
        "",
        "```json",
        json.dumps(tdr, indent=1),
        "```",
        "",
    ]
    (OUT / "report.md").write_text("\n".join(md), encoding="utf-8")
    (OUT / "report.json").write_text(json.dumps(report, indent=1, default=str), encoding="utf-8")
    print("\n".join(md))
