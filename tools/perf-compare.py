#!/usr/bin/env python3
"""The instruction-count gate (S-16.2): compares the callgrind totals of the benches with a committed baseline.

    perf-compare.py BASELINE RUN_DIR [--tolerance 0.01] [--update] [--skip PREFIX]...

RUN_DIR is gungraun's output (target/gungraun): every `callgrind.*.out` under it (not the `.out.old` kept beside it) is one
bench, named by its path relative to RUN_DIR. Its total is the `totals:` line's Ir (the instructions executed). A bench
moved by more than the tolerance (1% by default) fails the gate, in either direction: an improvement is a baseline
to update, not an accident to keep. `--update` writes the run as the new baseline (after a change that should move it).
Exits 1 on a change, on a bench missing from either side, or when there is no baseline yet (S-16.1 makes the first).
`--skip PREFIX` leaves out the benches whose name starts with PREFIX on both sides, for a run that did not measure them
(CI has no catalog data): each one is printed as SKIPPED. It cannot be combined with `--update`.
"""

import argparse
import json
import sys
from pathlib import Path


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
    args = parser.parse_args()
    if args.update and args.skip:
        parser.error("--update writes every bench it ran: it cannot be combined with --skip")

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
    for name in sorted((set(base) | set(run)) - skipped):
        if name not in run or name not in base:
            print(f"MISSING  {name} ({'only in the baseline' if name in base else 'only in the run'})")
            failed += 1
            continue
        change = (run[name] - base[name]) / base[name] if base[name] else float("inf")
        status = "same" if abs(change) <= args.tolerance else ("UP" if change > 0 else "DOWN")
        if status != "same":
            failed += 1
        print(f"{status:<8} {name:<40} {base[name]:>14,} -> {run[name]:>14,}  {change:+.2%}")
    print(f"{failed} bench(es) outside ±{args.tolerance:.0%}" if failed else "all benches within tolerance")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
