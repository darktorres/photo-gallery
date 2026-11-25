#!/usr/bin/env python3
"""Compares two catalogs table by table: every table's row count side by side, and whether they agree. Used to check that the
headless driver's run (`pg-driver run`) makes the same catalog as the command line it replaced, on the same library.

    python tools/compare_catalogs.py OLD/catalog.sqlite NEW/catalog.sqlite

Exit code 0 when every table has the same row count in both, 1 when any differs. The catalogs are opened read-only."""

import sqlite3
import sys


def counts(path):
    conn = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    try:
        names = [
            row[0]
            for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
            )
        ]
        return {name: conn.execute(f'SELECT COUNT(*) FROM "{name}"').fetchone()[0] for name in names}
    finally:
        conn.close()


def main(old_path, new_path):
    old, new = counts(old_path), counts(new_path)
    differ = False
    for name in sorted(set(old) | set(new)):
        a, b = old.get(name), new.get(name)
        mark = "" if a == b else "   <-- differs"
        differ = differ or a != b
        print(f"{name:32} old={a!s:>8} new={b!s:>8}{mark}")
    print("the catalogs agree on every table's row count" if not differ else "the catalogs differ")
    return 1 if differ else 0


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    sys.exit(main(sys.argv[1], sys.argv[2]))
