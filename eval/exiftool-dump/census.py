"""The classifier census (docs/plans/EXIFTOOL_RUST_REWRITE_DESIGN.md, "ExifTool's knowledge", step 2).

usage: python3 census.py DUMP_DIR [REPORT]

Sorts every condition and conversion of the dump into the typed shapes the generator would accept, and counts what
is left for hand ports. A shape is recognized on the whole string (after whitespace is normalized), never on part
of it, so an expression counts as classified only if all of it is. Conditions are split on and/or/not and
parentheses first: a condition is classified if every atom is one of the atom shapes.

The shapes are deliberately the simple, safe ones; anything else is "hand port". The report lists the shape
counts per field (all occurrences and distinct texts), the most frequent unclassified texts, the subs that strings
call by name, and what the value tables carry besides values (OTHER handlers, BITMASK, ...).
"""

import collections
import json
import pathlib
import re
import sys

STRING_FIELDS = ("Condition", "RawConv", "ValueConv", "PrintConv", "RawConvInv", "ValueConvInv", "PrintConvInv", "Hook")

# --- atoms --------------------------------------------------------------------------------------------------------
NUM = r"[-+]?(?:0x[0-9a-fA-F]+|\d+(?:\.\d*)?(?:[eE][-+]?\d+)?|\.\d+)"
STR = r"(?:'(?:[^'\\]|\\.)*'|\"(?:[^\"\\$@]|\\.)*\")"
RE = r"(?:m?/(?:[^/\\]|\\.)*/[a-z]*|m\{(?:[^}\\]|\\.)*\}[a-z]*|m\|(?:[^|\\]|\\.)*\|[a-z]*|m#(?:[^#\\]|\\.)*#[a-z]*)"
CMP = r"(?:eq|ne|lt|gt|le|ge|==|!=|<=|>=|<|>)"
SELF = r"(?:\$\$self\{|\$self->\{)(?:\w+|'\w+')\}(?:\{(?:\w+|'\w+')\})?"  # $$self{X}, $self->{X}, $$self{OPTIONS}{Y}
LOCAL = r"\$(?:count|format|index|size|val)\b(?:\[\d+\])?"  # what a condition or conversion is given
VALPT = r"\$\$valPt"

# Literals are masked before a condition is split (a regex's own parentheses and "or" bars must not split it):
# a regex literal after =~ or !~ becomes \x01R<n>\x02, a string literal \x01S<n>\x02.
TRE = r"\x01R\d+\x02"
TSTR = r"\x01S\d+\x02"
MASK_RE = re.compile(rf"(=~|!~)(\s*)({RE})")
MASK_STR = re.compile(STR)

ATOMS = {
    "regex on context": rf"{SELF}\s*(?:=~|!~)\s*{TRE}",
    "compare context": rf"(?:{SELF}\s*{CMP}\s*(?:{NUM}|{TSTR})|(?:{NUM}|{TSTR})\s*{CMP}\s*{SELF})",
    "context set": rf"(?:defined\s*\(?\s*{SELF}\s*\)?|{SELF})",
    "regex on bytes": rf"{VALPT}\s*(?:=~|!~)\s*{TRE}",
    "compare bytes": rf"{VALPT}\s*(?:eq|ne)\s*{TSTR}",
    "bytes at an offset": rf"substr\s*\(\s*{VALPT}\s*,\s*{NUM}\s*,\s*{NUM}\s*\)\s*(?:eq|ne)\s*{TSTR}",
    "length of bytes": rf"length\s*\(?\s*{VALPT}\s*\)?\s*{CMP}\s*{NUM}",
    "regex on value": rf"{LOCAL}\s*(?:=~|!~)\s*{TRE}",
    "compare local": rf"(?:{LOCAL}\s*{CMP}\s*(?:{NUM}|{TSTR}|{SELF})|(?:{NUM}|{TSTR})\s*{CMP}\s*{LOCAL})",
    "local set": rf"(?:defined\s*\(?\s*{LOCAL}\s*\)?|{LOCAL})",
    "option": r"\x01O\d+\x02",
    "bit test on context": rf"{SELF}\s*&\s*{NUM}",
    "bit test on value": rf"{LOCAL}\s*&\s*{NUM}",
}
MASK_OPTION = re.compile(rf"\$(?:self|et)->Options\(\s*{TSTR}\s*\)")
ATOM_RES = {k: re.compile(rf"^(?:{v})$") for k, v in ATOMS.items()}
SPLIT = re.compile(r"\s*(?:\band\b|\bor\b|&&|\|\||\bnot\b|!(?![=~])|\(|\))\s*")


