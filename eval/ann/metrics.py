"""Scoring a method's self-join against the exact one.

Every method's candidates are rescored exactly (cosine of the stored vectors) before the candidate rule, as the
pipeline would: each row keeps its K best candidates with similarity >= FLOOR, and the candidate pairs are the union
over rows (unordered). Pairs:
- required: in the exact rule's set, and beating that row's (K+1)th neighbour by TIE (a tie can't be required);
- radius: every pair >= FLOOR (exact top lists, plus hubs' full radius lists).
Pair kinds: planted (a copy and its own source), planted_other (any other pair with a copy), dups2 (two members of
one labelled group), test (the rest). Bands are by the pair's exact cosine.
"""

from dataclasses import dataclass

import numpy as np
import torch
from scipy.stats import beta

from ann.data import EDITS, FLOOR, TIE, Base, K, pair_half, pair_keys

BANDS = (0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.01)
KINDS = ("planted", "planted_other", "dups2", "test")


@dataclass
class Truth:
    """The exact sets of one subset, in its local ids."""

    n: int
    rule_keys: np.ndarray  # sorted: the exact candidate pairs (K best >= FLOOR per row)
    required: np.ndarray  # sorted keys of required pairs
    required_sim: np.ndarray
    radius_keys: np.ndarray  # sorted: every pair >= FLOOR
    top10: np.ndarray  # (n, K) exact top ids, for recall@10
    edit: np.ndarray  # local planted edit or -1
    source: np.ndarray  # local planted source or -1
    group: np.ndarray
    copies: np.ndarray  # local ids of planted copies whose source is present
    copy_keys: np.ndarray  # key of (copy, its source), aligned with copies
    dups2_keys: np.ndarray  # every pair inside a labelled group


def truth(base: Base, sub: np.ndarray, gt: dict) -> Truth:
    n = len(sub)
    sims, ids = gt["sims"], gt["ids"].astype(np.int64)
    rows = np.repeat(np.arange(n), K)
    top_s, top_i = sims[:, :K].ravel(), ids[:, :K].ravel()
    keep = top_s >= FLOOR
    rule_keys = np.unique(pair_keys(rows[keep], top_i[keep], n))
    margin = np.repeat(sims[:, K] + TIE, K)
    req = keep & (top_s > margin)
    req_keys, first = np.unique(pair_keys(rows[req], top_i[req], n), return_index=True)
    req_sim = top_s[req][first]

    rows64 = np.repeat(np.arange(n), ids.shape[1])
    s64, i64 = sims.ravel(), ids.ravel()
    rad = [pair_keys(rows64[s64 >= FLOOR], i64[s64 >= FLOOR], n)]
    for j, r in enumerate(gt["hub_rows"]):
        hub = gt["hub_ids"][gt["hub_ptr"][j] : gt["hub_ptr"][j + 1]].astype(np.int64)
        rad.append(pair_keys(np.full(len(hub), r), hub, n))
    radius_keys = np.unique(np.concatenate(rad))

    local = np.full(len(base.x), -1)
    local[sub] = np.arange(n)
    edit = base.planted_edit[sub]
    source = np.where(edit >= 0, local[np.clip(base.planted_source[sub], 0, None)], -1)
    group = base.group[sub]
    copies = np.flatnonzero((edit >= 0) & (source >= 0))
    pairs = [np.zeros(0, np.int64)]
    for g in np.unique(group[group >= 0]):
        members = np.flatnonzero(group == g)
        a, b = np.triu_indices(len(members), 1)
        pairs.append(pair_keys(members[a], members[b], n))
    return Truth(
        n,
        rule_keys,
        req_keys,
        req_sim,
        radius_keys,
        ids[:, :K],
        edit,
        source,
        group,
        copies,
        pair_keys(copies, source[copies], n),
        np.concatenate(pairs),
    )


def rescore(x: np.ndarray, cand: np.ndarray, chunk: int = 4096) -> tuple[np.ndarray, np.ndarray]:
    """Each row's candidates (local ids, -1 = none) sorted by exact cosine: (ids, sims), invalid ones at -inf."""
    xt = torch.from_numpy(x).cuda()
    out_i = np.empty(cand.shape, dtype=np.int64)
    out_s = np.empty(cand.shape, dtype=np.float32)
    for s in range(0, len(cand), chunk):
        c = torch.from_numpy(cand[s : s + chunk].astype(np.int64)).cuda()
        rows = torch.arange(s, s + len(c), device="cuda")[:, None]
        valid = (c >= 0) & (c != rows)
        sim = torch.einsum("rd,rkd->rk", xt[s : s + len(c)], xt[c.clamp(min=0)])
        sim[~valid] = -float("inf")
        # The same candidate twice (a method may return duplicates) counts once.
        cs, order = torch.sort(torch.where(valid, c, -1 - rows), dim=1)
        dup = torch.zeros_like(valid)
        dup[:, 1:] = cs[:, 1:] == cs[:, :-1]
        sim = sim.gather(1, order)
        sim[dup] = -float("inf")
        v, o = sim.sort(dim=1, descending=True)
        out_s[s : s + len(c)] = v.cpu().numpy()
        out_i[s : s + len(c)] = cs.gather(1, o).cpu().numpy()
    return out_i, out_s


