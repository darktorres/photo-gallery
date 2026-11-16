"""The keep rule of a pairing-pass change: paired runs of two builds, judged per workload.

Usage:
    python -I keep.py <runs.csv> [--part pass] [--margin 0.01] [--rule margin|no-loss]
    python -I keep.py <runs.csv> --pilot [--part pass] [--margin 0.01]

`runs.csv` is what `keep.ps1` writes: one row per run, with `pair`, `workload`, `build` (`A` or `B`) and the timed parts.
A pair is the two builds' runs of one workload, one after the other.

For each workload, on the paired log-ratios log(B / A):
- the Hodges-Lehmann estimate (the median of the Walsh averages);
- its 95% interval from the exact Wilcoxon signed-rank distribution;
- the verdict: **real gain** when the whole interval is below zero; otherwise, under `--rule margin` (the default),
  **kept** when the interval's upper bound is at most log(1 + margin), and under `--rule no-loss`, **kept** unless the
  whole interval is above zero (a proven loss). `no-loss` is the rule for few pairs (6), whose intervals are too wide
  for a 1% margin; the summary then also says whether the estimates are better overall (their median below zero).

Below 6 pairs there is no 95% interval: each pair's ratio is judged on its own (kept when none is slower than the
margin, a real gain when all are faster).

`--pilot` reads an A/A run (both builds the same binary) and prints, per workload, the spread of the log-ratios and the
number of pairs whose interval half-width falls below the margin (the Hodges-Lehmann estimator's variance is about
pi/3 that of the mean, for normal errors).
"""

import argparse
import csv
import math
from collections import defaultdict
from pathlib import Path


def walsh(d: list[float]) -> list[float]:
    """The Walsh averages (d_i + d_j) / 2 for i <= j, sorted."""
    return sorted((d[i] + d[j]) / 2 for i in range(len(d)) for j in range(i, len(d)))


def signed_rank_lower(n: int, alpha: float) -> int:
    """The largest k with P(T <= k - 1) <= alpha / 2, T the signed-rank statistic of n pairs (exact distribution).

    The interval is then [W_(k), W_(M + 1 - k)] of the M = n(n+1)/2 sorted Walsh averages (1-based).
    """
    m = n * (n + 1) // 2
    counts = [0] * (m + 1)
    counts[0] = 1
    for r in range(1, n + 1):
        for t in range(m, r - 1, -1):
            counts[t] += counts[t - r]
    total = 2**n
    cumulative, k = 0, 0
    for t in range(m + 1):
        cumulative += counts[t]
        if cumulative / total > alpha / 2:
            break
        k = t + 1
    return max(k, 1)


def interval(d: list[float], alpha: float = 0.05) -> tuple[float, float, float]:
    """The Hodges-Lehmann estimate of d's centre and its signed-rank interval."""
    w = walsh(d)
    m = len(w)
    mid = (w[(m - 1) // 2] + w[m // 2]) / 2
    k = signed_rank_lower(len(d), alpha)
    return mid, w[k - 1], w[m - k]


def pct(x: float) -> str:
    return f"{100 * math.expm1(x):+.2f}%"


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("runs")
    p.add_argument("--part", default="pass")
    p.add_argument("--margin", type=float, default=0.01)
    p.add_argument("--pilot", action="store_true")
    p.add_argument("--rule", choices=["margin", "no-loss"], default="margin")
    a = p.parse_args()
    estimates: list[float] = []
    runs: dict[tuple[str, str], dict[str, float]] = defaultdict(dict)
    order: list[str] = []
    with Path(a.runs).open(newline="") as f:
        for row in csv.DictReader(f):
            if row["workload"] not in order:
                order.append(row["workload"])
            # A decimal comma, as PowerShell's Export-Csv writes a double under some locales.
            runs[(row["workload"], row["pair"])][row["build"]] = float(row[a.part].replace(",", "."))
    bound = math.log1p(a.margin)
    verdicts = []
    for workload in order:
        both = [v for (w, _), v in runs.items() if w == workload and "A" in v and "B" in v]
        if any(v["A"] <= 0 or v["B"] <= 0 for v in both):
            # A part that rounds to 0 ms in some run has no ratio: it is below what the probe resolves.
            print(f"{workload}: {a.part} is 0 in some run, below the probe's resolution: not judged")
            continue
        d = [math.log(v["B"] / v["A"]) for v in both]
        if not d:
            continue
        if len(d) < 6:
            # Too few for a 95% signed-rank interval: each pair judged on its own.
            kept = max(d) <= bound
            gain = max(d) < 0
            verdicts.append(kept)
            verdict = "real gain" if gain else "kept" if kept else "NOT kept"
            each = ", ".join(pct(x) for x in d)
            print(f"{workload}: {len(d)} pairs, {a.part} {each} (no interval below 6 pairs): {verdict}")
            continue
        est, lo, hi = interval(d)
        if a.pilot:
            mean = sum(d) / len(d)
            sd = math.sqrt(sum((x - mean) ** 2 for x in d) / (len(d) - 1))
            need = math.ceil((1.96 * sd * math.sqrt(math.pi / 3) / bound) ** 2)
            print(
                f"{workload}: {len(d)} pairs, spread {100 * sd:.2f}%, interval [{pct(lo)}, {pct(hi)}]; "
                f"pairs for a half-width under {100 * a.margin:g}%: {need}"
            )
            continue
        kept = hi <= bound if a.rule == "margin" else lo <= 0
        gain = hi < 0
        verdicts.append(kept)
        estimates.append(est)
        verdict = "real gain" if gain else "kept" if kept else "NOT kept" if a.rule == "margin" else "proven loss"
        print(f"{workload}: {len(d)} pairs, {a.part} {pct(est)} [{pct(lo)}, {pct(hi)}]: {verdict}")
    if verdicts:
        print("all workloads kept" if all(verdicts) else "NOT kept on every workload")
    if a.rule == "no-loss" and estimates:
        estimates.sort()
        middle = (estimates[(len(estimates) - 1) // 2] + estimates[len(estimates) // 2]) / 2
        print(f"median estimate {pct(middle)}: " + ("better overall" if middle < 0 else "NOT better overall"))


if __name__ == "__main__":
    main()
