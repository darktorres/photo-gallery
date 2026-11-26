#!/usr/bin/env python3
"""Checks the forks under github.com/darktorres against the newest release of each crate upstream.

    fork-updates.py [--lock PATH ...] [--report PATH]

Each lock file names the crates it takes from a fork by their git source. The locked version of each is compared with the
newest stable release of the same crate on crates.io, where the upstream publishes. A newer release is listed, with the
fork's repository, so the fork can be rebased onto it or dropped once upstream has the change. A fork the upstream never
published (crates.io answers 404) has nothing to compare: it is listed as NOT PUBLISHED, and the run does not fail on it.

The lock files default to the root's and the separate workspaces' (fuzz, and the two eval harnesses). A crate locked in
several of them is fetched once and listed once, with each lock file that has it.
Only released versions count: an upstream commit with no release is not listed here.

Exit status: 0 when every fork is at the newest release (or has no release to compare), 1 when one has a newer release, 2
when a release could not be read, so the check did not finish. The Markdown report goes to stdout, and to --report's file
when given.
"""

import argparse
import json
import time
import tomllib
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FORK_OWNER = "darktorres"
FORK_SOURCE = f"git+https://github.com/{FORK_OWNER}/"
USER_AGENT = "photo-gallery-fork-check"
# crates.io asks for no more than one request a second.
REQUEST_GAP_SECONDS = 1.0
DEFAULT_LOCKS = [
    ROOT / "Cargo.lock",
    ROOT / "fuzz" / "Cargo.lock",
    ROOT / "eval" / "jpeg-decoders" / "Cargo.lock",
    ROOT / "eval" / "png-decoders" / "Cargo.lock",
]


class NotPublishedError(Exception):
    """The crate is not on crates.io (the upstream never published it there): there is no release to compare with."""


def forked_crates(lock_path: Path) -> list[tuple[str, str, str]]:
    """(crate, locked version, fork repository) for each crate the lock file takes from a fork."""
    packages = tomllib.loads(lock_path.read_text(encoding="utf-8"))["package"]
    forks = []
    for package in packages:
        source = package.get("source", "")
        if source.startswith(FORK_SOURCE):
            repository = source.removeprefix("git+https://github.com/").split("?")[0]
            forks.append((package["name"], package["version"], repository))
    return sorted(forks)


def collect(lock_paths: list[Path]) -> list[tuple[str, str, str, list[str]]]:
    """Each fork crate once per (crate, locked version, repository), with the lock files that name it, relative to the
    repository's root. A crate locked at two versions is two rows."""
    found: dict[tuple[str, str, str], list[str]] = {}
    for lock_path in lock_paths:
        name = lock_path.resolve().relative_to(ROOT).as_posix()
        for crate, locked, repository in forked_crates(lock_path):
            found.setdefault((crate, locked, repository), []).append(name)
    return [(crate, locked, repository, locks) for (crate, locked, repository), locks in sorted(found.items())]


def newest_release(crate: str) -> str:
    """The newest stable version of the crate on crates.io. Raises NotPublishedError when crates.io has no such crate, and
    ValueError when its answer names no version this script can compare (no stable one, or one it cannot read)."""
    request = urllib.request.Request(f"https://crates.io/api/v1/crates/{crate}", headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:  # noqa: S310 (a fixed https URL: crates.io)
            newest = json.load(response)["crate"]["max_stable_version"]
    except urllib.error.HTTPError as error:
        if error.code == 404:
            raise NotPublishedError(crate) from error
        raise
    if not isinstance(newest, str):
        raise ValueError(f"crates.io gives no stable version for {crate}: {newest!r}")
    version_key(newest)
    return newest


def version_key(version: str) -> tuple[int, int, int]:
    """The major, minor and patch numbers of a version: `0.41.1` is (0, 41, 1); a pre-release tag is ignored."""
    core = version.split("-", maxsplit=1)[0].split("+", maxsplit=1)[0]
    numbers = [int(part) for part in core.split(".")]
    if len(numbers) > 3:
        raise ValueError(f"not a version: {version}")
    numbers += [0] * (3 - len(numbers))
    return (numbers[0], numbers[1], numbers[2])


def status_of(locked: str, newest: str) -> str:
    """NEWER when crates.io has a later release than the locked version, else current."""
    return "NEWER" if version_key(newest) > version_key(locked) else "current"


def report(rows: list[dict[str, str]]) -> str:
    """The Markdown report: one row per fork crate, and what to do when one is behind."""
    lines = [
        "| Crate | Fork | Locked | Newest on crates.io | Status | Lock files |",
        "|---|---|---|---|---|---|",
    ]
    lines.extend(
        f"| `{row['crate']}` | [{row['repository']}](https://github.com/{row['repository']}) | {row['locked']} "
        f"| {row['newest']} | {row['status']} | {', '.join(row['locks'])} |"
        for row in rows
    )
    behind = [row for row in rows if row["status"] == "NEWER"]
    if behind:
        lines += [
            "",
            (
                "A newer release is out for the crates above. For each one, rebase the fork onto the release tag and bump "
                "its `rev` in the root `Cargo.toml` (and in the other manifests that pin it). "
                "Drop the fork once upstream has its changes."
            ),
        ]
    unpublished = [row for row in rows if row["status"] == "NOT PUBLISHED"]
    if unpublished:
        lines += [
            "",
            (
                "Not on crates.io, so there is no release to compare: "
                + ", ".join(f"`{row['crate']}`" for row in unpublished)
                + ". Check the fork's own commits by hand."
            ),
        ]
    return "\n".join(lines) + "\n"


def check(rows_in, fetch) -> list[dict[str, str]]:
    """One report row per collected fork crate. `fetch` returns a crate's newest release (raising NotPublishedError, or
    OSError for a read that failed); each crate is fetched once, with a pause between the requests."""
    rows: list[dict[str, str]] = []
    fetched: dict[str, str | Exception] = {}
    requests_made = 0
    for crate, locked, repository, locks in rows_in:
        row = {"crate": crate, "repository": repository, "locked": locked, "locks": locks}
        if crate not in fetched:
            if requests_made:
                time.sleep(REQUEST_GAP_SECONDS)
            requests_made += 1
            try:
                fetched[crate] = fetch(crate)
            except (OSError, KeyError, ValueError, NotPublishedError) as error:
                fetched[crate] = error
        result = fetched[crate]
        if isinstance(result, NotPublishedError):
            row.update(newest="-", status="NOT PUBLISHED")
        elif isinstance(result, Exception):
            row.update(newest="?", status=f"UNREADABLE ({result})")
        else:
            row.update(newest=result, status=status_of(locked, result))
        rows.append(row)
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description="Compare the forks in the lock files with their crates' newest releases.")
    parser.add_argument(
        "--lock",
        type=Path,
        action="append",
        help="a lock file to read (repeat for more); default: the root's and the fuzz and eval workspaces'",
    )
    parser.add_argument("--report", type=Path, help="also write the Markdown report to this file")
    args = parser.parse_args()

    lock_paths = args.lock or DEFAULT_LOCKS
    rows = check(collect(lock_paths), newest_release)
    text = report(rows)
    print(text, end="")
    if args.report:
        args.report.write_text(text, encoding="utf-8")

    if any(row["status"].startswith("UNREADABLE") for row in rows):
        return 2
    if any(row["status"] == "NEWER" for row in rows):
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
