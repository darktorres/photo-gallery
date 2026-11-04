#!/usr/bin/env python3
"""Callgrind output as tables: self and inclusive cost and call counts, and diffs of two runs.

Reads the raw format (https://valgrind.org/docs/manual/cl-format.html) instead of scraping callgrind_annotate,
so the columns don't depend on --show and the call counts come from the `calls=` lines.

    cgtab.py table FILE... [--by fn|crate|file|obj] [--sort self|incl|calls] [--top N] [--match RE] [--tsv]
    cgtab.py calls FILE... FUNC_RE          callers and callees of the matching functions
    cgtab.py diff BASE TIP [--rename OLD=NEW]... [--top N] [--tsv]

FILE is a callgrind.out; several files (threads, parts) are summed. gungraun keeps the previous run beside the
current one as `*.out.old`, so `diff X.out.old X.out` shows what the last change did.

Inclusive cost is self cost plus the cost of the calls the function makes, leaving out direct recursion.
Indirect recursion (A -> B -> A) is still counted twice, as in callgrind_annotate. Callgrind charges inlined code
to the function it was inlined into. The `crate` and `file` groupings follow the source file instead, through
fi=/fe=, so code from fast_image_resize inlined into pg_codec counts as fast_image_resize.
"""

import argparse
import re
import sys
from collections import defaultdict
from pathlib import PurePosixPath

# fn and cfn share one id space for compressed names, ob and cob another, and the file keys a third.
NAME_SPACE = {
    "ob": "ob",
    "cob": "ob",
    "fl": "fl",
    "fi": "fl",
    "fe": "fl",
    "cfi": "fl",
    "cfl": "fl",
    "fn": "fn",
    "cfn": "fn",
}
COMPRESSED = re.compile(r"^\((\d+)\)(?: (.*))?$")
BUILD_HASH = re.compile(r"-[0-9a-f]{16}$")  # cargo's target/*/deps/<name>-<hash>
RUST_HASH = re.compile(r"::h[0-9a-f]{16}$")  # legacy-mangled symbols
NO_SYMBOL = re.compile(r"^0x[0-9a-f]+$")


class Profile:
    def __init__(self):
        self.events = []
        self.self_fn = defaultdict(list)  # (ob, fn) -> costs
        self.self_src = defaultdict(list)  # (ob, fn, file the code came from) -> costs
        self.edges = defaultdict(list)  # ((ob, fn), (ob, fn)) -> costs of those calls
        self.edge_calls = defaultdict(int)  # same key -> number of calls

    def load(self, path):
        names = {"ob": {}, "fl": {}, "fn": {}}
        npos = 1
        ob = fl = fi = fn = "???"
        c_ob = c_fn = None
        pending = None  # call count of a calls= line; its cost line comes next
        file_events = None

        def name(key, value):
            m = COMPRESSED.match(value)
            if not m:
                return value
            table = names[NAME_SPACE[key]]
            if m.group(2) is not None:
                table[m.group(1)] = m.group(2)
            return table[m.group(1)]

        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                line = line.rstrip("\n")
                if not line or line[0] == "#":
                    continue
                c = line[0]
                if c.isdigit() or c in "+-*":
                    tokens = line.split()
                    costs = [int(t) for t in tokens[npos:]]
                    if pending is not None:
                        key = ((ob, fn), (c_ob or ob, c_fn or fn))
                        add(self.edges[key], costs)
                        self.edge_calls[key] += pending
                        pending = c_ob = c_fn = None
                    else:
                        add(self.self_fn[(ob, fn)], costs)
                        add(self.self_src[(ob, fn, fi)], costs)
                    continue
                key, sep, value = line.partition("=")
                if sep and key in NAME_SPACE:
                    v = name(key, value)
                    if key == "ob":
                        ob = v
                    elif key == "fl":
                        fl = fi = v
                    elif key in ("fi", "fe"):
                        fi = v
                    elif key == "fn":
                        fn = v
                        fi = fl
                    elif key == "cob":
                        c_ob = v
                    elif key == "cfn":
                        c_fn = v
                    continue  # cfi/cfl: the callee's file isn't needed
                if sep and key == "calls":
                    pending = int(value.split()[0])
                    continue
                if sep and key in ("jump", "jcnd"):
                    continue
                key, sep, value = line.partition(":")
                if not sep:
                    continue
                if key == "events":
                    file_events = value.split()
                    if self.events and self.events != file_events:
                        sys.exit(f"{path}: events {file_events} differ from {self.events}")
                    self.events = file_events
                elif key == "positions":
                    npos = len(value.split())
        if file_events is None:
            sys.exit(f"{path}: no events: line, not a callgrind output?")

    def event_index(self, event):
        if event not in self.events:
            sys.exit(f"event {event} not in {' '.join(self.events)}")
        return self.events.index(event)

    def functions(self, e):
        """(ob, fn) -> [self, inclusive, calls] for event index e."""
        rows = defaultdict(lambda: [0, 0, 0])
        for k, costs in self.self_fn.items():
            v = cost(costs, e)
            rows[k][0] += v
            rows[k][1] += v
        for (caller, callee), costs in self.edges.items():
            if caller != callee:
                rows[caller][1] += cost(costs, e)
            rows[callee][2] += self.edge_calls[(caller, callee)]
        return rows

    def total(self, e):
        return sum(cost(c, e) for c in self.self_fn.values())


