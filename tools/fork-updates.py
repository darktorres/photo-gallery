#!/usr/bin/env python3
"""Checks the forks under github.com/darktorres against the newest release of each crate upstream (vendor/README.md).

    fork-updates.py [--lock PATH] [--report PATH]

Cargo.lock names each crate taken from a fork by its git source. The locked version of each is compared with the newest
stable release of the same crate on crates.io, where the upstream publishes. A newer release is listed, with the fork's
repository, so the fork can be rebased onto it (vendor/README.md, "To update") or dropped once upstream has the change.
Only released versions count: an upstream commit with no release is not listed here.

Exit status: 0 when every fork is at the newest release, 1 when one has a newer release, 2 when a release could not be
read, so the check did not finish. The Markdown report goes to stdout, and to --report's file when given.
"""

import argparse
import json
import time
import tomllib
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FORK_OWNER = "darktorres"
FORK_SOURCE = f"git+https://github.com/{FORK_OWNER}/"
USER_AGENT = "photo-gallery-fork-check"
# crates.io asks for no more than one request a second.
REQUEST_GAP_SECONDS = 1.0


def forked_crates(lock_path: Path) -> list[tuple[str, str, str]]:
    """(crate, locked version, fork repository) for each crate Cargo.lock takes from a fork."""
    packages = tomllib.loads(lock_path.read_text(encoding="utf-8"))["package"]
    forks = []
    for package in packages:
        source = package.get("source", "")
        if source.startswith(FORK_SOURCE):
            repository = source.removeprefix("git+https://github.com/").split("?")[0]
            forks.append((package["name"], package["version"], repository))
    return sorted(forks)


def newest_release(crate: str) -> str:
    """The newest stable version of the crate on crates.io."""
    request = urllib.request.Request(f"https://crates.io/api/v1/crates/{crate}", headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=30) as response:  # noqa: S310 (crates.io over https only)
        return json.load(response)["crate"]["max_stable_version"]


def version_key(version: str) -> tuple[int, int, int]:
    """The major, minor and patch numbers of a version: `0.41.1` is (0, 41, 1); a pre-release tag is ignored."""
    core = version.split("-", maxsplit=1)[0].split("+", maxsplit=1)[0]
    numbers = [int(part) for part in core.split(".")]
    if len(numbers) > 3:
        raise ValueError(f"not a version: {version}")
    numbers += [0] * (3 - len(numbers))
    return (numbers[0], numbers[1], numbers[2])


def report(rows: list[dict[str, str]]) -> str:
    """The Markdown report: one row per fork crate, and what to do when one is behind."""
    lines = [
        "| Crate | Fork | Locked | Newest on crates.io | Status |",
        "|---|---|---|---|---|",
    ]
    lines.extend(
        f"| `{row['crate']}` | [{row['repository']}](https://github.com/{row['repository']}) | {row['locked']} "
        f"| {row['newest']} | {row['status']} |"
        for row in rows
    )
    behind = [row for row in rows if row["status"] == "newer release"]
    if behind:
        lines += [
            "",
            (
                "A newer release is out for the crates above. For each one, rebase the fork onto the release tag and bump "
                "its `rev` in the root `Cargo.toml` (and in the other manifests that pin it), as vendor/README.md describes. "
                "Drop the fork once upstream has its changes."
            ),
        ]
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description="Compare the forks in Cargo.lock with their crates' newest releases.")
    parser.add_argument("--lock", type=Path, default=ROOT / "Cargo.lock", help="the lock file to read")
    parser.add_argument("--report", type=Path, help="also write the Markdown report to this file")
    args = parser.parse_args()

    rows: list[dict[str, str]] = []
    unreadable = False
    for index, (crate, locked, repository) in enumerate(forked_crates(args.lock)):
        if index:
            time.sleep(REQUEST_GAP_SECONDS)
        row = {"crate": crate, "repository": repository, "locked": locked}
        try:
            newest = newest_release(crate)
            row["newest"] = newest
            behind = version_key(newest) > version_key(locked)
            row["status"] = "newer release" if behind else "current"
        except (OSError, KeyError, ValueError) as error:
            unreadable = True
            row["newest"] = "?"
            row["status"] = f"could not read crates.io ({error})"
        rows.append(row)

    text = report(rows)
    print(text, end="")
    if args.report:
        args.report.write_text(text, encoding="utf-8")

    if unreadable:
        return 2
    if any(row["status"] == "newer release" for row in rows):
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