def mask(text: str) -> tuple[str, list[str]]:
    """Mask regex literals (after =~ or !~) and string literals; returns the masked text and the literals."""
    lits: list[str] = []

    def keep(kind: str, lit: str) -> str:
        lits.append(lit)
        return f"\x01{kind}{len(lits) - 1}\x02"

    text = MASK_RE.sub(lambda m: m.group(1) + m.group(2) + keep("R", m.group(3)), text)
    text = MASK_STR.sub(lambda m: keep("S", m.group(0)), text)
    # an option call, whose own parentheses must not split the condition
    return MASK_OPTION.sub(lambda m: keep("O", m.group(0)), text), lits


def unmask(text: str, lits: list[str]) -> str:
    while "\x01" in text:
        text = re.sub(r"\x01[RSO](\d+)\x02", lambda m: lits[int(m.group(1))], text)
    return text


def split_ternary(masked: str) -> tuple[str, str, str] | None:
    """Split a top-level `c ? a : b` (outside parentheses), or None."""
    depth, q = 0, -1
    nested = 0
    for i, ch in enumerate(masked):
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        elif depth == 0 and ch == "?":
            if q < 0:
                q = i
            else:
                nested += 1
        elif depth == 0 and ch == ":" and q >= 0 and masked[i - 1 : i + 1] != "::" and masked[i : i + 2] != "::":
            if nested:
                nested -= 1
            else:
                return masked[:q].strip(), masked[q + 1 : i].strip(), masked[i + 1 :].strip()
    return None


# --- conversions (whole string) -----------------------------------------------------------------------------------
FUNCS = r"(?:int|abs|log|exp|sqrt|sin|cos|atan2|hex|oct)"
FORMULA_ATOM = rf"(?:\$val(?:\[\d+\])?|{NUM}|{FUNCS}\s*\(|\(|\)|,|\*\*|[-+*/%])"
FORMULA = re.compile(rf"^(?:\s*{FORMULA_ATOM}\s*)+$")
SPRINTF = re.compile(rf"^sprintf\s*\(\s*{STR}\s*(?:,\s*(?:\s*{FORMULA_ATOM}\s*)+)?\)$")
INTERP = re.compile(r"^\"(?:[^\"\\$@]|\\.)*\$val(?:[^\"\\$@]|\\.)*\"$")  # "$val mm"
SUBST_S = r"s/(?:[^/\\]|\\.)*/(?:[^/\\$]|\\.)*/[a-z]*"
SUBST_TR = r"tr/(?:[^/\\]|\\.)*/(?:[^/\\]|\\.)*/[a-z]*"
SUBST = re.compile(rf"^\$val\s*=~\s*(?:{SUBST_S}|{SUBST_TR})\s*;\s*\$val$")
CALL_ARGS = rf"(?:\$val(?:\[\d+\])?|\$self|\$et|\@val|\\\$val|{NUM}|{STR}|\s|,)*"
CALL = re.compile(rf"^(?:\$self->|\$et->|&?)([A-Z][\w:]*)\s*\(\s*{CALL_ARGS}\)$")
STORE = re.compile(rf"^(?:{SELF}\s*=\s*\$val\s*;?\s*)+(?:\$val|undef)?$")
IDENT = re.compile(r"^\$val$")
CONST = re.compile(rf"^(?:{NUM}|{STR}|undef)$")

CONV_SHAPES = [
    ("identity", IDENT),
    ("constant", CONST),
    ("formula", FORMULA),
    ("sprintf of formula", SPRINTF),
    ("interpolated text", INTERP),
    ("substitution", SUBST),
    ("call of a named sub", CALL),
    ("store into context", STORE),
]

CALLED = re.compile(r"(?:\$self->|\$et->|\b)((?:Image::ExifTool::)?[A-Z][\w]*(?:::[A-Z]\w*)*)\s*\(")


def normalize(text: str) -> str:
    text = re.sub(r"#[^\n]*", "", text) if "\n" in text else text  # comments only on multi-line code
    text = re.sub(r"\s+", " ", text).strip()
    return re.sub(r"\s*;\s*$", "", text)


def classify_masked_condition(masked: str) -> str:
    atoms = [a for a in SPLIT.split(masked) if a.strip()]
    if not atoms:
        return "hand port"
    kinds = set()
    for atom in atoms:
        for name, pat in ATOM_RES.items():
            if pat.match(atom.strip()):
                kinds.add(name)
                break
        else:
            return "hand port"
    return "predicate: " + ("combined" if len(atoms) > 1 else kinds.pop())


def classify_condition(text: str) -> str:
    return classify_masked_condition(mask(text)[0])


