#!/usr/bin/env python3
"""Deletes the pairs of the owed pictures from a catalog, so the pairing runs find them owed again (keep.ps1 and oracle.ps1
run it before each run). Prints how many pairs it deleted.

Usage: python -I drop_owed_pairs.py <catalog.sqlite>
"""

import sqlite3
import sys

conn = sqlite3.connect(sys.argv[1])
# The two pairing tables are the catalog's (crates/catalog/src/baseline.sql: `pdq_pairs` and `pairs_owed`). A catalog
# without them is another schema, and the delete would fail with a bare "no such table", so say so instead.
tables = {row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type = 'table'")}
missing = sorted({"pdq_pairs", "pairs_owed"} - tables)
if missing:
    sys.exit(
        f"{sys.argv[1]}: no {', '.join(missing)} table, so this catalog's pairing schema is not the one this script drops from"
    )
conn.execute("PRAGMA foreign_keys = ON")
deleted = conn.execute(
    "DELETE FROM pdq_pairs WHERE a IN (SELECT file_id FROM pairs_owed) OR b IN (SELECT file_id FROM pairs_owed)"
).rowcount
conn.commit()
print(f"deleted {deleted} pairs of the owed pictures")
