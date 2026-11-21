"""Makes exactly N eligible pictures owe their pairs in a scratch copy of a catalog, the others not.

Usage: python -I set_owed.py <catalog.sqlite> <N>

The owed pictures are spread over the ids (a multiplicative hash of the id orders them), so N owed pictures are not one
folder's. It works on the catalog's `pairs_owed` queue (crates/catalog/src/baseline.sql); a catalog without that table is
another schema, and is refused.
Never run it on the live catalog: it rewrites what is owed.
"""

import sqlite3
import sys

# The N owed: eligible pictures (a hash that is there and good enough to pair), spread over the ids by a multiplicative
# hash of the id.
OWE_QUEUE = (
    "INSERT INTO pairs_owed SELECT file_id FROM analysis WHERE pdq_dihedral IS NOT NULL AND pdq_quality >= 50 "
    "ORDER BY (file_id * 2654435761) % 1000003 LIMIT ?"
)


def main() -> None:
    path, n = sys.argv[1], int(sys.argv[2])
    con = sqlite3.connect(path, isolation_level=None)
    con.execute("PRAGMA journal_mode = WAL")
    con.execute("PRAGMA foreign_keys = ON")
    tables = {r[0] for r in con.execute("SELECT name FROM sqlite_schema WHERE type = 'table'")}
    if "pairs_owed" not in tables:
        sys.exit(f"{path} has no pairs_owed table: it is another schema (crates/catalog/src/baseline.sql)")
    con.execute("BEGIN IMMEDIATE")
    con.execute("DELETE FROM pairs_owed")
    con.execute(OWE_QUEUE, (n,))
    owed = con.execute("SELECT count(*) FROM pairs_owed").fetchone()[0]
    con.execute("COMMIT")
    con.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    con.close()
    print(owed, "owed")


if __name__ == "__main__":
    main()
