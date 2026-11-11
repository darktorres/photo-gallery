#!/usr/bin/env python3
"""The instruction-count gate (S-16.2): compares the callgrind totals of the benches with a committed baseline.

    perf-compare.py BASELINE RUN_DIR [--tolerance 0.01] [--update] [--skip PREFIX]... [--budgets [FILE]]

RUN_DIR is gungraun's output (target/gungraun): every `callgrind.*.out` under it (not the `.out.old` kept beside it) is one
bench, named by its path relative to RUN_DIR. Its total is the `totals:` line's Ir (the instructions executed). A bench
moved by more than the tolerance (1% by default) fails the gate, in either direction: an improvement is a baseline
to update, not an accident to keep. `--update` writes the run as the new baseline (after a change that should move it).
Exits 1 on a change, on a bench missing from either side, or when there is no baseline yet (S-16.1 makes the first).
`--skip PREFIX` leaves out the benches whose name starts with PREFIX on both sides, for a run that did not measure them
(CI has no catalog data): each one is printed as SKIPPED. It cannot be combined with `--update`.

`--budgets [FILE]` (default docs/perf/budgets.json) is for one comparison whose change moves some counts on purpose: each
budget names a bench and the range its change must fall in (`min`, `max`: fractions, either one optional; `"max": -0.9`
is "must fall by 90% or more"). A budget's bench is the one with a folder of its path named the budget's name, or that
name followed by `.` and the bench's id; `-` and `_` are the same there (`scan-walk` is gungraun's `scan_walk.<id>`). The
benches with no budget keep the tolerance. A budget no bench matches fails the gate, so a renamed bench is not passed
unseen. A bench that misses its budget fails; it cannot be combined with `--update` (the budgets are not a baseline).
"""

import argparse
import json
import sys
from pathlib import Path

DEFAULT_BUDGETS = Path(__file__).resolve().parent.parent / "docs" / "perf" / "budgets.json"


def budget_matches(budget: str, bench: str) -> bool:
    """Whether the bench (its path under RUN_DIR) is the one the budget names: a folder of the path named the budget's
    name, or that name and `.<id>`, with `-` and `_` the same."""
    key = budget.replace("-", "_")
    folders = [part.replace("-", "_") for part in bench.split("/")[:-1]]
    return any(part == key or part.startswith(f"{key}.") for part in folders)


def load_budgets(path: Path) -> dict[str, tuple[float, float]]:
    """The budgets file's {bench: (min, max)}; a missing bound is unbounded."""
    data = json.loads(path.read_text(encoding="utf-8"))
    budgets = {}
    for name, budget in data["budgets"].items():
        low, high = budget.get("min", float("-inf")), budget.get("max", float("inf"))
        if low > high:
            raise ValueError(f"{path}: the budget of {name} has min {low} above max {high}")
        budgets[name] = (float(low), float(high))
    return budgets


def bound_text(low: float, high: float) -> str:
    if low == float("-inf"):
        return f"<= {high:+.0%}"
    if high == float("inf"):
        return f">= {low:+.0%}"
    return f"{low:+.0%}..{high:+.0%}"


def ir_total(path: Path) -> int:
    """The Ir total of one callgrind file: the `totals:` column for the `Ir` event."""
    events: list[str] = []
    for line in path.read_text(errors="replace").splitlines():
        if line.startswith("events:"):
            events = line.split()[1:]
        elif line.startswith("totals:"):
            values = line.split()[1:]
            if "Ir" not in events:
                raise ValueError(f"{path}: no Ir event (events: {events})")
            return int(values[events.index("Ir")])
    raise ValueError(f"{path}: no totals line")


def measure(run_dir: Path) -> dict[str, int]:
    """The Ir total of each callgrind output under run_dir, keyed by its path there. gungraun names them
    callgrind.<bench>.out; its .out.old is the previous run, not a result."""
    found = {}
    for path in sorted(run_dir.rglob("callgrind.*.out")):
        found[path.relative_to(run_dir).as_posix()] = ir_total(path)
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    parser.add_argument("baseline", type=Path)
    parser.add_argument("run_dir", type=Path)
    parser.add_argument("--tolerance", type=float, default=0.01)
    parser.add_argument("--update", action="store_true")
    parser.add_argument("--skip", action="append", default=[], metavar="PREFIX")
    parser.add_argument("--budgets", type=Path, nargs="?", const=DEFAULT_BUDGETS, metavar="FILE")
    args = parser.parse_args()
    if args.update and args.skip:
        parser.error("--update writes every bench it ran: it cannot be combined with --skip")
    if args.update and args.budgets:
        parser.error("--budgets judges one comparison: a bench that misses its budget is not re-baselined with --update")
    budgets = load_budgets(args.budgets) if args.budgets else {}

    run = measure(args.run_dir)
    if not run:
        print(f"no callgrind.*.out under {args.run_dir}: run the benches first", file=sys.stderr)
        return 1
    if args.update:
        args.baseline.parent.mkdir(parents=True, exist_ok=True)
        args.baseline.write_text(json.dumps(run, indent=2, sort_keys=True) + "\n")
        print(f"wrote {len(run)} bench(es) to {args.baseline}")
        return 0
    if not args.baseline.exists():
        print(f"no baseline at {args.baseline}: measure it (S-16.1) and run with --update", file=sys.stderr)
        return 1

    base = json.loads(args.baseline.read_text())
    failed = 0
    skipped = {name for name in set(base) | set(run) if any(name.startswith(p) for p in args.skip)}
    for name in sorted(skipped):
        print(f"SKIPPED  {name}")
    benches = sorted((set(base) | set(run)) - skipped)
    for budget in sorted(budgets):
        if any(budget_matches(budget, name) for name in skipped):
            print(f"SKIPPED  the budget of {budget}: its bench is skipped")
        elif not any(budget_matches(budget, name) for name in benches):
            print(f"NO-BENCH {budget} (a budget in {args.budgets} that no bench matches)")
            failed += 1
    for name in benches:
        if name not in run or name not in base:
            print(f"MISSING  {name} ({'only in the baseline' if name in base else 'only in the run'})")
            failed += 1
            continue
        change = (run[name] - base[name]) / base[name] if base[name] else float("inf")
        budget = next((budgets[key] for key in sorted(budgets) if budget_matches(key, name)), None)
        note = ""
        if budget is None:
            status = "same" if abs(change) <= args.tolerance else ("UP" if change > 0 else "DOWN")
            failed += status != "same"
        else:
            low, high = budget
            status = "budget" if low <= change <= high else ("OVER" if change > high else "UNDER")
            failed += status != "budget"
            note = f"  (budget {bound_text(low, high)})"
        print(f"{status:<8} {name:<40} {base[name]:>14,} -> {run[name]:>14,}  {change:+.2%}{note}")
    limits = f"±{args.tolerance:.0%}" + (" or their budgets" if budgets else "")
    print(f"{failed} bench(es) outside {limits}" if failed else f"all benches within {limits}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
