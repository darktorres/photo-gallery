"""Every internal normal dependency edge: what the dependent's production code names through it.

Collects `pg_x::a::b` paths and `use pg_x::{...}` trees in src/ before the first line-start #[cfg(test)], per file.
Prints, per edge, the distinct names, how many files use it, and edges whose only uses are in test code.

A report for a review (`just modgraph-report`), not a gate: it matches by name, so it can both miss a use and invent one.
"""

import re
import tomllib
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def split(text):
    m = re.search(r"^#\[cfg\(test\)\]", text, flags=re.MULTILINE)
    return (text[: m.start()], text[m.start() :]) if m else (text, "")


def strip(text):
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    return "\n".join(l.split("//")[0] for l in text.splitlines())


def names(text, ident):
    found = set()
    for m in re.finditer(r"\b" + ident + r"::([A-Za-z0-9_:]+(?:::\{)?)", text):
        found.add(m.group(1).rstrip(":{"))
    # use trees: use pg_x::{a, b::{c, d}};
    for m in re.finditer(r"\buse\s+" + ident + r"::\{(.*?)\};", text, flags=re.DOTALL):
        for part in re.split(r"[,\s]+", re.sub(r"[{}]", " ", m.group(1))):
            if part:
                found.add(part)
    return found


rows = []
for manifest in sorted(ROOT.glob("crates/*/Cargo.toml")):
    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    me = data["package"]["name"]
    deps = [d for d in data.get("dependencies", {}) if d.startswith("pg-")]
    files = list((manifest.parent / "src").rglob("*.rs"))
    for dep in deps:
        ident = dep.replace("-", "_")
        prod, test = defaultdict(set), defaultdict(set)
        for f in files:
            p, t = split(f.read_text(encoding="utf-8", errors="replace"))
            pn, tn = names(strip(p), ident), names(strip(t), ident)
            rel = f.relative_to(manifest.parent / "src").as_posix()
            if pn:
                prod[rel] |= pn
            if tn:
                test[rel] |= tn
        all_prod = set().union(*prod.values()) if prod else set()
        rows.append((me, dep, all_prod, sorted(prod), bool(test)))

for me, dep, items, files, has_test in rows:
    flag = "" if items else ("  <-- ONLY TESTS" if has_test else "  <-- NO PATH USE (macro/derive/build?)")
    print(f"{me} -> {dep}: {len(items)} names in {len(files)} files{flag}")
    print("    " + ", ".join(sorted(items))[:600])
    if len(files) <= 3:
        print("    files: " + ", ".join(files))
