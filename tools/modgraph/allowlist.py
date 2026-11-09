"""The module-cycle allowlist (docs/MODULARITY_PLAN.md, M-0 and M-16): which cycle groups are known and still to be fixed.

`tools/modgraph-allow.txt` has one entry per known group, `source crate: member, member, ...`:

- `source` is `scan` (the text scan, `module_graph.py`, which is the same on every machine), `linux` or `windows`
  (the compiler's module graph, `compiler_graph.py`, on that target). A group that exists on one target only is listed
  under that target alone.
- `crate` is the crate's directory name without `pg-` (`desktop`, `app`, ...), as the scan names it.
- `members` are the modules in the group, as the scan labels them (`(crate root)` for the root).

A group passes if an entry with the same source, crate and member set exists. A group that is not on the list fails
the check. An entry that matches no group also fails, so a fixed cycle must leave the list in the same commit.
"""

import os
import sys
from pathlib import Path

ALLOWLIST = Path(__file__).resolve().parents[1] / "modgraph-allow.txt"


def target_source():
    """The compiler's source name on this machine: `windows` or `linux`."""
    if os.name == "nt":
        return "windows"
    if sys.platform.startswith("linux"):
        return "linux"
    raise RuntimeError(f"no module-graph target for {sys.platform}")


def load(path=ALLOWLIST):
    """The entries as {(source, crate): [member sets]}."""
    entries = {}
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.split("#", 1)[0].strip()  # a comment starts at `#`; no module name contains one
        if not line:
            continue
        head, _, rest = line.partition(":")
        source, _, crate = head.strip().partition(" ")
        if source not in ("scan", "linux", "windows") or not crate.strip() or not rest.strip():
            raise ValueError(f"{path}:{number}: expected `source crate: member, member`, got {raw!r}")
        members = frozenset(m.strip() for m in rest.split(","))
        entries.setdefault((source, crate.strip()), []).append(members)
    return entries


def check(source, crate, groups, entries):
    """Compare one crate's groups (each an iterable of member names) with the list.

    Returns (unlisted, stale): the groups not on the list, and the listed member sets that match no group.
    """
    found = [frozenset(g) for g in groups]
    listed = entries.get((source, crate), [])
    unlisted = [g for g in found if g not in listed]
    stale = [m for m in listed if m not in found]
    return unlisted, stale


def check_all(source, groups_by_crate, path=ALLOWLIST):
    """Check every crate for one source: `groups_by_crate` maps crate name to its groups. Prints each failure and
    returns how many there are. A crate with entries but no groups counts its entries as stale."""
    entries = load(path)
    crates = set(groups_by_crate) | {crate for (src, crate) in entries if src == source}
    failures = 0
    for crate in sorted(crates):
        unlisted, stale = check(source, crate, groups_by_crate.get(crate, []), entries)
        for g in unlisted:
            print(f"  NOT ALLOWED ({source} {crate}): " + ", ".join(sorted(g)))
        for m in stale:
            print(f"  STALE ENTRY ({source} {crate}), remove from {path.name}: " + ", ".join(sorted(m)))
        failures += len(unlisted) + len(stale)
    return failures
