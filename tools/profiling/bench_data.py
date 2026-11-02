"""Prepare the catalog benches' data (crates/core/benches/ir/catalog.rs) from a catalog folder.

Writes OUT/LAYOUT/catalog.sqlite (VACUUM INTO: compact, consistent with the WAL) and OUT/LAYOUT/thumbs.sqlite (the
cache's schema, entries, totals and limits, without the JPEGs). An existing LAYOUT folder is never overwritten, so
the data a baseline was measured on stays. The sources are opened read-only.

    python tools/profiling/bench_data.py v1-orderkey D:\\PhotoGallery\\scratch_orderkey\\catalog D:\\PhotoGallery\\bench-data

wsl-bench.sh copies OUT/* to WSL's disk (/root/pg-bench) before running the benches.
"""

import argparse
import sqlite3
import sys
from pathlib import Path


def read_only(path: Path) -> sqlite3.Connection:
    return sqlite3.connect(f"{path.resolve().as_uri()}?mode=ro", uri=True)


def slim_cache(source: Path, out: Path) -> None:
    """The cache without its `data` rows: what the pending check reads."""
    src = read_only(source)
    schema = src.execute("SELECT type, sql FROM sqlite_schema WHERE sql IS NOT NULL").fetchall()
    user_version = src.execute("PRAGMA user_version").fetchone()[0]
    src.close()

    # As a URI, so the source can be attached read-only.
    dst = sqlite3.connect(out.resolve().as_uri(), uri=True)
    dst.execute("PRAGMA auto_vacuum = INCREMENTAL")
    # Tables first, the data, then indexes and triggers (so the copy doesn't fire the totals' triggers).
    for kind in ("table", "data", "index", "trigger"):
        if kind == "data":
            dst.execute("ATTACH DATABASE ? AS src", (f"{source.resolve().as_uri()}?mode=ro",))
            for table in ("entries", "totals", "limits"):
                dst.execute(f"INSERT INTO main.{table} SELECT * FROM src.{table}")
            dst.commit()
            dst.execute("DETACH DATABASE src")
            continue
        for type_, sql in schema:
            if type_ == kind:
                dst.execute(sql)
    dst.execute(f"PRAGMA user_version = {user_version}")
    dst.commit()
    dst.execute("VACUUM")
    dst.close()


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    ap.add_argument("layout", help="the catalog layout's name, as the bench's LAYOUT constant has it")
    ap.add_argument("catalog_dir", type=Path, help="the folder with catalog.sqlite and thumbs.sqlite")
    ap.add_argument("out", type=Path, help="where the layouts' folders go")
    args = ap.parse_args()

    target = args.out / args.layout
    if target.exists():
        print(f"{target} exists: a baseline may have been measured on it; pick another layout name", file=sys.stderr)
        return 1
    target.mkdir(parents=True)

    src = read_only(args.catalog_dir / "catalog.sqlite")
    src.execute("VACUUM INTO ?", (str(target / "catalog.sqlite"),))
    src.close()
    slim_cache(args.catalog_dir / "thumbs.sqlite", target / "thumbs.sqlite")

    for name in ("catalog.sqlite", "thumbs.sqlite"):
        print(f"{target / name}: {(target / name).stat().st_size / 1e6:.1f} MB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