def classify_conversion(text: str, depth: int = 0) -> str:
    for name, pat in CONV_SHAPES:
        if pat.match(text):
            return name
    masked, lits = mask(text)
    parts = split_ternary(masked) if depth < 3 else None
    if parts:
        cond, yes, no = parts
        if cond.startswith("(") and cond.endswith(")"):
            cond = cond[1:-1]
        if (
            classify_masked_condition(cond) != "hand port"
            and classify_conversion(unmask(yes, lits), depth + 1) != "hand port"
            and classify_conversion(unmask(no, lits), depth + 1) != "hand port"
        ):
            return "ternary on a predicate"
    return "hand port"


def main() -> None:
    dump = pathlib.Path(sys.argv[1])
    report_path = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else None
    shared: dict[str, object] = {}
    tags = []
    for path in sorted(dump.glob("*.jsonl")):
        if path.name in ("code.jsonl", "shortcuts.jsonl"):
            continue
        for line in path.open(encoding="utf-8"):
            rec = json.loads(line)
            if rec["kind"] == "shared":
                shared[rec["id"]] = rec["value"]
            elif rec["kind"] == "tag":
                tags.append(rec)

    def resolve(v):
        while isinstance(v, dict) and set(v) == {"ref"}:
            v = shared[v["ref"]]
        return v

    shape_all: dict[str, collections.Counter] = collections.defaultdict(collections.Counter)
    shape_distinct: dict[str, dict[str, set]] = collections.defaultdict(lambda: collections.defaultdict(set))
    unclassified: dict[str, collections.Counter] = collections.defaultdict(collections.Counter)
    called = collections.Counter()
    table_extras = collections.Counter()
    regex_features = collections.Counter()
    for rec in tags:
        fields = rec["fields"]
        for field in STRING_FIELDS:
            if field not in fields:
                continue
            v = fields[field]
            if isinstance(v, dict) and "u" in v:
                v = v["u"]
            if not isinstance(v, str):
                if field.startswith("PrintConv"):
                    pv = resolve(v)
                    if isinstance(pv, dict) and "code" not in pv:
                        for key in ("OTHER", "BITMASK", "PrintHex", "PrintString", "PrintSort", "Notes"):
                            if key in pv:
                                table_extras[key] += 1
                continue
            text = normalize(v)
            kind = classify_condition(text) if field in ("Condition", "Hook") else classify_conversion(text)
            if field == "Hook":
                kind = "hand port"  # a hook is code run while a binary table is read: always ported
            shape_all[field][kind] += 1
            shape_distinct[field][kind].add(text)
            if kind == "hand port":
                unclassified[field][text] += 1
            for name in CALLED.findall(text):
                if name not in ("Get8u", "Get16u", "Get32u", "Get16s", "Get32s"):
                    called[name] += 1
            if re.search(r"\(\?[=!<]|\\[1-9]|\(\?\{", text):
                regex_features["lookaround, backreference or code in a regex"] += 1

    out = []
    total_all = total_hand = 0
    hand_texts: set[str] = set()
    for field in STRING_FIELDS:
        if field not in shape_all:
            continue
        n = sum(shape_all[field].values())
        d = len(set().union(*shape_distinct[field].values()))
        out.append(f"\n## {field}: {n} strings, {d} distinct")
        for kind, count in shape_all[field].most_common():
            out.append(f"  {kind:<32} {count:>6}  ({len(shape_distinct[field][kind])} distinct)")
        total_all += n
        total_hand += shape_all[field]["hand port"]
        hand_texts |= shape_distinct[field]["hand port"]
    classified = total_all - total_hand
    hand_line = f"hand ports: {total_hand} occurrences, {len(hand_texts)} distinct texts"
    summary = [
        "# Classifier census of the dump",
        f"strings: {total_all}; classified: {classified} ({100 * classified / total_all:.1f}%); {hand_line}",
        f"value tables carrying more than values: {dict(table_extras)}",
        f"regex features beyond a plain engine: {dict(regex_features)}",
        f"subs called by name from strings: {len(called)} distinct, {sum(called.values())} calls",
    ]
    out = summary + out
    out.append("\n## Most called subs (from strings)")
    out += [f"  {c:>5}  {n}" for n, c in called.most_common(30)]
    for field in STRING_FIELDS:
        if unclassified[field]:
            out.append(f"\n## Most frequent hand ports: {field}")
            out += [f"  {c:>4}  {t[:150]}" for t, c in unclassified[field].most_common(15)]
    text = "\n".join(out) + "\n"
    if report_path:
        report_path.write_text(text, encoding="utf-8")
    print(text)


if __name__ == "__main__":
    main()
