"""Score one model's embeddings against the ground truth.

Usage: python evaluate.py --model sscd

"Flagged" pairs are pairs outside the labeled groups whose similarity clears the
threshold. They are an upper bound on false positives: the library surely holds
unlabeled duplicates too, so review_<model>.csv samples them for hand-labeling.
"""

import argparse
import csv
import json

import numpy as np
import torch

from common import WORK_DIR, ground_truth, image_list

TARGET_RECALLS = (0.80, 0.90, 0.95, 0.98)


def knn(emb: torch.Tensor, k: int, chunk: int = 2048) -> tuple[np.ndarray, np.ndarray]:
    """Exact top-k neighbors (self excluded) by cosine similarity, on the GPU."""
    n = emb.shape[0]
    sims = np.empty((n, k), dtype=np.float32)
    idx = np.empty((n, k), dtype=np.int64)
    for start in range(0, n, chunk):
        q = emb[start:start + chunk]
        s = q @ emb.T
        rows = torch.arange(len(q), device=emb.device)
        s[rows, rows + start] = -2.0
        values, indices = s.topk(k, dim=1)
        sims[start:start + len(q)] = values.float().cpu().numpy()
        idx[start:start + len(q)] = indices.cpu().numpy()
    return sims, idx


def unique_pairs(sims: np.ndarray, idx: np.ndarray, n: int) -> tuple[np.ndarray, np.ndarray]:
    """Collapse directed k-NN edges into unordered pairs: (keys a*n+b with a<b, similarity)."""
    rows = np.repeat(np.arange(sims.shape[0], dtype=np.int64), sims.shape[1])
    cols = idx.ravel()
    keys = np.minimum(rows, cols) * n + np.maximum(rows, cols)
    keys, first = np.unique(keys, return_index=True)
    return keys, sims.ravel()[first]


def directed_ranks(positives: list[tuple[int, int]], groups: list[list[int]], idx: np.ndarray) -> np.ndarray:
    """Rank (0-based) of the partner in each query's neighbor list, both directions; k if absent.

    Only images *outside* the query's group count as ranked ahead: in a group of 3+, another
    member legitimately coming first is not a retrieval miss.
    """
    k = idx.shape[1]
    group_of = {i: g for g, members in enumerate(groups) for i in members}
    ranks = []
    for a, b in positives:
        for q, target in ((a, b), (b, a)):
            hit = np.flatnonzero(idx[q] == target)
            if not len(hit):
                ranks.append(k)
                continue
            ahead = idx[q][:hit[0]]
            ranks.append(sum(group_of.get(int(j)) != group_of[q] for j in ahead))
    return np.array(ranks)