def candidate_sets(ids: np.ndarray, sims: np.ndarray, n: int, kprime: int) -> tuple[np.ndarray, np.ndarray]:
    """(rule keys, radius keys) of a method keeping its first kprime rescored candidates per row."""
    ids, sims = ids[:, :kprime], sims[:, :kprime]
    rows = np.repeat(np.arange(len(ids)), ids.shape[1]).reshape(ids.shape)
    rule = (sims >= FLOOR) & (np.arange(ids.shape[1])[None, :] < K)
    rad = sims >= FLOOR
    return np.unique(pair_keys(rows[rule], ids[rule], n)), np.unique(pair_keys(rows[rad], ids[rad], n))


def kind_of(t: Truth, keys: np.ndarray) -> np.ndarray:
    a, b = keys // t.n, keys % t.n
    planted_own = (t.source[a] == b) | (t.source[b] == a)
    planted_any = (t.edit[a] >= 0) | (t.edit[b] >= 0)
    dups2 = (t.group[a] >= 0) & (t.group[a] == t.group[b])
    return np.select([planted_own, planted_any, dups2], [0, 1, 2], 3)


def score(t: Truth, ids: np.ndarray, sims: np.ndarray, kprime: int) -> dict:
    rule, rad = candidate_sets(ids, sims, t.n, kprime)
    found = np.isin(t.required, rule, assume_unique=True)
    half = pair_half(t.required)
    kind = kind_of(t, t.required)
    band = np.digitize(t.required_sim, BANDS) - 1
    out: dict = {"kprime": kprime, "required": len(t.required)}
    for h, name in ((0, "tune"), (1, "eval")):
        m = half == h
        out[f"recall_{name}"] = float(found[m].mean())
        out[f"missed_{name}"] = int((~found[m]).sum())
        out[f"bands_{name}"] = {
            f"{BANDS[b]:.1f}": [float(found[m & (band == b)].mean()), int((m & (band == b)).sum())]
            for b in range(len(BANDS) - 1)
            if (m & (band == b)).any()
        }
        out[f"kinds_{name}"] = {
            KINDS[k]: [float(found[m & (kind == k)].mean()), int((m & (kind == k)).sum())]
            for k in range(len(KINDS))
            if (m & (kind == k)).any()
        }
    out["recall_radius"] = float(np.isin(t.radius_keys, rad, assume_unique=True).mean())

    # Planted copies vs their source, per edit: the exact rule's rate is the ceiling.
    in_exact = np.isin(t.copy_keys, t.rule_keys)
    in_method = np.isin(t.copy_keys, rule)
    edit = t.edit[t.copies]
    out["planted"] = {
        EDITS[e]: [float(in_method[edit == e].mean()), float(in_exact[edit == e].mean())]
        for e in range(len(EDITS))
        if (edit == e).any()
    }
    # dups2 pairs: found by the method / by the exact rule.
    dk = t.dups2_keys
    out["dups2"] = [float(np.isin(dk, rule).mean()), float(np.isin(dk, t.rule_keys).mean()), len(dk)]
    hits = (ids[:, :K, None] == t.top10[:, None, :]).any(2).sum(1)
    out["recall_at_10"] = float(hits.mean() / K)
    return out


def missed(t: Truth, ids: np.ndarray, sims: np.ndarray, kprime: int) -> np.ndarray:
    """Keys of the required pairs this method's rule set lacks."""
    rule, _ = candidate_sets(ids, sims, t.n, kprime)
    return t.required[~np.isin(t.required, rule, assume_unique=True)]


def clopper_pearson(successes: int, trials: int, conf: float = 0.95) -> tuple[float, float]:
    a = (1 - conf) / 2
    lo = beta.ppf(a, successes, trials - successes + 1) if successes > 0 else 0.0
    hi = beta.ppf(1 - a, successes + 1, trials - successes) if successes < trials else 1.0
    return float(lo), float(hi)
