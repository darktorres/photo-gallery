"""The workspace's structural rules, checked (docs/MODULARITY_PLAN.md, ground rules 2 and 6, M-14 and M-16).

1. Crate layers: each crate's normal dependencies on other workspace crates are exactly the ones its line in
   `tools/crate-layers.txt` lists, and each of them is on an earlier line. A new internal edge fails until the file
   changes in the same commit, and the file's order is a layering, so the normal graph has no cycle.
2. Dev edges: the graph of every internal edge (normal, dev and build) has no cycle. A crate's tests may not depend on
   a crate that depends on it (M-14: the test helpers sit on the crates they test, below the crates whose tests use them).
3. Re-exports: a `pub use` in production code names a child module of the module it is in (a crate's public API laid
   over its private layout). One that names another crate is allowed only in a boundary file (`REEXPORT_BOUNDARIES`),
   and one through `crate::` or `super::` (an item kept at an old path) is not allowed.
4. Platform code: no `cfg` on `windows`, `unix`, `target_os` or `target_family`, no `std::os::windows` or
   `std::os::unix`, and no OS crate or module (`windows_sys`, `libloading::os`), in production code outside `pg-platform`,
   except the glue in `PLATFORM_GLUE`, which must name the platform's own signatures or types.

Production code is a crate's `src/` and its build script, without `#[cfg(test)]` items (an inline module, or a
`mod name;` whose file is then left out too). The test helpers are checked like the rest: pg-testkit's folders are pg-platform's
code, so the rule has no exemption for a test crate.

Usage: python tools/boundaries.py. Needs cargo (`cargo metadata --no-deps`, which reads the manifests and builds
nothing). Standard library only.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
LAYERS = REPO / "tools" / "crate-layers.txt"

# The files that re-export other crates' items, each a documented layer boundary.
REEXPORT_BOUNDARIES = {
    # The front end depends on `pg-app` alone, and names the lower crates' types through it.
    "crates/app/src/reexport.rs",
    # pg-testkit re-exports pg-platform's test folders under the names the tests use.
    "crates/testkit/src/lib.rs",
}
# Production code outside pg-platform that may name a platform, with the reason (ground rule 6).
PLATFORM_GLUE: dict[str, str] = {}

PLATFORM_WORD = re.compile(r"\b(windows|unix|target_os|target_family)\b")
CFG = re.compile(r"\bcfg(?:_attr)?!?\s*\(")
STD_OS = re.compile(r"\bstd::os::(windows|unix)\b")
# The OS crates and modules that only pg-platform calls: the Win32 bindings, and libloading's per-OS loaders.
OS_CRATE = re.compile(r"\bwindows_sys\b|\blibloading::os\b")
PUB_USE = re.compile(r"^\s*pub\s+use\s+([^;]+);", re.MULTILINE)
CHILD_MOD = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_]\w*)\s*[;{]", re.MULTILINE)
# An item compiled only in tests: `#[cfg(test)]`, or `#[cfg(all(test, ...))]`.
CFG_TEST = re.compile(r"^[ \t]*#\[cfg\((?:test|all\(test\b[^\]]*\))\)\][ \t]*\n", re.MULTILINE)


def short(package):
    """`pg-catalog` -> `catalog`: the crate's directory name, as the other tools name it."""
    return package.removeprefix("pg-")


def metadata():
    """The workspace's crates, as {crate: {kind: {internal dependency, ...}}}, kind being normal, dev or build."""
    run = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=REPO,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=True,
    )
    packages = json.loads(run.stdout)["packages"]
    names = {p["name"] for p in packages}
    graph = {}
    for p in packages:
        edges = {"normal": set(), "dev": set(), "build": set()}
        for d in p["dependencies"]:
            if d["name"] in names:
                edges[d["kind"] or "normal"].add(short(d["name"]))
        graph[short(p["name"])] = edges
    return graph


def load_layers(path=LAYERS):
    """The layer file's lines in order, as [(crate, {dependency, ...}, line number)]."""
    out = []
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        crate, sep, rest = line.partition(":")
        if not sep or not crate.strip():
            raise ValueError(f"{path.name}:{number}: expected `crate: dependency dependency ...`, got {raw!r}")
        out.append((crate.strip(), set(rest.split()), number))
    return out


def check_layers(graph, layers):
    failures = []
    seen = set()
    for crate, listed, number in layers:
        if crate in seen:
            failures.append(f"{LAYERS.name}:{number}: {crate} is listed twice")
        if crate not in graph:
            failures.append(f"{LAYERS.name}:{number}: {crate} is not a crate of the workspace")
        else:
            actual = graph[crate]["normal"]
            failures.extend(f"{crate} depends on {dep}, which its line does not list" for dep in sorted(actual - listed))
            failures.extend(f"{crate}'s line lists {dep}, which it does not depend on" for dep in sorted(listed - actual))
        failures.extend(
            f"{LAYERS.name}:{number}: {crate} lists {dep}, which is not on an earlier line" for dep in sorted(listed - seen)
        )
        seen.add(crate)
    failures.extend(f"{crate} is not in {LAYERS.name}" for crate in sorted(set(graph) - seen))
    return failures


