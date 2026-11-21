"""Every Cargo feature of the workspace's crates is linted: `just lint` compiles it, or it is named in ALLOWED with a reason.

A feature of a crate counts as linted when one of these holds:
1. A lint step compiles it: a cargo build, check or clippy step that selects the crate (`--workspace` selects every crate,
   with its default features; `-p` selects one) and turns the feature on (`--features`, or `--all-features`).
2. A crate in the workspace depends on this one with the feature (`features = [...]`). Cargo unifies those features into
   every build that compiles the depending crate, the tests included (`--all-targets`). An optional dependency counts only
   when the depending crate's default features turn it on.
3. It is in ALLOWED, with the reason it is not compiled.

The check builds nothing: it reads `cargo metadata` and the lint steps in tools/lint.py. It covers the root workspace
only; `fuzz` and `eval/jpeg-decoders` are separate workspaces, which lint.py checks with their own metadata commands.
An ALLOWED entry that names no feature of the workspace fails too, so the list cannot go stale.
"""

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# (crate, feature) -> why `just lint` does not compile it. Empty: every feature is compiled by a lint step or a dependency.
ALLOWED: dict[tuple[str, str], str] = {}

# The cargo subcommands that compile the code, so a feature they turn on is linted.
COMPILING = {"clippy", "check", "build", "test"}


def workspace_metadata() -> dict:
    """The root workspace's packages and their declared dependencies (no dependency graph is needed)."""
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return json.loads(out)


def closure(features: dict[str, list[str]], start: set[str]) -> set[str]:
    """The features of one crate that `start` turns on, counting the features they enable. A `dep:` entry or a
    `dep/feature` entry enables a dependency, not a feature of this crate, so it is not followed."""
    seen: set[str] = set()
    todo = list(start)
    while todo:
        name = todo.pop()
        if name in seen or name not in features:
            continue
        seen.add(name)
        for entry in features[name]:
            if not entry.startswith("dep:") and "/" not in entry:
                todo.append(entry)
    return seen


def optional_on(package: dict, dep_name: str) -> bool:
    """Whether the default features of `package` turn on its optional dependency `dep_name`."""
    entries = {entry for feature in closure(package["features"], {"default"}) for entry in package["features"][feature]}
    return f"dep:{dep_name}" in entries or dep_name in entries


def values_after(step: list[str], flags: tuple[str, ...]) -> list[str]:
    """Each value a flag takes in a command, split on commas (`--features a,b` names two features)."""
    found: list[str] = []
    for i, token in enumerate(step[:-1]):
        if token in flags:
            found.extend(part for part in step[i + 1].replace(" ", ",").split(",") if part)
    return found


def lint_coverage(members: dict[str, dict], steps: list[list[str]]) -> dict[str, set[str]]:
    """The features each crate's lint steps compile."""
    covered: dict[str, set[str]] = {name: set() for name in members}
    for step in steps:
        if len(step) < 2 or step[0] != "cargo" or step[1] not in COMPILING:
            continue
        workspace = "--workspace" in step
        selected = list(members) if workspace else values_after(step, ("-p", "--package"))
        features = values_after(step, ("--features", "-F"))
        for name in selected:
            if name not in members:
                continue
            package = members[name]
            if "--all-features" in step:
                covered[name] |= set(package["features"])
                continue
            start = set(features)
            if "--no-default-features" not in step:
                start.add("default")
            covered[name] |= closure(package["features"], start)
    return covered


def dependency_coverage(members: dict[str, dict]) -> dict[str, set[str]]:
    """The features the workspace's crates turn on in each other: a dependency's `features` list, when it is active."""
    covered: dict[str, set[str]] = {name: set() for name in members}
    for package in members.values():
        for dep in package["dependencies"]:
            target = dep["name"]
            if target not in members:
                continue
            if dep["optional"] and not optional_on(package, target):
                continue
            covered[target] |= closure(members[target]["features"], set(dep["features"]))
    return covered


def check(meta: dict, steps: list[list[str]]) -> tuple[list[tuple[str, str]], list[tuple[str, str]]]:
    """(the features nothing compiles, the ALLOWED entries that name no feature). Both empty when every feature is linted."""
    members = {package["name"]: package for package in meta["packages"]}
    by_steps = lint_coverage(members, steps)
    by_deps = dependency_coverage(members)
    uncovered = sorted(
        (name, feature)
        for name, package in members.items()
        for feature in package["features"]
        if feature != "default" and feature not in by_steps[name] | by_deps[name] and (name, feature) not in ALLOWED
    )
    stale = sorted(
        key for key in ALLOWED if key[0] not in members or key[1] not in members[key[0]]["features"]
    )
    return uncovered, stale


def main() -> None:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from lint import STEPS

    meta = workspace_metadata()
    uncovered, stale = check(meta, STEPS)
    for name, feature in uncovered:
        print(f"not linted: {name} feature `{feature}` (add a lint step with --features, or ALLOWED with a reason)")
    for name, feature in stale:
        print(f"stale ALLOWED entry: {name} feature `{feature}` is not a feature of the workspace")
    if uncovered or stale:
        sys.exit(1)
    total = sum(sum(1 for feature in package["features"] if feature != "default") for package in meta["packages"])
    print(f"feature coverage: all {total} features of {len(meta['packages'])} crates are linted or allowed")


if __name__ == "__main__":
    main()
