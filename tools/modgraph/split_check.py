"""Group-level DAG check for a planned split of one Rust file (docs/MODULARITY_PLAN.md, M-8 to M-11).

Before a split is made, the planned groups (the new files) are checked: each item of the file is assigned to a group,
and every reference from one item to an item of another group is an edge between the groups. A cycle among the groups
means the split cannot be made as planned.

This is a heuristic, not proof. Items are matched by identifier in their text. An item of another group counts as
referenced when its full name appears in the body, or when its bare name is called (`name(` or `.name`) and that bare
name is unique in the file and not in SKIP. SKIP holds common method names that many types share, so matching them
would invent edges; a call to one of them is therefore never seen. The planned splits name these as blind spots
(`detect`, `trim`, `find` and `put` among them). The compiler's module graph, run after the step (`just modgraph`), is
the check that counts.

Usage: python tools/modgraph/split_check.py SPEC.json
  SPEC.json: {"file": "crates/x/src/y.rs", "groups": {"group": ["item", "Type::method", ...], ...}}
  Items are free functions and types (`name`) or methods (`Type::name`); a type name covers all of its methods.
  Methods of a type that the spec does not name are unassigned and are listed.
"""

import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
FREE = re.compile(
    r"^(?:pub(?:\([a-z]+\))? )?(?:const |static |unsafe |async )*(fn|struct|enum|const|static|type|trait)\s+([A-Za-z_]\w*)"
)
IMPL = re.compile(r"^impl(?:<[^>]*>)?\s+(?:[\w:<>', ]+?\s+for\s+)?([A-Za-z_]\w*)")
METHOD = re.compile(r"^    (?:pub(?:\([\w:]+\))? )?(?:const )?(?:unsafe )?fn\s+([A-Za-z_]\w*)")

# Common method names that many types share. A reference through one of these is not seen (see the module docstring).
SKIP = {
    "new",
    "open",
    "close",
    "drop",
    "push",
    "next",
    "put",
    "get",
    "clear",
    "flush",
    "write",
    "read",
    "commit",
    "default",
    "insert",
    "len",
    "from",
    "into",
    "run",
    "trim",
    "clone",
    "eq",
    "fmt",
    "iter",
    "map",
    "take",
    "find",
    "detect",
}


def parse(path):
    """Return (items, lines): items = [(name, start, end, type)], 0-based line indices, end exclusive.

    Only the production part of the file is read: everything from the first line-start `#[cfg(test)]` is skipped.
    """
    text = (REPO / path).read_text(encoding="utf-8")
    lines = text.split("\n")
    cut = re.search(r"^#\[cfg\(test\)\]", text, re.MULTILINE)
    stop = text[: cut.start()].count("\n") if cut else len(lines)
    starts = []  # (line, kind, name, impl_type)
    impl_type = None
    for i in range(stop):
        line = lines[i]
        if line.startswith("impl"):
            m = IMPL.match(line)
            if m:
                impl_type = m.group(1)
                continue
            if line.startswith("impl Default"):
                m = re.search(r"for\s+(\w+)", line)
                impl_type = m.group(1) if m else None
                continue
        if line.startswith("}"):
            impl_type = None
            continue
        if impl_type is not None:
            m = METHOD.match(line)
            if m:
                starts.append((i, "method", f"{impl_type}::{m.group(1)}", impl_type))
            continue
        m = FREE.match(line)
        if m:
            starts.append((i, "free", m.group(2), None))
    items = []
    for k, (i, _kind, name, t) in enumerate(starts):
        end = stop
        if k + 1 < len(starts):
            end = starts[k + 1][0]
        items.append((name, i, end, t))
    return items, lines


def check(path, groups):
    """Print the group edges for a planned split and whether the groups form a DAG. Returns True when they do."""
    items, lines = parse(path)
    group_of = {}
    for group, names in groups.items():
        for name in names:
            group_of[name] = group

    def group_for(name, t):
        return group_of.get(name) or (group_of.get(t) if t else None)

    assigned = [(n, s, e, t, group_for(n, t)) for n, s, e, t in items if group_for(n, t)]
    unassigned = [n for n, _s, _e, t in items if not group_for(n, t)]
    bare_count = {}
    for n, *_ in items:
        bare = n.split("::")[-1]
        bare_count[bare] = bare_count.get(bare, 0) + 1

    edges = {}
    for name, s, e, _t, group in assigned:
        body = re.sub(r"//[^\n]*", "", "\n".join(lines[s:e]))
        words = set(re.findall(r"\b([A-Za-z_]\w*)\b", body))
        calls = set(re.findall(r"\b([A-Za-z_]\w*)\s*\(", body)) | set(re.findall(r"\.([A-Za-z_]\w*)\b", body))
        for other, _s2, _e2, _t2, group2 in assigned:
            if other == name or group2 == group:
                continue
            bare = other.split("::")[-1]
            usable = bare_count.get(bare) == 1 and bare not in SKIP
            if ("::" not in other and other in words) or (usable and bare in calls):
                edges.setdefault((group, group2), []).append(f"{name} -> {other}")

    adjacency = {}
    for a, b in edges:
        adjacency.setdefault(a, set()).add(b)

    def cyclic():
        def visit(node, stack, seen):
            for nxt in adjacency.get(node, ()):
                if nxt in stack:
                    return True
                if nxt not in seen:
                    seen.add(nxt)
                    if visit(nxt, stack | {nxt}, seen):
                        return True
            return False

        return any(visit(n, {n}, set()) for n in adjacency)

    dag = not cyclic()
    print(f"== {path}: {len(assigned)} items grouped, {len(unassigned)} unassigned, DAG: {dag}")
    if unassigned:
        print("   unassigned: " + ", ".join(unassigned))
    for (a, b), ex in sorted(edges.items()):
        print(f"   {a:>12} -> {b:<12} {len(ex):3}  {', '.join(ex[:3])}")
    return dag


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    spec = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    sys.exit(0 if check(spec["file"], spec["groups"]) else 1)


if __name__ == "__main__":
    main()
