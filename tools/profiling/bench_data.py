"""Prepare the catalog benches' data (crates/bench/benches/ir/catalog.rs) from a catalog folder.

Writes OUT/LAYOUT/catalog.sqlite (VACUUM INTO: compact, consistent with the WAL) and OUT/LAYOUT/thumbs.sqlite (the
cache's schema, its application id, entries, totals, limits, meta and checked, without the JPEGs). An existing LAYOUT
folder is never overwritten, so the data a baseline was measured on stays. The sources are opened read-only.

The data must be made by the build the benches run against: a catalog or cache of another schema is rebuilt by the
app's start (the bench's open refuses it). Bump the LAYOUT name in crates/bench/benches/ir/catalog.rs with each schema
change.

    python tools/profiling/bench_data.py v3-windows-rules \\
        $PG_ROOT\\synthetic\\catalog-v3 $PG_ROOT\\bench-data

The large dataset (F0's `scan-walk`, `import-flush` and `inbox-refresh` benches) is made the same way, under its own
layout name ending in `-large` (`v6-windows-rules-large` at schema v6), from `synthetic_library.py --profile large`
(200,000 files, 50,000 of them broken, so the import records 50,000 analysis failures):

    python tools/profiling/synthetic_library.py $PG_ROOT\\synthetic\\library-large --profile large
    gallery --catalog-dir $PG_ROOT\\synthetic\\catalog-large scan $PG_ROOT\\synthetic\\library-large
    gallery --catalog-dir $PG_ROOT\\synthetic\\catalog-large import
    gallery --catalog-dir $PG_ROOT\\synthetic\\catalog-large thumbs --height 256
    python tools/profiling/bench_data.py v6-windows-rules-large \\
        $PG_ROOT\\synthetic\\catalog-large $PG_ROOT\\bench-data

A `-large` layout also gets the failure ledger seeded (`seed_ledger`) to --ledger-rows rows (100,000 by default; 0 for
none; other layouts default to 0), when the copy's schema has the ledger the inbox reads: `file_failures` with its
`hidden` column (the notifications migration). An older schema is left as it is, with a note: its inbox does not read
the ledger that way, and its `inbox-refresh` is not measured on it.

`just perf` (tools/linux.py) mounts the bench data folder into the container, which copies it to its own
volume before the benches run.
"""

import argparse
import sqlite3
import sys
from pathlib import Path

# The ledger rows a `-large` layout is seeded to, with the import's own failures counted.
LARGE_LEDGER_ROWS = 100_000
# The seeded rows' fixed fields: the same data on every run.
SEED_ERROR = "bench data: seeded failure"
SEED_FIRST_SEEN = 1_767_225_600  # 2026-01-01 UTC


def read_only(path: Path) -> sqlite3.Connection:
    return sqlite3.connect(f"{path.resolve().as_uri()}?mode=ro", uri=True)


def slim_cache(source: Path, out: Path) -> None:
    """The cache without its `data` rows: what the pending check reads, and the file state the app checks on open."""
    src = read_only(source)
    schema = src.execute("SELECT type, sql FROM sqlite_schema WHERE sql IS NOT NULL").fetchall()
    user_version = src.execute("PRAGMA user_version").fetchone()[0]
    application_id = src.execute("PRAGMA application_id").fetchone()[0]
    src.close()

    # As a URI, so the source can be attached read-only.
    dst = sqlite3.connect(out.resolve().as_uri(), uri=True)
    dst.execute("PRAGMA auto_vacuum = INCREMENTAL")
    # Tables first, the data, then indexes and triggers (so the copy doesn't fire the totals' triggers).
    for kind in ("table", "data", "index", "trigger"):
        if kind == "data":
            dst.execute("ATTACH DATABASE ? AS src", (f"{source.resolve().as_uri()}?mode=ro",))
            for table in ("entries", "totals", "limits", "meta", "checked"):
                dst.execute(f"INSERT INTO main.{table} SELECT * FROM src.{table}")  # noqa: S608 (fixed table names)
            dst.commit()
            dst.execute("DETACH DATABASE src")
            continue
        for type_, sql in schema:
            if type_ == kind:
                dst.execute(sql)
    dst.execute(f"PRAGMA application_id = {application_id}")
    dst.execute(f"PRAGMA user_version = {user_version}")
    dst.commit()
    dst.execute("VACUUM")
    dst.close()


