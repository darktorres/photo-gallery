"""Build the intra-crate module graph: which module refers to which, per crate.

Nodes are the modules: each source file under crates/<crate>/src (its module path comes from the file layout), and
each inline `mod name { ... }` block inside a file (its path is the file's, then the block's name). Edges come from
`crate::`, `super::` and `self::` paths in production code, from paths that start with a child module of the current
module (edition 2018+ allows `browse::read::X` in browse.rs without `crate::`), and from the brace and glob re-export
forms (`pub use child::{A, B};`, `pub use child::*;`).

Test-only code is excluded: whole files named tests.rs (or under a tests/ folder), and inline `#[cfg(test)] mod`
blocks, which are cut from the file before scanning.

Usage: python tools/modgraph/module_graph.py [OUT_DIR]   (default: target/modgraph)
Writes graph.json, summary.json, and one <crate>.dot per crate. Standard library only.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

from allowlist import check_all

REPO = Path(__file__).resolve().parents[2]
OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO / "target" / "modgraph"

PATH = re.compile(r"\b(crate|super|self)((?:::[A-Za-z_][A-Za-z0-9_]*)+)")
BRACE = re.compile(r"\b(crate|super|self)::\{([^{}]*)\}")
BARE = re.compile(r"(?<![:\w.])([a-z_][a-z0-9_]*)((?:::[A-Za-z_][A-Za-z0-9_]*)+)")
# A child module used in the brace or glob form, as in `pub use pair::{Coverage, Pairing};` or `pub use grid::*;`.
BARE_BRACE = re.compile(r"(?<![:\w.])([a-z_][a-z0-9_]*)::(?:\{[^{}]*\}|\*)")


def crate_of(path):
    return path.split("/")[1]


def module_of(path):
    """crates/<c>/src/a/b.rs -> 'a::b'; lib.rs / main.rs -> ''; a/mod.rs -> 'a'."""
    rel = path.split("/src/", 1)[1]
    parts = rel[:-3].split("/")
    if parts[-1] == "mod":
        parts = parts[:-1]
    if parts in (["lib"], ["main"]):
        return ""
    return "::".join(parts)


def is_test_file(path):
    rel = path.split("/src/", 1)[1]
    return rel.endswith("tests.rs") or rel.startswith("tests/") or "/tests/" in rel


def production_text(text):
    """Cut the text at an inline test module (by convention it comes last in the file)."""
    m = re.search(r"#\[cfg\(test\)\]\s*mod\s+\w+\s*\{", text)
    return text[: m.start()] if m else text


def inline_mods(text):
    """(start, end, name) of each inline `mod name { ... }` block, by brace matching."""
    out = []
    for m in re.finditer(r"\bmod\s+([A-Za-z_]\w*)\s*\{", text):
        start = m.end() - 1
        depth = 0
        i = start
        while i < len(text):
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        out.append((start, i, m.group(1)))
    return out


def enclosing(ranges, pos):
    """The inline module names around `pos`, outermost first."""
    hits = sorted((s, name) for s, e, name in ranges if s < pos < e)
    return [name for _, name in hits]


def resolve(cur, kind, segs, modules, enc):
    """The module a path refers to. `enc` is the inline modules the path sits in: `super` and `self` are relative
    to that inline module, so `super::Item` inside `mod qobject` in command_line.rs is command_line."""
    effective = (cur.split("::") if cur else []) + enc
    if kind == "crate":
        base = []
    elif kind == "self":
        base = effective
    else:  # super
        base = effective[:-1]
    full = base + segs
    for n in range(len(full), 0, -1):
        cand = "::".join(full[:n])
        if cand in modules:
            return cand
    # No module of that name: the item lives in the module the base names (or the root).
    b = "::".join(base)
    return b if b in modules else ""


def node_id(m, chain):
    """The id of a module: its file's module path, then the inline modules around it (`chain`, outermost first)."""
    return "::".join((m.split("::") if m else []) + chain)


def analyse_crate(crate_files):
    """Module nodes and edges of one crate. Each inline `mod name { }` block is a node of its own: a reference inside
    it is attributed to it, and its child modules resolve from it, so `super::` means the enclosing module, as in
    Rust. The compiler graph (cargo-modules) and this scan then name the same modules."""
    # Every file's module is a possible target (test files too, as before); only production files are nodes.
    modules = {module_of(f): f for f in crate_files}
    nodes = {}
    sources = []
    for f in crate_files:
        if is_test_file(f):
            continue
        m = module_of(f)
        full_text = (REPO / f).read_text(encoding="utf-8", errors="replace")
        # Strip line comments, so doc prose does not make edges.
        text = re.sub(r"//[^\n]*", "", production_text(full_text))
        ranges = inline_mods(text)
        nodes[m] = {"file": f, "lines": len(full_text.splitlines())}
        for start, end, name in ranges:
            mid = node_id(m, [*enclosing(ranges, start), name])
            modules[mid] = f
            nodes[mid] = {"file": f, "lines": text[start:end].count("\n") + 1}
        sources.append((m, text, ranges))

    children_cache = {}

    def children_of(src):
        # Edition 2018+: a path may start with a child module of the current module without `crate::`. Only child
        # modules count, so local names and std or extern paths add nothing.
        if src not in children_cache:
            prefix = (src + "::") if src else ""
            children_cache[src] = {
                x[len(prefix) :] for x in modules if x and x.startswith(prefix) and "::" not in x[len(prefix) :]
            }
        return children_cache[src]

    edges = {}
    for m, text, ranges in sources:

        def source_at(pos, m=m, ranges=ranges):
            return node_id(m, enclosing(ranges, pos))

        refs = []  # (kind, segments, module the path sits in)
        for mm in BRACE.finditer(text):
            src = source_at(mm.start())
            for item in mm.group(2).split(","):
                item = item.strip()
                if not item:
                    continue
                head = item.split("::")[0].strip()
                segs = [] if head == "self" else re.findall(r"[A-Za-z_][A-Za-z0-9_]*", head)[:1]
                refs.append((mm.group(1), segs, src))
        for mm in PATH.finditer(text):
            rest = [s for s in mm.group(2).split("::") if s]
            refs.append((mm.group(1), rest, source_at(mm.start())))
        for mm in BARE.finditer(text):
            src = source_at(mm.start())
            if mm.group(1) in children_of(src):
                rest = [mm.group(1)] + [s for s in mm.group(2).split("::") if s]
                refs.append(("self", rest, src))
        for mm in BARE_BRACE.finditer(text):
            src = source_at(mm.start())
            if mm.group(1) in children_of(src):
                refs.append(("self", [mm.group(1)], src))
        for kind, segs, src in refs:
            tgt = resolve(src, kind, segs, modules, [])
            if tgt == src or tgt not in modules:
                continue
            edges[(src, tgt)] = edges.get((src, tgt), 0) + 1
    edges = {k: v for k, v in edges.items() if k[0] in nodes and k[1] in nodes}
    return nodes, edges


