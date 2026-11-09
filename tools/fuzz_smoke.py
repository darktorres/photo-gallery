#!/usr/bin/env python3
"""Each fuzz target (fuzz/, S-18.1) for a short time, in the container's tools image (`just fuzz-smoke`, which runs this
through tools/linux.py fuzz). The tree fuzzed is /head: the launcher's git archive of HEAD. The fuzzer's build is a volume of
its own. A crash stops the run, and its input is copied to fuzz/artifacts/ in the checkout.
    python3 tools/fuzz_smoke.py [seconds per target, default 60]
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TREE = Path("/head")


def run(command: list[str], cwd: Path, *, capture: bool = False, quiet: bool = False) -> subprocess.CompletedProcess[str]:
    """Runs `command`, printing it first. `capture` keeps its output; `quiet` discards it."""
    print("+ " + " ".join(command), flush=True)
    return subprocess.run(
        command,
        cwd=cwd,
        check=False,
        text=True,
        capture_output=capture,
        stdout=subprocess.DEVNULL if quiet else None,
    )


def main() -> None:
    seconds = sys.argv[1] if len(sys.argv) > 1 else "60"
    fuzz = TREE / "fuzz"
    os.environ["CARGO_TARGET_DIR"] = "/bench/fuzz-target"
    print(f"fuzz at {os.environ.get('PG_HEAD_SHA', 'HEAD')}")
    # fuzz/ is its own workspace with its own Cargo.lock, and cargo-fuzz does not pass --locked: fail here if the lock is stale.
    if run(["cargo", "metadata", "--locked", "--format-version", "1"], fuzz, quiet=True).returncode != 0:
        sys.exit(1)
    # Listed first: a failing list would run no target, and the run would still pass.
    listed = run(["cargo", "fuzz", "list"], fuzz, capture=True)
    if listed.returncode != 0:
        sys.exit(listed.returncode)
    targets = listed.stdout.split()
    if not targets:
        sys.exit(f"cargo fuzz list found no fuzz targets in {fuzz}")
    for target in targets:
        print(f"== {target}: {seconds}s", flush=True)
        if run(["cargo", "fuzz", "run", target, "--", f"-max_total_time={seconds}"], fuzz).returncode != 0:
            shutil.copytree(TREE / "fuzz" / "artifacts", REPO / "fuzz" / "artifacts", dirs_exist_ok=True)
            print("crash: the input is in fuzz/artifacts/ in the checkout", file=sys.stderr)
            sys.exit(1)
    print(f"fuzz-smoke: every target ran for {seconds}s with no crash")


if __name__ == "__main__":
    main()
