"""Make exactly N eligible pictures owe their pairs in a scratch copy of a catalog, the others not.

Usage: python -I set_owed.py <catalog.sqlite> <N>

The owed pictures are spread over the ids (a multiplicative hash of the id orders them), so N owed pictures are not one
folder's. Works on both layouts: the draft v12 column (`analysis.dups_state`) and the v13 queue table (`pairs_owed`).
Never run it on the live catalog: it rewrites what is owed.
"""
import sqlite3
import sys

ELIGIBLE = "pdq_dihedral IS NOT NULL AND pdq_quality >= 50"
SPREAD = "ORDER BY (file_id * 2654435761) % 1000003"


def main() -> None:
    path, n = sys.argv[1], int(sys.argv[2])
    con = sqlite3.connect(path, isolation_level=None)
    con.execute("PRAGMA journal_mode = WAL")
    con.execute("PRAGMA foreign_keys = ON")
    tables = {r[0] for r in con.execute("SELECT name FROM sqlite_schema WHERE type = 'table'")}
    con.execute("BEGIN IMMEDIATE")
    if "pairs_owed" in tables:
        con.execute("DELETE FROM pairs_owed")
        con.execute(f"INSERT INTO pairs_owed SELECT file_id FROM analysis WHERE {ELIGIBLE} {SPREAD} LIMIT ?", (n,))
        owed = con.execute("SELECT count(*) FROM pairs_owed").fetchone()[0]
    else:
        con.execute("UPDATE analysis SET dups_state = 'done' WHERE dups_state = 'due'")
        con.execute(
            f"UPDATE analysis SET dups_state = 'due' WHERE file_id IN "
            f"(SELECT file_id FROM analysis WHERE {ELIGIBLE} {SPREAD} LIMIT ?)",
            (n,),
        )
        owed = con.execute("SELECT count(*) FROM analysis WHERE dups_state = 'due'").fetchone()[0]
    con.execute("COMMIT")
    con.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    con.close()
    print(owed, "owed")


if __name__ == "__main__":
    main()