def label(m):
    return m or "(crate root)"


def sccs(nodes, edges):
    """Tarjan's strongly connected components (iterative). Returns the groups with more than one member."""
    index = {}
    low = {}
    on = set()
    stack = []
    out = {n: [] for n in nodes}
    for a, b in edges:
        out[a].append(b)
    result = []
    counter = [0]

    for root in nodes:
        if root in index:
            continue
        work = [(root, 0)]
        while work:
            v, i = work.pop()
            if i == 0:
                index[v] = low[v] = counter[0]
                counter[0] += 1
                stack.append(v)
                on.add(v)
            recurse = False
            children = out[v]
            while i < len(children):
                w = children[i]
                i += 1
                if w not in index:
                    work.append((v, i))
                    work.append((w, 0))
                    recurse = True
                    break
                if w in on:
                    low[v] = min(low[v], index[w])
            if recurse:
                continue
            if low[v] == index[v]:
                comp = []
                while True:
                    w = stack.pop()
                    on.discard(w)
                    comp.append(w)
                    if w == v:
                        break
                if len(comp) > 1:
                    result.append(sorted(comp))
            if work:
                parent = work[-1][0]
                low[parent] = min(low[parent], low[v])
    return result


def main():
    files = subprocess.run(
        ["git", "-C", str(REPO), "ls-files", "--", "crates/*/src/*.rs", "crates/*/src/**/*.rs"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()
    files = sorted(set(files))

    summary = {}
    graph = {"crates": {}}
    OUT.mkdir(parents=True, exist_ok=True)
    for crate in sorted({crate_of(f) for f in files}):
        cfiles = [f for f in files if crate_of(f) == crate]
        nodes, edges = analyse_crate(cfiles)
        if not nodes:
            continue
        indeg = dict.fromkeys(nodes, 0)
        outdeg = dict.fromkeys(nodes, 0)
        for a, b in edges:
            outdeg[a] += 1
            indeg[b] += 1
        cycles = sccs(nodes, edges)
        graph["crates"][crate] = {
            "nodes": [{"id": m, "label": label(m), "file": nodes[m]["file"], "lines": nodes[m]["lines"]} for m in sorted(nodes)],
            "edges": [{"from": a, "to": b, "weight": w} for (a, b), w in sorted(edges.items())],
            "cycles": cycles,
        }
        summary[crate] = {
            "modules": len(nodes),
            "edges": len(edges),
            "cycles": cycles,
            "top_fan_in": sorted(((indeg[n], label(n)) for n in nodes), reverse=True)[:5],
            "top_fan_out": sorted(((outdeg[n], label(n)) for n in nodes), reverse=True)[:5],
            "isolated": sorted(label(n) for n in nodes if indeg[n] == 0 and outdeg[n] == 0),
        }
        write_dot(crate, nodes, edges)

    (OUT / "graph.json").write_text(json.dumps(graph, indent=1), encoding="utf-8")
    (OUT / "summary.json").write_text(json.dumps(summary, indent=1), encoding="utf-8")

    for crate, s in summary.items():
        print(f"{crate}: {s['modules']} modules, {s['edges']} edges, cycles={len(s['cycles'])}")
        for c in s["cycles"]:
            print("   cycle:", " <-> ".join(label(x) for x in c))
    groups = {crate: [[label(x) for x in c] for c in s["cycles"]] for crate, s in summary.items()}
    failures = check_all("scan", groups)
    print(f"text scan: {failures} group(s) not on the allowlist, or stale entries")
    if failures:
        sys.exit(1)


def write_dot(crate, nodes, edges):
    lines = [f'digraph "{crate}" {{', "  rankdir=LR; node [shape=box, fontsize=10];"]
    lines += [f'  "{label(m)}" [label="{label(m)}\\n{nodes[m]["lines"]} lines"];' for m in sorted(nodes)]
    lines += [f'  "{label(a)}" -> "{label(b)}" [label="{w}"];' for (a, b), w in sorted(edges.items())]
    lines.append("}")
    (OUT / f"{crate}.dot").write_text("\n".join(lines) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
