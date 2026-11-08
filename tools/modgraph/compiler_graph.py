"""The compiler's module graph: `cargo modules dependencies` for each library crate, and its module-level cycle groups.

Runs on the target of the current machine (Windows on the host; Linux in the pg-dev container). For each library crate
it writes the DOT graph to OUT/<crate>.dot, maps each item to its enclosing module (the "uses" edges only),
and prints the cycle groups (strongly connected components with more than one module). cargo-modules cannot see the
cxx-qt bridge modules, so the text scan (module_graph.py) covers those; `just modgraph` runs both.

A group is reported, not failed here: the allowlist that makes a new group fail is M-16 in docs/MODULARITY_PLAN.md.
Usage: python tools/modgraph/compiler_graph.py [OUT_DIR]   (default: target/modgraph/cargo-modules)
"""

import json
import re
import subprocess
import sys
from pathlib import Path

from module_graph import REPO, sccs

DEFAULT_OUT = REPO / "target" / "modgraph" / "cargo-modules"
NODE = re.compile(r'^\s*"([^"]+)" \[label="([^"]*)"', re.MULTILINE)
# cargo-modules labels each edge "uses" or "owns" (a module's containment of its items). Only "uses" are dependencies.
# The "owns" edges stay in the output, because without them cargo-modules drops a module that has no "uses" edge, and
# its items would then be counted as the crate root's.
EDGE = re.compile(r'"([^"]+)" -> "([^"]+)" \[label="(\w+)"')


def library_crates():
    """The workspace's packages with a library target, by name."""
    meta = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=True,
    )
    return [p["name"] for p in json.loads(meta.stdout)["packages"] if any("lib" in t["kind"] for t in p["targets"])]


def graph_of(crate):
    """The crate's module graph as DOT text, or raises with cargo-modules' error."""
    run = subprocess.run(
        [
            "cargo",
            "modules",
            "dependencies",
            "--package",
            crate,
            "--lib",
            "--no-externs",
            "--no-sysroot",
        ],
        check=False,
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    if run.returncode != 0:
        raise RuntimeError(run.stderr.strip() or f"cargo modules exited with {run.returncode}")
    return run.stdout


def cycle_groups(dot, crate):
    """The module-level cycle groups of one crate's DOT graph, and the number of module edges."""
    prefix = crate.replace("-", "_")

    def name(node):
        return node.replace(prefix, "", 1).lstrip(":")

    kinds = {name(node): label for node, label in NODE.findall(dot)}
    modules = {n for n, label in kinds.items() if "mod|" in label or n == ""}

    def module_of(item):
        parts = item.split("::") if item else []
        for k in range(len(parts), -1, -1):
            candidate = "::".join(parts[:k])
            if candidate in modules:
                return candidate
        return ""

    edges = {}
    for a, b, label in EDGE.findall(dot):
        if label != "uses":
            continue
        ma, mb = module_of(name(a)), module_of(name(b))
        if ma != mb:
            edges[(ma, mb)] = 1
    return sccs(modules, edges), len(edges)


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_OUT
    out.mkdir(parents=True, exist_ok=True)
    failed = []
    groups_total = 0
    for crate in library_crates():
        try:
            dot = graph_of(crate)
        except RuntimeError as err:
            failed.append(crate)
            print(f"{crate}: cargo-modules failed: {err}")
            continue
        (out / f"{crate}.dot").write_text(dot, encoding="utf-8")
        groups, edges = cycle_groups(dot, crate)
        groups_total += len(groups)
        print(f"{crate}: {edges} module edges, {len(groups)} cycle group(s)")
        for group in groups:
            print("   group:", " <-> ".join(sorted(x or "(crate root)" for x in group)))
    print(f"compiler graph: {groups_total} cycle group(s); DOT files in {out}")
    if failed:
        print("cargo-modules failed for: " + ", ".join(failed))
        sys.exit(1)


if __name__ == "__main__":
    main()
