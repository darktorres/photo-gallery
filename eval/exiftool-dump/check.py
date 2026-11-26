"""Cross-check the dump against ExifTool's own tag list.

usage: python3 check.py DUMP_DIR LIST_FILE
LIST_FILE is the output of `exiftool -config '' -list` (every tag name ExifTool knows, all tables).
Prints the names on each side only, and the dump's totals.
"""

import json
import pathlib
import re
import sys


def main() -> None:
    dump, listing = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    names: set[str] = set()
    tables: set[str] = set()
    defs = 0
    flat = 0
    for path in sorted(dump.glob("*.jsonl")):
        if path.name == "code.jsonl":
            continue
        for line in path.open(encoding="utf-8"):
            rec = json.loads(line)
            if rec["kind"] == "shortcut":
                names.add(rec["name"])
            elif rec["kind"] == "table":
                tables.add(rec["name"])
            elif rec["kind"] == "tag":
                defs += 1
                # ExifTool marks a flattened structure field with Flat => 0 (XMP.pm 3249): the key, not its value
                flat += 1 if "Flat" in rec["fields"] else 0
                name = rec["fields"].get("Name")
                if isinstance(name, str):
                    names.add(name)
    text = listing.read_text(encoding="utf-8", errors="replace")
    # the list follows its heading line ("Available tags:"); names are space-separated words
    body = text.split(":", 1)[1] if ":" in text else text
    listed = set(re.findall(r"[A-Za-z0-9_-]+", body))
    print(f"tables {len(tables)}, definitions {defs} ({flat} flattened), distinct names {len(names)}")
    print(f"listed by -list: {len(listed)}")
    only_dump = sorted(names - listed)
    only_list = sorted(listed - names)
    print(f"in the dump only: {len(only_dump)}: {' '.join(only_dump[:40])}")
    print(f"in -list only: {len(only_list)}: {' '.join(only_list[:40])}")


if __name__ == "__main__":
    main()