def quantiles(x: np.ndarray) -> str:
    q = np.quantile(x, [0.01, 0.10, 0.50, 0.90, 0.99]) if len(x) else [np.nan] * 5
    return " / ".join(f"{v:.3f}" for v in q)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True)
    parser.add_argument("--k", type=int, default=50)
    args = parser.parse_args()

    files = image_list()
    n = len(files)
    gt = ground_truth(files)
    ok = np.load(WORK_DIR / f"ok_{args.model}.npy")
    emb = torch.from_numpy(np.load(WORK_DIR / f"emb_{args.model}.npy")).cuda()

    # Labeled images that failed to load can't be scored.
    positives = [(a, b) for a, b in gt.positives if ok[a] and ok[b]]
    negative_pool = [i for i in gt.negative_pool if ok[i]]
    unpaired_pool = [i for i in gt.unpaired_pool if ok[i]]

    pa = torch.tensor([a for a, _ in positives], device="cuda")
    pb = torch.tensor([b for _, b in positives], device="cuda")
    pos_sims = (emb[pa].float() * emb[pb].float()).sum(1).cpu().numpy()

    pool = emb[torch.tensor(negative_pool, device="cuda")].float()
    neg_rows, neg_cols = np.triu_indices(len(negative_pool), k=1)
    neg_sims = (pool @ pool.T).cpu().numpy()[neg_rows, neg_cols]

    # True negatives: pairs from two different labeled groups are different pictures.
    members = [i for g in gt.groups for i in g if ok[i]]
    member_group = torch.tensor([gi for gi, g in enumerate(gt.groups) for i in g if ok[i]], device="cuda")
    m = emb[torch.tensor(members, device="cuda")].float()
    different = torch.triu(member_group[:, None] != member_group[None, :], diagonal=1)
    cross_sims = (m @ m.T)[different].cpu().numpy()

    # Each loose dups2 file's best match among the other loose files.
    loose = emb[torch.tensor(unpaired_pool, device="cuda")].float()
    loose_sims = loose @ loose.T
    loose_sims.fill_diagonal_(-2.0)
    best_sims, best_idx = (t.cpu().numpy() for t in loose_sims.max(dim=1))

    sims, idx = knn(emb, args.k)
    pair_keys, pair_sims = unique_pairs(sims, idx, n)
    positive_keys = np.array([min(a, b) * n + max(a, b) for a, b in positives], dtype=np.int64)
    outside = ~np.isin(pair_keys, positive_keys)
    flagged_keys, flagged_sims = pair_keys[outside], pair_sims[outside]

    ranks = directed_ranks(positives, gt.groups, idx)

    operating_points = []
    for target in TARGET_RECALLS:
        t = float(np.quantile(pos_sims, 1 - target, method="lower"))
        operating_points.append({
            "target_recall": target,
            "threshold": t,
            "recall": float((pos_sims >= t).mean()),
            "flagged_pairs": int((flagged_sims >= t).sum()),
            "images_with_a_match": int((sims[:, 0] >= t).sum()),
            # Rows whose k-th neighbor still clears t: their flagged counts are truncated at k.
            "saturated_rows": int((sims[:, -1] >= t).sum()),
            "hard_negative_pairs_flagged": int((neg_sims >= t).sum()),
            "cross_group_pairs_flagged": int((cross_sims >= t).sum()),
            "unpaired_with_partner": float((best_sims >= t).mean()),
        })

    summary = {
        "model": args.model,
        "dim": int(emb.shape[1]),
        "images": n,
        "failed_images": int((~ok).sum()),
        "groups": len(gt.groups),
        "positive_pairs": len(positives),
        "hard_negative_pairs": len(neg_sims),
        "cross_group_pairs": len(cross_sims),
        "unpaired_files": len(unpaired_pool),
        "recall_at_1": float((ranks < 1).mean()),
        "recall_at_10": float((ranks < 10).mean()),
        f"recall_at_{args.k}": float((ranks < args.k).mean()),
        "operating_points": operating_points,
    }
    (WORK_DIR / f"results_{args.model}.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")

    # The labeled pairs the model scores lowest: what kinds of edits does it miss?
    with open(WORK_DIR / f"misses_{args.model}.csv", "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["similarity", "path_a", "path_b"])
        for i in np.argsort(pos_sims)[:50]:
            a, b = positives[i]
            w.writerow([f"{pos_sims[i]:.4f}", files[a], files[b]])

    # The non-duplicates the model finds most similar: what fools it?
    with open(WORK_DIR / f"hard_negatives_{args.model}.csv", "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["similarity", "path_a", "path_b"])
        for i in np.argsort(-neg_sims)[:50]:
            a, b = negative_pool[neg_rows[i]], negative_pool[neg_cols[i]]
            w.writerow([f"{neg_sims[i]:.4f}", files[a], files[b]])

    # Proposed pairing of the loose dups2 files (best match of each, deduplicated).
    # "mutual" = each is the other's best match: a strong candidate for a subfolder.
    proposals = {}
    for i, (j, s) in enumerate(zip(best_idx, best_sims, strict=True)):
        key = (min(i, int(j)), max(i, int(j)))
        proposals[key] = (float(s), int(best_idx[j]) == i)
    with open(WORK_DIR / f"unpaired_pairs_{args.model}.csv", "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["similarity", "mutual", "path_a", "path_b"])
        for (i, j), (s, mutual) in sorted(proposals.items(), key=lambda kv: -kv[1][0]):
            w.writerow([f"{s:.4f}", mutual, files[unpaired_pool[i]], files[unpaired_pool[j]]])

    # Flagged unlabeled pairs at the 95%-recall threshold, spread evenly over the
    # similarity range, for hand-labeling to estimate real precision.
    t95 = next(p["threshold"] for p in operating_points if p["target_recall"] == 0.95)
    above = np.flatnonzero(flagged_sims >= t95)
    above = above[np.argsort(flagged_sims[above])]
    sample = above[np.linspace(0, len(above) - 1, min(300, len(above))).astype(int)] if len(above) else above
    with open(WORK_DIR / f"review_{args.model}.csv", "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["similarity", "path_a", "path_b"])
        for i in sample:
            a, b = divmod(int(flagged_keys[i]), n)
            w.writerow([f"{flagged_sims[i]:.4f}", files[a], files[b]])

    print(f"## {args.model} (dim {summary['dim']}, {n} images, {summary['failed_images']} failed to load)")
    print(f"labeled: {len(gt.groups)} groups -> {len(positives)} positive pairs; {len(neg_sims)} hard negative pairs; "
          f"{len(unpaired_pool)} unpaired dups2 files")
    print(f"retrieval of the partner: R@1 {summary['recall_at_1']:.3f}  R@10 {summary['recall_at_10']:.3f}  "
          f"R@{args.k} {summary[f'recall_at_{args.k}']:.3f}")
    print(f"similarity quantiles p1/p10/p50/p90/p99: positives {quantiles(pos_sims)}")
    print(f"                                       hard negatives {quantiles(neg_sims)}")
    print(f"                                       cross-group    {quantiles(cross_sims)}")
    print()
    print("| target recall | threshold | recall | unpaired files with a partner | flagged pairs (unlabeled) "
          "| images with a match | similar-folder pairs flagged | cross-group pairs flagged |")
    print("|---|---|---|---|---|---|---|---|")
    for p in operating_points:
        print(f"| {p['target_recall']:.2f} | {p['threshold']:.3f} | {p['recall']:.3f} | {p['unpaired_with_partner']:.3f} "
              f"| {p['flagged_pairs']:,} | {p['images_with_a_match']:,} "
              f"| {p['hard_negative_pairs_flagged']} / {len(neg_sims)} "
              f"| {p['cross_group_pairs_flagged']} / {len(cross_sims):,} |")


if __name__ == "__main__":
    main()
