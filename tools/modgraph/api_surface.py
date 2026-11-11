"""Every public item of every workspace library crate (cargo-modules structure), and where it is used.

For each item: production uses in the dependent crates' src/ (other crates), uses in tests/benches/examples
(any crate, own crate's integration tests included), by word match on the item's name, restricted to files of crates
that can name the defining crate (it, or a crate depending on it). Output: JSON plus a text table of the items no
other crate's production code names.

A report for a review (`just modgraph-report`), not a gate: it matches by name, so it can both miss a use and invent one.
"""

import json
import re
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target" / "modgraph" / "api.json"
OUT.parent.mkdir(parents=True, exist_ok=True)
ANSI = re.compile(r"\x1b\[[0-9;]*m")
LINE = re.compile(r"^(?P<indent>[│├└─ ]*)(?P<kind>[a-z ]+?) (?P<name>[A-Za-z0-9_]+)(?:: (?P<vis>.+))?$")

crates = {}
for manifest in sorted(ROOT.glob("crates/*/Cargo.toml")):
    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    name = data["package"]["name"]
    deps = set()
    for table in ("dependencies",):
        for dep in data.get(table, {}):
            if dep.startswith("pg-"):
                deps.add(dep)
    dev = {d for d in data.get("dev-dependencies", {}) if d.startswith("pg-")}
    crates[name] = {"dir": manifest.parent, "deps": deps, "dev": dev}


def rs_files(d: Path, sub: str):
    p = d / sub
    return list(p.rglob("*.rs")) if p.exists() else []


def strip_comments(text: str) -> str:
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    return "\n".join(line.split("//")[0] for line in text.splitlines())


def prod_text(path: Path) -> str:
    text = path.read_text(encoding="utf-8", errors="replace")
    # Production code: up to the first line-start #[cfg(test)].
    m = re.search(r"^#\[cfg\(test\)\]", text, flags=re.MULTILINE)
    if m:
        text = text[: m.start()]
    return strip_comments(text)


def test_text(path: Path) -> str:
    text = path.read_text(encoding="utf-8", errors="replace")
    m = re.search(r"^#\[cfg\(test\)\]", text, flags=re.MULTILINE)
    return strip_comments(text[m.start() :]) if m else ""


texts = {}
for name, c in crates.items():
    d = c["dir"]
    texts[name] = {
        "prod": "\n".join(prod_text(p) for p in rs_files(d, "src")),
        "unit": "\n".join(test_text(p) for p in rs_files(d, "src")),
        "itest": "\n".join(
            strip_comments(p.read_text(encoding="utf-8", errors="replace"))
            for sub in ("tests", "benches", "examples")
            for p in rs_files(d, sub)
        ),
    }

users = {n: [m for m, c in crates.items() if n in c["deps"]] for n in crates}
dev_users = {n: [m for m, c in crates.items() if n in c["dev"]] for n in crates}

result = {}
for name, c in crates.items():
    if not (c["dir"] / "src" / "lib.rs").exists():
        continue
    proc = subprocess.run(
        ["cargo", "modules", "structure", "-p", name, "--lib"],
        check=True,
        cwd=ROOT,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    items = []
    stack = []
    for raw in ANSI.sub("", proc.stdout).splitlines():
        m = LINE.match(raw)
        if not m:
            continue
        depth = len(m["indent"]) // 4
        kind, iname, vis = m["kind"].strip(), m["name"], (m["vis"] or "").strip()
        if kind == "crate":
            stack = []
            continue
        stack = [*stack[: depth - 1], iname]
        if vis == "pub":
            items.append({"kind": kind, "path": "::".join(stack), "name": iname})
    for it in items:
        if it["kind"] == "mod":
            continue
        word = re.compile(r"\b" + re.escape(it["name"]) + r"\b")
        prod_users = [u for u in users[name] if word.search(texts[u]["prod"])]
        test_users = sorted(
            set(
                [u for u in users[name] + dev_users[name] if word.search(texts[u]["unit"]) or word.search(texts[u]["itest"])]
                + ([name] if word.search(texts[name]["itest"]) else [])
            )
        )
        own_prod = len(word.findall(texts[name]["prod"])) - 1
        it.update(prod_users=prod_users, test_users=test_users, own_prod_refs=own_prod)
    result[name] = items

OUT.write_text(json.dumps(result, indent=1, default=str), encoding="utf-8")
total = sum(len([i for i in v if i["kind"] != "mod"]) for v in result.values())
print(f"{total} public items in {len(result)} crates")
for name, items in result.items():
    lone = [i for i in items if i["kind"] != "mod" and not i["prod_users"]]
    if not lone:
        continue
    named = len([i for i in items if i["kind"] != "mod"])
    print(f"\n{name}: {len(lone)} of {named} pub items no other crate's production code names")
    for i in lone:
        where = ("tests: " + ",".join(i["test_users"])) if i["test_users"] else "NO USE OUTSIDE"
        print(f"  {i['kind']:8} {i['path']:60} own={i['own_prod_refs']:<3} {where}")
