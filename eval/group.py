"""Group images into near-duplicate sets from the verified k-NN graph, and score the groups against the labels.

Usage: python group.py [--model sscd_fp16] [--shortcut-pdq 22] [--shortcut-sim 0.95] [--method multicut|components]

Reads verify.py --pairs knn (verify_knn_<model>.csv). Each candidate pair becomes a signed edge:
- +1: the shortcut accepts it without the verifier (PDQ distance <= --shortcut-pdq with both hashes
  reliable, or SSCD cosine >= --shortcut-sim).
- the verifier's decision when it aligned and compared the pair, weighted by its margin |max_noise - noise| /
  max_noise (0.05-1): positive for a match (near 1 for a clean copy), negative for different content.
- none when it couldn't align the pair: that says nothing, as two crops of one picture that don't overlap
  don't align, yet belong together through the original.

Methods:
- components: connected components of the match edges (union-find). One false match joins two groups, and
  a series of similar pictures chains into one group.
- multicut (default): correlation clustering by greedy additive edge contraction (GAEC; Keuper et al. 2015):
  repeatedly merge the two clusters joined by the largest positive total weight, until none is positive. A
  match is outvoted by the conflicts it would bring into the cluster.

Scored on the labels: dups2/<NNN> groups (different folders = different pictures) and the loose dups2 files
(each has a duplicate among the others). Writes groups_<model>.json (largest group first, each with the
signed edges inside it), browsable on the viewer's Groups page.
"""

import argparse
import csv
import heapq
import json
from collections import Counter, defaultdict, deque
from itertools import combinations

import numpy as np

from common import WORK_DIR, ground_truth, image_list

PDQ_MIN_QUALITY = 50  # Meta: hashes below this quality are unreliable (flat or tiny images)
MIN_EVIDENCE = 0.05  # weight of a verifier decision right at the noise threshold
SHORTCUT_PDQ = 22  # accept without the verifier: PDQ distance <= this ...
SHORTCUT_SIM = 0.95  # ... or SSCD cosine >= this


def pdq_distances(files: list[str]):
    """Plain (upright) PDQ Hamming distance between two images, or None if either hash is unreliable."""
    emb = np.load(WORK_DIR / "emb_pdq.npy").astype(np.float32)
    reliable = np.load(WORK_DIR / "ok_pdq.npy") & (np.load(WORK_DIR / "quality_pdq.npy") >= PDQ_MIN_QUALITY)
    index = {p: i for i, p in enumerate(files)}

    def distance(path_a: str, path_b: str) -> int | None:
        a, b = index[path_a], index[path_b]
        return round(128 * (1 - float(emb[a] @ emb[b]))) if reliable[a] and reliable[b] else None

    return distance


def shortcut(pdq_d: int | None, similarity: float, max_pdq: int, min_sim: float) -> str:
    """Why a candidate pair is accepted without the verifier ("pdq" / "sscd"), or "" if it needs verifying.
    Measured on the k-NN subset (FINDINGS: "Which candidate pairs need the verifier"): PDQ <= 22 matched
    99.5-100%, SSCD >= 0.95 100%, and by eye the shortcut was right on every disagreement."""
    if pdq_d is not None and pdq_d <= max_pdq:
        return "pdq"
    return "sscd" if similarity >= min_sim else ""


def edge_weight(row: dict, max_noise: float) -> float | None:
    """Signed evidence from the verifier, or None if it didn't compare the pair. The sign is its decision; the
    size is the noise margin to the threshold (at least MIN_EVIDENCE), so a pair rejected by another rule
    (motion, interior change) despite low noise counts against grouping as strongly as its margin."""
    if not row.get("noise"):
        return None
    margin = abs(max_noise - float(row["noise"])) / max_noise
    return float(np.clip(max(margin, MIN_EVIDENCE), 0, 1)) * (1 if row["decision"] == "match" else -1)


def components(weights: dict[tuple[str, str], float]) -> list[list[str]]:
    adj: dict[str, set[str]] = defaultdict(set)
    for (a, b), w in weights.items():
        if w > 0:
            adj[a].add(b)
            adj[b].add(a)
    seen: set[str] = set()
    out = []
    for start in adj:
        if start in seen:
            continue
        seen.add(start)
        comp, queue = [], deque([start])
        while queue:
            x = queue.popleft()
            comp.append(x)
            for y in adj[x] - seen:
                seen.add(y)
                queue.append(y)
        out.append(sorted(comp))
    return out


def gaec(weights: dict[tuple[str, str], float]) -> list[list[str]]:
    """Greedy additive edge contraction: merge the cluster pair with the largest positive summed weight
    (parallel edges add up on contraction) until no positive edge is left."""
    members: dict[str, list[str]] = {}
    adj: dict[str, dict[str, float]] = defaultdict(dict)
    for (a, b), w in weights.items():
        for x in (a, b):
            members.setdefault(x, [x])
        adj[a][b] = adj[a].get(b, 0.0) + w
        adj[b][a] = adj[b].get(a, 0.0) + w
    heap = [(-w, a, b) for (a, b), w in weights.items() if w > 0]
    heapq.heapify(heap)
    while heap:
        neg, a, b = heapq.heappop(heap)
        if a not in members or b not in members or adj[a].get(b) != -neg:
            continue  # stale: a cluster was merged away or the weight changed since
        # Merge b into a.
        members[a] += members.pop(b)
        for c, w in adj.pop(b).items():
            del adj[c][b]
            if c == a:
                continue
            total = adj[a].get(c, 0.0) + w
            adj[a][c] = adj[c][a] = total
            if total > 0:
                heapq.heappush(heap, (-total, a, c) if a < c else (-total, c, a))
    return [sorted(m) for m in members.values() if len(m) > 1]