def add(acc, costs):
    if len(acc) < len(costs):
        acc.extend([0] * (len(costs) - len(acc)))
    for i, v in enumerate(costs):
        acc[i] += v


def cost(costs, e):
    return costs[e] if e < len(costs) else 0


def short_obj(ob):
    return BUILD_HASH.sub("", PurePosixPath(ob).name) if ob != "???" else ob


def short_fn(fn):
    return RUST_HASH.sub("", fn)


def short_file(path):
    if "//" in path:  # "simd/x86_64///root/..." as nasm sources come out
        path = "/" + path.rsplit("//", 1)[1]
    path = re.sub(r"^.*/registry/src/[^/]+/", "", path)
    path = re.sub(r"^/rustc/[0-9a-f]+/library/", "std/", path)
    path = re.sub(r"^/root/[^/]+/", "", path)  # the repo clone
    return path


def crate_of_fn(fn):
    """The crate a Rust symbol belongs to: `a::b`, `<a::T>::f`, `<a::T as b::Tr>::f` -> a."""
    m = re.match(r"^<*(?:&(?:mut )?)?([A-Za-z_][A-Za-z0-9_]*)::", fn)
    return m.group(1) if m else None


def crate_of(path, ob, fn):
    p = short_file(path)
    m = re.match(r"^([A-Za-z0-9_\-]+)-\d+\.\d+\.\d+[^/]*/", p)  # registry crate dir
    if m:
        return m.group(1).replace("-", "_")
    if p.startswith("std/"):
        # Inlined std code (iterators, SIMD intrinsics from core_arch, slices) belongs to the crate whose function
        # it was inlined into: fast_image_resize's AVX2 kernels are mostly core_arch lines.
        owner = crate_of_fn(fn)
        if owner and owner not in ("std", "core", "alloc", "__rustc"):
            return owner
        return "std"
    m = re.search(r"(libjpeg-turbo|ffmpeg)", p)
    if m:
        return m.group(1)
    m = re.match(r"^crates/([^/]+)/", p)
    if m:
        return f"pg_{m.group(1)}"  # crates/codec -> pg_codec, as the symbols name it
    if p.startswith("benches/") or "/benches/" in p:
        return "bench harness"
    return short_obj(ob)  # glibc, ld.so, FFmpeg's shared libs, no debug info


def load(paths):
    prof = Profile()
    for p in paths:
        prof.load(p)
    return prof


def pct(v, total):
    return 100.0 * v / total if total else 0.0


def cmd_table(args):
    prof = load(args.files)
    e = prof.event_index(args.event)
    total = prof.total(e)
    if args.by == "fn":
        rows = [(short_fn(fn), short_obj(ob), *vals) for (ob, fn), vals in prof.functions(e).items()]
    else:
        groups = defaultdict(int)
        if args.by == "obj":
            for (ob, _), costs in prof.self_fn.items():
                groups[short_obj(ob)] += cost(costs, e)
        else:
            for (ob, fn, src), costs in prof.self_src.items():
                k = short_file(src) if args.by == "file" else crate_of(src, ob, fn)
                groups[k] += cost(costs, e)
        rows = [(k, "", v, None, None) for k, v in groups.items()]
    if args.match:
        rx = re.compile(args.match)
        rows = [r for r in rows if rx.search(r[0])]
    col = {"self": 2, "incl": 3, "calls": 4}[args.sort]
    if args.by != "fn" and col != 2:
        sys.exit("--sort incl/calls only with --by fn")
    rows.sort(key=lambda r: (-(r[col] or 0), r[0]))
    if args.tsv:
        print("self\tinclusive\tcalls\tname\tobject")
        for name_, ob, s, i, n in rows:
            print(f"{s}\t{'' if i is None else i}\t{'' if n is None else n}\t{name_}\t{ob}")
        return
    print(f"{args.event} total: {total:,}  ({', '.join(args.files)})")
    if args.by == "fn":
        print(f"{'self':>14} {'%':>6} {'inclusive':>14} {'%':>6} {'calls':>10}  function [object]")
    else:
        print(f"{'self':>14} {'%':>6}  {args.by}")
    for name_, ob, s, i, n in rows[: args.top]:
        if args.by == "fn":
            print(f"{s:>14,} {pct(s, total):>6.2f} {i:>14,} {pct(i, total):>6.2f} {n:>10,}  {name_} [{ob}]")
        else:
            print(f"{s:>14,} {pct(s, total):>6.2f}  {name_}")


