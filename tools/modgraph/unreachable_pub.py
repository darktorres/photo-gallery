#!/usr/bin/env python3
"""Counts rustc's `unreachable_pub` warnings for each crate's library target (docs/MODULARITY_PLAN.md, M-18), and any compile
errors (a crate that does not build here is reported, with its first error lines). Run it on the target you want counted:
Windows on the host (with QT_DIR and its bin folder on PATH), Linux in the pg-dev container.

    python tools/modgraph/unreachable_pub.py

Crates with no library target (pg-bench, pg-gallery) are not listed: the script takes only the library crates.
"""

import json
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent


def library_crates() -> list[str]:
    """The workspace's packages that have a library target."""
    metadata = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=REPO, capture_output=True, text=True, check=True
    )
    return [
        package["name"]
        for package in json.loads(metadata.stdout)["packages"]
        if any(target["kind"] == ["lib"] for target in package["targets"])
    ]


def main() -> None:
    for crate in library_crates():
        result = subprocess.run(
            ["cargo", "rustc", "-p", crate, "--lib", "--profile", "check", "--", "-W", "unreachable_pub"],
            cwd=REPO,
            capture_output=True,
            text=True,
            check=False,
        )
        lines = (result.stdout + result.stderr).splitlines()
        count = sum("unreachable `pub`" in line for line in lines)
        errors = [i for i, line in enumerate(lines) if line.startswith("error")]
        print(f"{crate} unreachable_pub={count} errors={len(errors)}")
        # The first three errors, each with the two lines after it.
        for i in errors[:3]:
            for line in lines[i : i + 3]:
                print(f"    {line}")
    sys.exit(0)


if __name__ == "__main__":
    main()