def score(groups: list[list[str]], files: list[str]) -> dict:
    """Groups vs the labels. Labeled folders are different pictures from one another."""
    gt = ground_truth(files)
    folder_of = {files[i]: g for g, members in enumerate(gt.groups) for i in members}
    group_of = {p: g for g, members in enumerate(groups) for p in members}

    tp = fp = fn = 0
    for a, b in combinations(list(folder_of), 2):
        same_true = folder_of[a] == folder_of[b]
        same_pred = a in group_of and group_of.get(a) == group_of.get(b)
        tp += same_true and same_pred
        fp += same_pred and not same_true
        fn += same_true and not same_pred

    # Labeled folder -> number of predicted groups (or singletons) its images end up in.
    spread = [len({group_of.get(files[i], files[i]) for i in members}) for members in gt.groups]
    merged = [members for members in groups if len({folder_of[p] for p in members if p in folder_of}) > 1]

    loose = {files[i] for i in gt.unpaired_pool}
    loose_grouped = sum(any(q in loose and q != p for q in groups[group_of[p]]) for p in loose if p in group_of)
    return {
        "labeled_pairs": {"precision": tp / max(tp + fp, 1), "recall": tp / max(tp + fn, 1), "tp": tp, "fp": fp,
                          "fn": fn},
        "labeled_folders": len(gt.groups),
        "folders_whole": sum(v == 1 for v in spread),
        "folders_split": {str(k): v for k, v in sorted(Counter(spread).items()) if k > 1},
        "groups_merging_folders": [sorted({p.split("/")[1] for p in m if p in folder_of}) for m in merged],
        "loose_files": len(loose),
        "loose_grouped_with_loose": loose_grouped,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", default="sscd_fp16")
    parser.add_argument("--shortcut-pdq", type=int, default=SHORTCUT_PDQ, help="accept PDQ distance <= this (-1 = off)")
    parser.add_argument("--shortcut-sim", type=float, default=SHORTCUT_SIM, help="accept SSCD cosine >= this (> 1 = off)")
    parser.add_argument("--method", choices=("multicut", "components"), default="multicut")
    args = parser.parse_args()

    files = image_list()
    pdq = pdq_distances(files)
    with open(WORK_DIR / f"verify_knn_{args.model}.csv", newline="", encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    max_noise = json.loads((WORK_DIR / f"verify_knn_{args.model}.json").read_text(encoding="utf-8"))["params"][
        "max_noise"]

    weights: dict[tuple[str, str], float] = {}
    kinds: dict[tuple[str, str], str] = {}  # "pdq" / "sscd" (shortcut) / "verifier"
    source = Counter()
    for r in rows:
        a, b = sorted((r["path_a"], r["path_b"]))
        why = shortcut(pdq(a, b), float(r["similarity"]), args.shortcut_pdq, args.shortcut_sim)
        w = 1.0 if why else edge_weight(r, max_noise)
        if w is None:
            source["not aligned (no edge)"] += 1
            continue
        weights[(a, b)], kinds[(a, b)] = w, why or "verifier"
        source[why or ("verifier match" if w > 0 else "verifier conflict")] += 1
        if why and r["decision"] != "match":
            source[f"{why} shortcut overriding a verifier non-match"] += 1
    print(f"{len(rows)} candidate pairs: " + ", ".join(f"{v} {k}" for k, v in source.items()))

    groups = gaec(weights) if args.method == "multicut" else components(weights)
    groups.sort(key=len, reverse=True)
    s = score(groups, files)
    lp = s["labeled_pairs"]
    print(f"\n## {args.method}: {len(groups)} groups of 2+ ({sum(map(len, groups))} images), "
          f"largest {[len(g) for g in groups[:8]]}")
    print(f"  sizes: {dict(sorted(Counter(map(len, groups)).items()))}")
    print(f"  labeled pairs: precision {lp['precision']:.4f} recall {lp['recall']:.4f} (fp {lp['fp']}, fn {lp['fn']})")
    print(f"  labeled folders: {s['folders_whole']}/{s['labeled_folders']} whole, split into n: "
          f"{s['folders_split'] or '-'}; groups merging folders: {s['groups_merging_folders'] or '-'}")
    print(f"  loose dups2 files grouped with another loose file: {s['loose_grouped_with_loose']}/{s['loose_files']}")
    # Each group with the evidence inside it, for the viewer's Groups page: [i, j, weight, kind], member indices.
    where = {p: (g, i) for g, members in enumerate(groups) for i, p in enumerate(members)}
    inside: list[list] = [[] for _ in groups]
    for (a, b), w in weights.items():
        if a in where and b in where and where[a][0] == where[b][0]:
            inside[where[a][0]].append([where[a][1], where[b][1], round(w, 3), kinds[(a, b)]])
    result = {"args": vars(args), "max_noise": max_noise, "edges": dict(source), "score": s,
              "groups": [{"members": m, "edges": e} for m, e in zip(groups, inside, strict=True)]}
    (WORK_DIR / f"groups_{args.model}.json").write_text(json.dumps(result, indent=1), encoding="utf-8")


if __name__ == "__main__":
    main()