def cmd_calls(args):
    prof = load(args.files)
    e = prof.event_index(args.event)
    total = prof.total(e)
    funcs = prof.functions(e)
    rx = re.compile(args.func)
    matched = sorted((k for k in funcs if rx.search(k[1])), key=lambda k: -funcs[k][1])
    if not matched:
        sys.exit(f"no function matches {args.func!r}")
    for k in matched[: args.top]:
        s, i, n = funcs[k]
        print(
            f"* {short_fn(k[1])} [{short_obj(k[0])}]: self {s:,}, inclusive {i:,} ({pct(i, total):.2f}%), "
            f"{n:,} calls" + (f", {i / n:,.0f} per call" if n else "")
        )
        callers = [(c, v) for (c, g), v in prof.edges.items() if g == k]
        callees = [(g, v) for (c, g), v in prof.edges.items() if c == k]
        for arrow, edges, idx in (("<", callers, 0), (">", callees, 1)):
            for other, costs in sorted(edges, key=lambda x: -cost(x[1], e)):
                key = (other, k) if idx == 0 else (k, other)
                n_calls = prof.edge_calls[key]
                v = cost(costs, e)
                print(f"  {arrow} {v:>14,} {pct(v, total):>6.2f}% {n_calls:>10,}x  {short_fn(other[1])} [{short_obj(other[0])}]")
        print()


def by_name(prof, e, renames):
    """Function name -> [self, inclusive, calls], merged across objects (build hashes differ between runs)."""
    rows = defaultdict(lambda: [0, 0, 0])
    for (ob, fn), vals in prof.functions(e).items():
        # Code without a symbol is named by its address, which moves between builds.
        name_ = f"(no symbol) [{short_obj(ob)}]" if NO_SYMBOL.match(fn) else short_fn(fn)
        for old, new in renames:
            name_ = name_.replace(old, new)
        for i, v in enumerate(vals):
            rows[name_][i] += v
    return rows


def cmd_diff(args):
    renames = []
    for r in args.rename:
        old, sep, new = r.partition("=")
        if not sep:
            sys.exit(f"--rename wants OLD=NEW, got {r!r}")
        renames.append((old, new))
    base, tip = load([args.base]), load([args.tip])
    be, te = base.event_index(args.event), tip.event_index(args.event)
    b, t = by_name(base, be, renames), by_name(tip, te, [])
    bt, tt = base.total(be), tip.total(te)
    rows = []
    zero = [0, 0, 0]
    for k in set(b) | set(t):
        (bs, bi, bn), (ts, ti, tn) = b.get(k, zero), t.get(k, zero)
        if (bs, bi, bn) != (ts, ti, tn):
            rows.append((ts - bs, ti - bi, bs, ts, bi, ti, bn, tn, k))
    rows.sort(key=lambda r: (-abs(r[0]), -abs(r[1]), r[-1]))
    if args.tsv:
        print("d_self\td_inclusive\tbase_self\ttip_self\tbase_inclusive\ttip_inclusive\tbase_calls\ttip_calls\tfunction")
        for r in rows:
            print("\t".join(str(x) for x in r))
        return
    d = tt - bt
    print(f"{args.event} total: {bt:,} -> {tt:,}  ({d:+,}, {pct(d, bt):+.3f}%)")
    if not rows:
        print("no function changed")
        return
    print(f"{'Δself':>13} {'Δinclusive':>13} {'base self':>13} {'tip self':>13} {'calls':>19}  function")
    for ds, di, bs, ts, _bi, _ti, bn, tn, k in rows[: args.top]:
        calls = f"{bn:,}" if bn == tn else f"{bn:,}->{tn:,}"
        print(f"{ds:>+13,} {di:>+13,} {bs:>13,} {ts:>13,} {calls:>19}  {k}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--event", default="Ir", help="event column (default Ir)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    t = sub.add_parser("table", help="self / inclusive / calls per function, or self cost per crate, file, object")
    t.add_argument("files", nargs="+")
    t.add_argument("--by", choices=["fn", "crate", "file", "obj"], default="fn")
    t.add_argument("--sort", choices=["self", "incl", "calls"], default="self")
    t.add_argument("--top", type=int, default=30)
    t.add_argument("--match", help="regex on the function (or group) name")
    t.add_argument("--tsv", action="store_true", help="every row, tab-separated")
    t.set_defaults(run=cmd_table)

    c = sub.add_parser("calls", help="callers (<) and callees (>) of functions matching a regex")
    c.add_argument("files", nargs="+", help="callgrind outputs, then the function regex last")
    c.add_argument("--top", type=int, default=5, help="at most this many matching functions")
    c.set_defaults(run=cmd_calls)

    d = sub.add_parser("diff", help="per-function change from BASE to TIP")
    d.add_argument("base")
    d.add_argument("tip")
    d.add_argument("--rename", action="append", default=[], help="OLD=NEW, applied to BASE's function names")
    d.add_argument("--top", type=int, default=30)
    d.add_argument("--tsv", action="store_true")
    d.set_defaults(run=cmd_diff)

    args = ap.parse_args()
    if args.cmd == "calls":
        if len(args.files) < 2:
            ap.error("calls wants FILE... FUNC_RE")
        args.files, args.func = args.files[:-1], args.files[-1]
    args.run(args)


if __name__ == "__main__":
    try:
        main()
    except BrokenPipeError:
        sys.stderr.close()