def columns(conn: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in conn.execute(f"PRAGMA table_info({table})")}


def seed_ledger(catalog: Path, rows: int) -> None:
    """Tops the copy's `file_failures` up to `rows` rows: the import's own (one per failed analysis, kept by the schema's
    triggers) and then thumbs rows, then borders rows, for the files by id that have none of that pass. Each row records
    the file as it is (mtime, size, rotation), `hidden` as the schema keeps it for those passes (the file missing), and
    the same error and first-seen time; the schema's own trigger stamps its serial, as the app's writes are."""
    conn = sqlite3.connect(catalog)
    ledger = columns(conn, "file_failures")
    if "hidden" not in ledger:
        print(f"{catalog}: file_failures has no hidden column (the ledger before the notifications migration): not seeded")
        conn.close()
        return
    have = conn.execute("SELECT count(*) FROM file_failures").fetchone()[0]
    files = conn.execute("SELECT count(*) FROM files").fetchone()[0]
    for pass_ in ("thumbs", "borders"):
        wanted = rows - have
        if wanted <= 0:
            break
        conn.execute(
            "INSERT INTO file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen, hidden)\n"
            "SELECT f.id, ?, f.mtime, f.size, f.rotation, ?, 1, ?, f.missing FROM files f\n"
            "WHERE NOT EXISTS (SELECT 1 FROM file_failures x WHERE x.file_id = f.id AND x.pass = ?)\n"
            "ORDER BY f.id LIMIT ?",
            (pass_, SEED_ERROR, SEED_FIRST_SEEN, pass_, wanted),
        )
        have = conn.execute("SELECT count(*) FROM file_failures").fetchone()[0]
    conn.commit()
    by_pass = conn.execute("SELECT pass, count(*), sum(hidden) FROM file_failures GROUP BY pass ORDER BY pass").fetchall()
    conn.execute("VACUUM")
    conn.close()
    shown = ", ".join(f"{name} {count} ({hidden} hidden)" for name, count, hidden in by_pass)
    print(f"{catalog}: {have} ledger rows over {files} files: {shown}")
    if have < rows:
        print(f"{catalog}: only {have} of the {rows} ledger rows asked for fit (two passes per file)", file=sys.stderr)


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    ap.add_argument("layout", help="the catalog layout's name, as the bench's LAYOUT constant has it")
    ap.add_argument("catalog_dir", type=Path, help="the folder with catalog.sqlite and thumbs.sqlite")
    ap.add_argument("out", type=Path, help="where the layouts' folders go")
    ap.add_argument(
        "--ledger-rows",
        type=int,
        help=f"seed file_failures to this many rows (default {LARGE_LEDGER_ROWS:,} for a -large layout, else 0)",
    )
    args = ap.parse_args()
    ledger_rows = args.ledger_rows
    if ledger_rows is None:
        ledger_rows = LARGE_LEDGER_ROWS if args.layout.endswith("-large") else 0

    target = args.out / args.layout
    if target.exists():
        print(f"{target} exists: a baseline may have been measured on it; pick another layout name", file=sys.stderr)
        return 1
    target.mkdir(parents=True)

    src = read_only(args.catalog_dir / "catalog.sqlite")
    src.execute("VACUUM INTO ?", (str(target / "catalog.sqlite"),))
    src.close()
    if ledger_rows > 0:
        seed_ledger(target / "catalog.sqlite", ledger_rows)
    slim_cache(args.catalog_dir / "thumbs.sqlite", target / "thumbs.sqlite")

    for name in ("catalog.sqlite", "thumbs.sqlite"):
        print(f"{target / name}: {(target / name).stat().st_size / 1e6:.1f} MB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
