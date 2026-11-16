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
import shutil
import sqlite3
import sys

GRID = 16


def orbits() -> list[list[int]]:
    """The 16x16 grid's cells (bit k: row k / 16, column k % 16) in orbits of the eight dihedral maps."""
    seen: set[int] = set()
    out = []
    last = GRID - 1
    for k in range(GRID * GRID):
        if k in seen:
            continue
        r, c = divmod(k, GRID)
        cells = {(r, c), (c, r), (last - r, c), (r, last - c), (last - r, last - c), (c, last - r), (last - c, r),
                 (last - c, last - r)}
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

    src = sqlite3.connect(f"file:{args.src}?mode=ro", uri=True)
    shutil.rmtree(args.dst + "-wal", ignore_errors=True)
    dst = sqlite3.connect(args.dst)
    src.backup(dst)
    src.close()
    dst.execute("PRAGMA foreign_keys = ON")

    file_cols = [r[1] for r in dst.execute("PRAGMA table_info(files)")]
    analysis_cols = [r[1] for r in dst.execute("PRAGMA table_info(analysis)")]
    originals = dst.execute(
        "SELECT a.file_id FROM analysis a CROSS JOIN files f WHERE f.id = a.file_id AND f.volume IS NOT NULL "
        "AND a.pdq_dihedral IS NOT NULL ORDER BY a.file_id"
    ).fetchall()
    have = len(originals)
    if args.total <= have:
        sys.exit(f"the source holds {have} pictures already")
    next_id = dst.execute("SELECT max(id) FROM files").fetchone()[0] + 1
    # File references past any NTFS file ID's 48-bit record and 16-bit sequence: no real file has one.
    ref_base = 1 << 62
    cells = orbits()
    rng = random.Random(args.seed)
    files_select = f"SELECT {', '.join(file_cols)} FROM files WHERE id = ?"
    analysis_select = f"SELECT {', '.join(analysis_cols)} FROM analysis WHERE file_id = ?"
    files_insert = f"INSERT INTO files({', '.join(file_cols)}) VALUES ({', '.join('?' * len(file_cols))})"
    analysis_insert = f"INSERT INTO analysis({', '.join(analysis_cols)}) VALUES ({', '.join('?' * len(analysis_cols))})"
    fi = {c: i for i, c in enumerate(file_cols)}
    ai = {c: i for i, c in enumerate(analysis_cols)}
    added = 0
    copy = 0
    dst.execute("BEGIN")
    while have + added < args.total:
        copy += 1
        m = mask(rng, cells) * 4
        for (orig,) in originals:
            if have + added >= args.total:
                break
            f = list(dst.execute(files_select, (orig,)).fetchone())
            a = list(dst.execute(analysis_select, (orig,)).fetchone())
            f[fi["id"]] = next_id
            f[fi["file_ref"]] = ref_base + next_id
            f[fi["pending"]] = 0
            a[ai["file_id"]] = next_id
            blob = a[ai["pdq_dihedral"]]
            a[ai["pdq_dihedral"]] = bytes(x ^ y for x, y in zip(blob, m))
            dst.execute(files_insert, f)
            dst.execute(analysis_insert, a)
            next_id += 1
            added += 1
    dst.execute("COMMIT")
    dst.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    total = dst.execute("SELECT count(*) FROM analysis WHERE pdq_dihedral IS NOT NULL").fetchone()[0]
    dst.close()
    print(f"{have} pictures, {added} added in {copy} copies: {total}")


if __name__ == "__main__":
    main()
