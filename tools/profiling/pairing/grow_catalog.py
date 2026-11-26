#!/usr/bin/env python3
"""A larger pairing workload from a real catalog: copies of its pictures, each copy's hashes moved by a mask.

    python tools/profiling/pairing/grow_catalog.py SRC DST TOTAL [--seed 1]

Copies the catalog SRC to DST (SQLite's backup), then adds copies of its pictures (the analyses with stored hashes, of
files with an identity) until DST holds TOTAL of them: each copy is a new `files` row (a new id, the same folder and
name, a file reference no real file has) and a new `analysis` row whose stored hashes are the original's XOR a mask of
its copy.

The mask takes one random bit for each orbit of the 16x16 grid's cells under its eight dihedral maps, so it is the same
in every orientation: a picture's stored orientations are permutations of its grid's bits, and each one XOR the mask
is the orientation of the masked hash. So a copy's pictures are at the same distances among themselves, in every
orientation, as the originals (each copy holds the library's pairs again, exactly), and each chunk's values are the
originals' moved by one constant (the same bucket sizes). Between copies, the distances are those of unrelated hashes,
so a copy adds no pair with another one (a picture and its own copy are popcount(mask a ^ mask b) apart, about 128).

The source is read only. The pass's probe (`pairs_probe`, which owes every picture again first) then runs on DST.
"""

import argparse
import random
import sqlite3
import sys
from itertools import count
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from pgtools import register_catalog_functions

GRID = 16
# The copy's ids: each original's file id and its copy's.
MAPPING = "CREATE TEMP TABLE copied(old INTEGER PRIMARY KEY, new INTEGER NOT NULL)"
# One copy, each statement whole: the originals' rows into temporary tables of the same columns (`SELECT *`), given
# there the copy's ids, file references and moved hashes, then inserted in id order, the files first (the analyses
# reference them). The triggers run on the inserts as on any file and analysis added. A copy's file reference is 2^62
# plus its id: past any NTFS file ID's 48-bit record and 16-bit sequence, so no real file has one.
COPY = (
    "CREATE TEMP TABLE copy_files AS SELECT * FROM files WHERE id IN (SELECT old FROM temp.copied)",
    "UPDATE temp.copy_files SET id = (SELECT new FROM temp.copied WHERE old = id), pending = 0",
    "UPDATE temp.copy_files SET file_ref = 4611686018427387904 + id",
    "INSERT INTO files SELECT * FROM temp.copy_files ORDER BY id",
    "CREATE TEMP TABLE copy_analysis AS SELECT * FROM analysis WHERE file_id IN (SELECT old FROM temp.copied)",
    (
        "UPDATE temp.copy_analysis SET file_id = (SELECT new FROM temp.copied WHERE old = file_id), "
        "pdq_dihedral = moved(pdq_dihedral)"
    ),
    "INSERT INTO analysis SELECT * FROM temp.copy_analysis ORDER BY file_id",
    "DROP TABLE temp.copy_files",
    "DROP TABLE temp.copy_analysis",
)


def orbits() -> list[list[int]]:
    """The 16x16 grid's cells (bit k: row k / 16, column k % 16) in orbits of the eight dihedral maps."""
    seen: set[int] = set()
    out = []
    last = GRID - 1
    for k in range(GRID * GRID):
        if k in seen:
            continue
        r, c = divmod(k, GRID)
        cells = {
            (r, c),
            (c, r),
            (last - r, c),
            (r, last - c),
            (last - r, last - c),
            (c, last - r),
            (last - c, r),
            (last - c, last - r),
        }
        orbit = sorted(a * GRID + b for a, b in cells)
        seen.update(orbit)
        out.append(orbit)
    return out


def mask(rng: random.Random, cells: list[list[int]]) -> bytes:
    """A 32-byte hash mask with one random bit per orbit (bit k is byte k / 8, most significant first)."""
    out = bytearray(32)
    for orbit in cells:
        if rng.getrandbits(1):
            for k in orbit:
                out[k // 8] |= 1 << (7 - k % 8)
    return bytes(out)


def main() -> None:
    ap = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    ap.add_argument("src")
    ap.add_argument("dst")
    ap.add_argument("total", type=int)
    ap.add_argument("--seed", type=int, default=1)
    args = ap.parse_args()

    for suffix in ("", "-wal", "-shm"):
        Path(args.dst + suffix).unlink(missing_ok=True)
    src = sqlite3.connect(f"file:{args.src}?mode=ro", uri=True)
    dst = sqlite3.connect(args.dst, isolation_level=None)
    src.backup(dst)
    src.close()
    dst.execute("PRAGMA foreign_keys = ON")
    # The copies are file rows: the name index's function is the catalog's.
    register_catalog_functions(dst)

    originals = [
        r[0]
        for r in dst.execute(
            "SELECT a.file_id FROM analysis a CROSS JOIN files f WHERE f.id = a.file_id AND f.volume IS NOT NULL "
            "AND a.pdq_dihedral IS NOT NULL ORDER BY a.file_id"
        )
    ]
    have = len(originals)
    if args.total <= have:
        sys.exit(f"the source holds {have} pictures already")
    next_id = dst.execute("SELECT max(id) FROM files").fetchone()[0] + 1
    cells = orbits()
    rng = random.Random(args.seed)
    # The copy's mask, as the SQL function the copy's hashes are moved by.
    current = bytearray()

    def moved(blob: bytes) -> bytes:
        return bytes(x ^ y for x, y in zip(blob, current, strict=True))

    # Not deterministic: its mask (`current`) changes between the copies, so the same blob gives different results.
    dst.create_function("moved", 1, moved, deterministic=False)
    added = 0
    copy = 0
    dst.execute("BEGIN")
    dst.execute(MAPPING)
    while have + added < args.total:
        copy += 1
        current[:] = mask(rng, cells) * 4
        part = originals[: args.total - have - added]
        dst.execute("DELETE FROM temp.copied")
        dst.executemany("INSERT INTO temp.copied(old, new) VALUES (?1, ?2)", zip(part, count(next_id)))
        for statement in COPY:
            dst.execute(statement)
        next_id += len(part)
        added += len(part)
    dst.execute("COMMIT")
    dst.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    total = dst.execute("SELECT count(*) FROM analysis WHERE pdq_dihedral IS NOT NULL").fetchone()[0]
    dst.close()
    print(f"{have} pictures, {added} added in {copy} copies: {total}")


if __name__ == "__main__":
    main()