def cycles(graph):
    """The strongly connected components of the graph of every internal edge that have more than one crate (Tarjan)."""
    edges = {c: set().union(*kinds.values()) for c, kinds in graph.items()}
    index, low, stack, on_stack, out = {}, {}, [], set(), []

    def visit(v):
        index[v] = low[v] = len(index)
        stack.append(v)
        on_stack.add(v)
        for w in sorted(edges[v]):
            if w not in index:
                visit(w)
                low[v] = min(low[v], low[w])
            elif w in on_stack:
                low[v] = min(low[v], index[w])
        if low[v] == index[v]:
            group = []
            while True:
                w = stack.pop()
                on_stack.discard(w)
                group.append(w)
                if w == v:
                    break
            if len(group) > 1 or v in edges[v]:
                out.append(sorted(group))

    sys.setrecursionlimit(max(1000, 4 * len(edges)))
    for v in sorted(edges):
        if v not in index:
            visit(v)
    return out


def item_end(text, start):
    """The end of the item that starts at `start`: after its `;`, or after the `}` that closes its first `{`."""
    depth = 0
    for i in range(start, len(text)):
        ch = text[i]
        if ch == ";" and depth == 0:
            return i + 1
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return i + 1
    return len(text)


def production(text):
    """`text` with its `#[cfg(test)]` items blanked (their newlines kept, so line numbers hold), and the names of the
    `mod name;` files those items declare."""
    kept, test_mods, pos = [], set(), 0
    for m in CFG_TEST.finditer(text):
        if m.start() < pos:
            continue
        end = item_end(text, m.end())
        item = text[m.end() : end]
        decl = re.match(r"\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;", item)
        if decl:
            test_mods.add(decl.group(1))
        kept.append(text[pos : m.start()])
        kept.append("\n" * text.count("\n", m.start(), end))
        pos = end
    kept.append(text[pos:])
    return "".join(kept), test_mods


def child_dir(path):
    """Where the files of `path`'s child modules are: next to a lib.rs, main.rs or mod.rs; else in a folder of its name."""
    if path.name in ("lib.rs", "main.rs", "mod.rs"):
        return path.parent
    return path.with_suffix("")


def production_files():
    """{file: production text} for every crate's src/ and build.rs, test-only module files left out."""
    out, skipped = {}, set()
    for crate_dir in sorted((REPO / "crates").iterdir()):
        files = sorted(crate_dir.glob("src/**/*.rs"))
        if (crate_dir / "build.rs").exists():
            files.append(crate_dir / "build.rs")
        for path in files:
            text, test_mods = production(path.read_text(encoding="utf-8"))
            out[path] = text
            for name in test_mods:
                skipped.update({child_dir(path) / f"{name}.rs", child_dir(path) / name / "mod.rs"})
    # A test-only module's own children are test-only too.
    return {p: t for p, t in out.items() if not any(p == s or s.with_suffix("") in p.parents for s in skipped)}


def rel(path):
    return path.relative_to(REPO).as_posix()


def crate_of(path):
    return path.relative_to(REPO).parts[1]


def check_reexports(files):
    failures = []
    for path, text in files.items():
        children = set(CHILD_MOD.findall(text))
        for m in PUB_USE.finditer(text):
            body = re.sub(r"\s+", "", m.group(1))
            first = body.split("::", 1)[0]
            if first == "self":
                first = body.split("::")[1] if body.count("::") else ""
            line = text.count("\n", 0, m.start()) + 1
            where = f"{rel(path)}:{line}: pub use {body}"
            if first in ("crate", "super"):
                failures.append(f"{where}: an item re-exported from elsewhere in the crate (name it where it is)")
            elif first not in children and rel(path) not in REEXPORT_BOUNDARIES:
                failures.append(f"{where}: another crate's item, outside the boundary files")
    return failures


def cfg_args(text, start):
    """The text inside the parentheses that open at `start - 1`."""
    depth = 1
    for i in range(start, len(text)):
        if text[i] == "(":
            depth += 1
        elif text[i] == ")":
            depth -= 1
            if depth == 0:
                return text[start:i]
    return text[start:]


def check_platform(files):
    failures = []
    for path, text in files.items():
        if crate_of(path) == "platform" or rel(path) in PLATFORM_GLUE:
            continue
        hits = [m.start() for m in CFG.finditer(text) if PLATFORM_WORD.search(cfg_args(text, m.end()))]
        hits += [m.start() for m in STD_OS.finditer(text)]
        hits += [m.start() for m in OS_CRATE.finditer(text)]
        for pos in sorted(hits):
            line = text.count("\n", 0, pos) + 1
            code = text.splitlines()[line - 1].strip()
            failures.append(f"{rel(path)}:{line}: {code} (platform code belongs in pg-platform)")
    return failures


def main():
    graph = metadata()
    files = production_files()
    sections = [
        ("crate layers (tools/crate-layers.txt)", check_layers(graph, load_layers())),
        ("dev edges acyclic", [f"cycle: {' -> '.join(c)}" for c in cycles(graph)]),
        ("re-exports", check_reexports(files)),
        ("platform code outside pg-platform", check_platform(files)),
    ]
    failed = 0
    for title, failures in sections:
        print(f"{title}: {'ok' if not failures else f'{len(failures)} failure(s)'}")
        for f in failures:
            print(f"  {f}")
        failed += len(failures)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
