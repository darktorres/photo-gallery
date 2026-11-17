# Pairing pass plan: rebuilt for speed and memory

> Note 2026-11-16 (commit aeb7d2c): the pairing pass does not run while duplicates detection is off (Library panel, or
> `gallery import --no-dups`). Its owed pairs wait in `pairs_owed`. Files the import left without hashes owe them in
> `hashes_owed`; the Hashes pass makes those hashes from the same decode, before the pairing pass, and the "not a duplicate"
> view leaves owed files out.

## Status (2026-11-16)

Blocks A - D are done: `d3d307a` (A and B), `b7d13a6` (C), `98c6973` (D). The second round,
[PAIRING_SECOND_ROUND_PLAN.md](PAIRING_SECOND_ROUND_PLAN.md) (steps 0 - 10 done, with the items its status lists
as not built), rebuilt the join's sort, scan
kernel, few-owed path and threads, fixed a pair loss, and re-tuned what that made stale. It changed three things this
plan set: **the sorter threads and G3's rule are gone** (one thread team sorts and scans; a thread takes a sort's task
while the sort is behind, so no count is tuned); **the 330 MB cap is removed** (peak commit is recorded with each
change, not held to a target); and **G5 was run** on a grown 500,000-picture catalog. The measurements and each gate's
result are in [PROFILING.md](../reference/PROFILING.md), "The pairing pass" and after. Against the targets below:

| Target | After block D | After the second round |
|---|---|---|
| Same pairs | Met: the oracle's 5,277 exactly, in one slice, four slices, and partial passes on both paths | Met, on the same seven paths |
| 12 workers, 5x or better | Met: 6.56 -> 1.27 s (5.2x) | Met: 8.60 -> 0.92 s (9.3x), both timed with turbo off |
| 1 worker, 3.5x or better | **Not met**: 10.63 -> 5.15 s (2.1x); the scan is 3.9 s of it | **Not met**: 14.80 -> 4.69 s (3.2x), both timed with turbo off (the first column's baseline was timed with turbo, its result without). Tracked as P-13 in [PROFILING_PLAN.md](PROFILING_PLAN.md) |
| A pass for a few files, about 0.35 s | Met: 0.24 s for one owed picture | 0.13 - 0.14 s for 300 owed at 12 workers |
| Peak commit at most 330 MB | Met: 254 MiB all owed, 75 MiB one owed | The cap is removed; 172 - 174 MiB all owed, 63 MiB 300 owed |
| `prepare` at every start, about 30 ms | 42 ms for the probe's whole process with nothing owed (111 - 126 ms when the extension check runs) | Not re-measured |

Gates: G1, G2, G2b, G5, G6 and the cache size are measured and set; G3 is replaced by the team. **G4 (the kernel's
loop nest) was not run**: at one 200,000 slice the query buckets are the longer side, which the current kernel
already vectorizes; the second round's step 3 built a new kernel instead. Not tested: a failed write while the
overlapped join is paused (the unpaused case is). Block E: the `pairing.join` bench is in `crates/bench`
(`ir::pairing::join`, both paths); `pairing.write` was not made, as the import flush bench already counts the
catalog's write path and the pass's write is timed by `pairs_probe`.

## Context

The pairing pass (post-import duplicate detection: `pg_import::pairs`, the join in `crates/dedup/src/join.rs`,
`PairsWriter`, migration TO_12) is correct (5,277 pairs, exactly the snapshot's) and moved the import's duplicate work out of
the import (the user sees no stalls). It is slow where nobody had looked, and heavier in memory than assumed. Everything here
was measured on the user's library (189,520 files, 187,804 hashed images) on scratch copies, release build, i7-8700
(6 cores / 12 threads); the live catalog (`D:\PhotoGallery\catalog`, schema v12) is never written. Evidence is in
`<scratchpad>\e2e\` (callgrind drilldowns, per-slice dumps, timing and sweep files), `proto\` (a prototype that replays the
join on the real hashes), `g1_writes.py`, `g2_migration.py`, `load_q.py`, `sweep2.ps1`, `dhat_run.ps1`.

The brief is **the most proper design, churn is free**: each item below is the right fix for its cost, not the smallest one.
Nothing is left conditional on "if it still shows": the few places where a measurement still decides something decide a
tuning value (slice size, thread counts, cache size, a threshold), listed as gates in Part 4.

### Where the time and memory go (measured)

Pass at 12 workers: **7.4 s median** (11.6 - 12.1 s on 1 worker; flat past 6 workers = the physical cores). Only 0.9 s is the
parallel scan; about 6.4 s is serial or single-purpose work:

| Part | Wall (12 workers) | Cause |
|---|---|---|
| Join preparation | 1.5 - 1.7 s | `is_symmetric` recomputed for every picture on every slice (4.43 G Ir per slice, 17.7 G): a bit-by-bit 16x16 transpose, 486 ns each; a word-level one is 29 ns (16 - 18x) and equals the stored orientation `v+4` on all 751,216 hashes (0 mismatches) |
| Sorts | 2.1 - 2.5 s | 16 chunk sorts x 4 slices x 2 sides, a 65,536-bucket counting sort of 40-byte entries (561 ms per slice on one thread). The stored side is identical every slice and re-sorted 4 times: 210 of the 313 ms per slice (compact radix). 4.7 GB allocated over the pass, 64 fresh 30 MB buffers in `join::sort` alone (page faults) |
| Writes | 1.1 - 1.4 s | 187,804 `UPDATE`s of the `analysis` row (about 430 bytes of payload on a hashed row): 98.7 MB of WAL and 22 - 24 checkpoints (about 50 ms each); `commit()` alone 0.75 - 1.1 s although `synchronous = NORMAL` |
| Load | 0.34 - 0.56 s | rusqlite decoding (0.93 G Ir), three scans of `analysis JOIN files`, a temp B-tree for `ORDER BY`, a second 50 MB copy of the hashes |
| Scan stages | 0.9 s (5.2 s on 1 worker) | 54.6 G Ir: 1.78 G vector iterations, 7.1 G lane-slots against 2.9 G useful candidates: the kernel ignores the ordering rule structurally (a stored picture that cannot pair still costs its lanes) and the chunk grouping is skewed |
| `Catalog::prepare`, every start | about 130 ms (0.41 G Ir) | `mark_unheld_missing` scans 190 k rows (64 ms); the expected schema is built in memory twice (524 prepares) |
| Peak memory | **415 MB commit** all-owed, **366 MB for one owed file** (62 MB with nothing owed) | dhat: Rust heap 247.7 MB (201.9 MB for one owed file): `Vec<PairingPicture>` 79.7 MB (unsized, 17 growth steps allocate 159 MB), the `Vec<Picture>` copy 48.3 MB, join base entries 30 + 21 MB, one stage 66 MB. The other 167 MB is SQLite's page cache (256 MB `cache_size`, filled by the load), the binary and stacks |
| A pass for one file | 1.3 - 1.6 s (0.18 s with nothing owed) | about 1.3 s of fixed cost: load 0.3 + symmetry 0.43 + stored sorts 0.35 + startup |

Run-to-run noise is real (the machine is shared; 24 - 82% CPU before runs): every figure is a median, and one 6-worker run
had a single 18 s stage from a stalled machine. The writes do not grow with workers beyond noise (1.09 - 1.26 s at 1 worker,
1.14 - 1.39 s at 12).

### Targets

Same pairs (oracle: `e2e\after_pairs.txt`, 5,277), no new `unsafe` block: the join's new kernels are SIMD variants
under the existing dispatch and safety contract, and the free-space query (Part 1) extends the existing
`GetDiskFreeSpaceExW`/`statvfs` call in `pg_platform::volume`. **At 12 workers 5x or
better (about 1.3 - 1.5 s)**: preparation 0.1 + the sort/scan pipeline about 0.5 (sorts 0.45 on two sorter threads against a
scan of 0.35) + load 0.2 + writes 0.45. **On one worker 3.5x or better (about 2.6 - 3.2 s).** **A pass for a few files about
0.35 s** (load 0.2 + prepare 0.1 + a linear bit-test pass, from 1.3 s). **Peak commit at most 330 MB at every setting
(today 415 MB)**, lower with a smaller slice (the cap was removed in the second round: peak commit is recorded, not a
target). The `prepare` cost at every start falls from 130 to about 30 ms.

## Part 1: the data (migration TO_13, on top of draft TO_12)

The live catalog is already at schema v12 (the draft with `dups_state`), so TO_12 is not redefined: TO_13 migrates the v12
layout forward. `prepare` already copies a catalog aside before a migration (`back_up`, `rebuild.rs:38`).

**Free space.** The disk needs room for three things in turn: the copy (the file's size), the migration's WAL (it rewrites
every `analysis` page, at most the file's size), and `VACUUM`'s WAL (the new file's size). Each is checkpointed with
`TRUNCATE` before the next: `back_up` already does this first; it is added after the migration commits and after the
`VACUUM`. So the most needed at once is about twice the file. `pg_platform::volume::total_bytes` becomes `space(path) ->
Option<Space { total, free }>`, the same `GetDiskFreeSpaceExW` call (its first out-pointer, null today) and on Linux
`f_bavail`, with no new `unsafe` block (plus the non-Windows, non-Linux stub at `volume.rs:45`); its one caller,
`thumbcache/src/config.rs:32` (the cache's cap), takes `total`. The check runs before `back_up`:

- not room for the copy and the migration: `prepare` fails with an error naming the space needed and the folder, as a
  failed copy does today, and the catalog is not touched;
- room for those but not for `VACUUM`: the migration runs and `vacuum_owed` is set (item 3);
- `None` (unknown): it proceeds, and a write that fails fails as today.

1. **Pairs owed live in a narrow queue table**, not a column. `pairs_owed(file_id INTEGER PRIMARY KEY REFERENCES
   analysis(file_id) ON DELETE CASCADE)`; no column, CHECK or partial index on `analysis`. **The key is the analysis, not the
   file**: `pair.rs:424` deletes an analysis when a path is taken over by another file, and a queue row outliving its analysis
   would never clear (`has_owed_pairs` would stay true and `NOT_DUPLICATES` empty forever).
   - `store_analysis`: `INSERT OR IGNORE INTO pairs_owed` after the analysis row when the picture owes pairs (existing rule).
     There is no explicit `DELETE` in `store_analysis` or `store_failure`: both write the analysis with `INSERT OR
     REPLACE`, which always deletes the existing row and with it the queue row (next bullet), and with no existing row
     there is no queue row (the foreign key). One mechanism, pinned by a test; `delete_pairs` stays.
   - `PairsWriter::done`: `DELETE FROM pairs_owed WHERE file_id = ?1 AND (SELECT pdq_dihedral FROM analysis WHERE file_id = ?1)
     IS ?2`: the same guard, now on the whole stored hashes (stronger than the 32-byte `pdq`); the `pair` guards on both
     endpoints change the same way.
   - `owe_pairs_again`: `INSERT OR IGNORE INTO pairs_owed SELECT file_id FROM analysis WHERE pdq_dihedral IS NOT NULL AND
     pdq_quality >= ?`, then the existing `DELETE FROM pdq_pairs` and `next_seq!()`.
   - `NOT_DUPLICATES` (`crates/query/src/lib.rs`): its `NOT EXISTS` probes `pairs_owed JOIN files` (present, not pending).
   - `pairing_work`: owed = `LEFT JOIN pairs_owed`; `owed_pairs()` counts through it; `has_owed_pairs()` (`EXISTS`) is the
     pass's "nothing to do" probe.
   - **Invariant: a queue row implies an eligible analysis** (orientation hashes and `pdq_quality >= MIN_QUALITY`). The pass
     loads only eligible pictures, so a queue row on any other analysis would never clear (`has_owed_pairs` true and
     `NOT_DUPLICATES` empty forever). Every writer keeps it: `store_analysis` (its existing rule), `store_failure` (through
     the cascade), `owe_pairs_again` (its `WHERE`), and the data step (item 3) both when it carries and when it nulls a blob.
   - **`INSERT OR REPLACE INTO analysis` deletes the queue row through the cascade** (`foreign_keys = ON`, `schema.rs:447`;
     SQLite runs foreign-key actions for rows a REPLACE deletes). `store_analysis` (`writers.rs:256`) inserts the queue row
     after the analysis row, so it survives; the failure write (`writers.rs:264`) and the ledger tests' REPLACEs
     (`ledger.rs:544,787`) store no hashes, so losing it there is correct. A test pins this.
   - Migration steps: `Step::Sql` `CREATE TABLE pairs_owed`; then the `Step::Rust` data step (item 3), **which carries the
     owed rows**: a rebuild runs only the `Rust` steps (`rebuild.rs:228`), so a carry written as SQL would mark every
     picture done on a rebuild from a v12 file and its pairs would never be made; then `Step::Sql` `DROP INDEX
     analysis_dups_due`, `ALTER TABLE analysis DROP COLUMN dups_state`, `ALTER TABLE analysis DROP COLUMN pdq`. The
     `VACUUM` runs after the migration commits (item 3). `rebuild.rs` `CARRIED` gains `pairs_owed` after `analysis` (a
     table the old file lacks is skipped, `rebuild.rs:199`). The baseline schema is the v13 layout. Every use of
     `dups_state` follows (writers, schema tests, `fixture::state`, `browse/read/tests.rs`, `query`,
     `library/tests/pairs.rs`).
   - **Measured (G1, 3 runs each, the app's pragmas, the pass's real guarded statements):** column `UPDATE` today 1.75 - 1.87 s
     (statements 0.62 - 0.67, commits 1.12 - 1.18, 98.7 MB WAL, 22 of 94 commits spike over 20 ms); with `wal_autocheckpoint =
     0` the 0.75 s only moves to a final checkpoint; **queue `DELETE` 0.43 - 0.55 s (3.4 MB WAL, no spikes): 4.2x**, and
     batch size 2,000 or 10,000 makes no difference (2,000 kept: each write lock is a few ms). `owe_pairs_again` 1.25 s ->
     0.11 s (11x).
2. **The stored hashes: 4 orientations, no `pdq` column.** `pdq_dihedral` is 256 bytes x 187,808 rows = 48.1 MB (21% of the
   file); orientation `v+4` is the exact transpose of `v` (the dihedral transform negates and transposes the DCT
   coefficients and leaves the median unchanged), and `pdq` equals the blob's first 32 bytes on 187,808 of 187,808 rows and is
   read only by `pairing_work` and the pairs' guards. The blob keeps orientations 0..3 (128 bytes), `analysis.pdq` is
   dropped, `pdq_quality` stays. A hashed row's payload goes from about 430 to about 270 bytes (measured on the scratch
   copy: 431 bytes on average, less 128 of the blob and 32 of `pdq`): every `analysis` write carries less, the
   load reads less, and the join loses its symmetry check and fallbacks. The import hashes 4 variants, not 8 (2 - 3% of
   `analyze`, about 12 - 18 s of the 612 s import): `Hasher::hash_rgb_dihedral` returns 4 (its test still compares all 8
   against Meta's dihedral hasher and asserts the identity on real inputs).

   **The model keeps the hashes in one place.** Today the hash is held twice: `PixelHashes.pdq: Pdq { hash, quality }`
   (`model/src/analysis.rs:101`), and `Analyzed.pdq_dihedral: Option<[Hash; 8]>` (line 55), whose element 0 is the same
   hash. With `analysis.pdq` gone, `Pdq.hash` would be stored nowhere. So `Pdq` becomes `{ orientations: [Hash; 4], quality
   }` in `PixelHashes`, and `Analyzed.pdq_dihedral` goes: only an image has the hashes, as only an image has `PixelHashes`,
   so the `Option` is the `image()` that already exists. `hash_rgb` (a single hash, used only by `pdq.rs`'s own tests,
   lines 363 and 395) returns `(Hash, u8)`. Following it: `analyze.rs`, `writers.rs` (`store_analysis` binds
   `pixels.pdq.orientations`), `work.rs`, `render/maker.rs:207`, `test_support.rs:86-94`, `writers.rs:1123`,
   `analysis/examples/stage_times.rs:54`, the import-flush bench's `Analyzed` (`bench/benches/ir/catalog.rs:182-201`)
   and the fixtures. The word-level `Hash::transpose` lives in
   `pg_model::pdq`, shared by the join, the migration and the analysis. `IMPORT_VERSION` and `PAIRING_VERSION` stay (the
   stored values are a prefix of the old ones; the pairs are identical).
3. **The data step** (`convert::pairs_owed_and_hashes`, a `Step::Rust` like `retire_kinds`). It recognises the old data by
   its shape, never by the version, and is idempotent, so it serves both paths (`convert.rs:1-7`). In this order: the cut,
   then the carry.
   - **The cut, on `main.analysis` and keyed on blob length** (on a rebuild the column copy has already brought the 256-byte
     blobs into `main`): each 256-byte blob is verified (`blob[128..256]` equals the word-level transposes of
     `blob[0..128]`). A row that passes is cut to 128 bytes. A row that fails, or whose blob is neither 128 nor 256 bytes,
     gets a NULL blob **and loses its queue row** (the invariant, item 1), and becomes a "legacy" analysis, which the pass
     already reports ("run `gallery import --again`"). **The identity is an invariant of the data; no fallback exists.**
     A 128-byte blob is left as it is (already converted).
   - **The carry, after the cut:** when `{source}.analysis` has `dups_state` (`Source::Main` while migrating,
     `Source::Carried` for a rebuild), `INSERT OR IGNORE INTO main.pairs_owed SELECT file_id FROM {source}.analysis WHERE
     dups_state = 'due'`, restricted to the rows of `main.analysis` that are eligible after the cut. An old file without
     the column (before v12) owes nothing, as TO_12 says.
   - **`VACUUM` after the commit, outside the migration's transaction** (`bring_to` runs the steps in one write transaction,
     `schema.rs:495-501`, and `VACUUM` cannot run inside one). Dropping columns leaves the pages in use, so the file shrinks
     only after it. **A failed `VACUUM` (busy, disk full) does not fail the migration**, since the data is already
     converted: it is logged, and a mark in `meta` (`vacuum_owed`) retries it at the next start. The free-space check
     comes before it. The app's `temp_store = MEMORY` builds VACUUM's temporary copy in RAM (about the file's size,
     176 MB), so the measurement is repeated under the app's pragmas (G2b, Part 4). A rebuild needs no `VACUUM` (it
     writes a new file). **`VACUUM` keeps every id the code relies on.** It may renumber the rowids of tables without an
     explicit `INTEGER PRIMARY KEY`. Every table whose id is stored or returned has one (`files`, `folders`, `scans`,
     `roots`, `notices`, `file_failures`, `folder_failures`, `scan_leases`, `analysis`, `ratings`, `file_ratings`,
     `border_marks`; `roots.rs:126` reads `scans.id`). The others are keyed by their own columns (`file_writes`
     `(file_id, kind)`, `views`/`session`/`meta` by text, `border_sides` and `pdq_pairs` `WITHOUT ROWID`), and nothing
     reads their rowid (checked by grep: the only `last_insert_rowid` is `roots.rs:126`). `user_version`,
     `application_id` and the WAL mode survive `VACUUM`; the test checks them after it.
   - Its progress goes through the existing migration notice.
   **Measured (G2) on two scratch copies:** 0 identity failures on 187,808 rows each time; steps 3.3 s (209 MB copy) to 6.8 s
   (227.6 MB copy), `VACUUM` 1.84 - 2.50 s, file 227.6 -> 174.1 MB and 209.4 -> 176.3 MB (-16% to -23.5%), integrity and
   foreign-key checks clean; the pass's load query 436 -> 244 - 256 ms (Part 3 item 1). A one-off the user sees at the first start.

## Part 2: the join engine (`crates/dedup`)

1. **No symmetry check; word-level transposes; compact entries.** With Part 1 the invariant `v+4 = transpose(v)` is given,
   so `is_symmetric`, the per-picture flags, the 8-entry fallbacks and their tests go. `join` takes `&[[Hash; 4]]` and
   `&[bool]` (structure of arrays, what `PairingWork` returns) instead of the 50 MB `Vec<Picture>` copy; `Picture` goes. A
   pass-level `Joiner::new(hashes, due)` builds the stored entries once (4 per picture); per slice only the query side is
   built, orientations 4..7 by a **word-level 16x16 transpose** (4 butterfly rounds). The old bit-level `transpose` stays in
   the tests as the reference.
   - **One store of permuted hashes, no full-width base entries.** It holds the full 256 bits, after item 4's permutation,
     of orientations 0..3 of every picture (751 k x 32 B = 24 MB) and of orientations 4..7 of the owed pictures (at most
     751 k x 32 B = 24 MB). A query's orientations 0..3 are that picture's stored entries, so they are not kept twice. Each
     sort reads its keys and head words from this store, in store order (picture order, so the radix sort's stability
     holds).
   - **Sorted stage copies are three separate arrays, 20 B per entry:** `head0` and `head1` (the permuted hash's first two
     words) and `code` (`picture << 4 | due << 3 | orientation`). `scan_avx2` loads 4 consecutive `head0`/`head1` values
     (`join.rs:349`), so side-by-side 24-byte records would need gathers. The final radix pass writes the three arrays
     directly. The 309 ms in item 2 was measured on side-by-side records, so the three-array variant is timed again in block B.
   - **`consider` reads the full permuted words from the store**, at the index the code gives (stored: `picture * 4 + o`;
     owed orientations 4..7: `rank[picture] * 4 + (o - 4)`, with `rank` a `u32` per picture, 0.75 MB). It recomputes
     nothing: `matched_earlier` needs the *permuted* chunks, so rebuilding the hash from the loaded `[Hash; 4]` would cost
     a transpose and a permutation per surviving candidate, a number nobody has measured.
   - The loaded, unpermuted blobs (24 MB) stay for the writers' guards.
   - `MAX_PICTURES = u32::MAX >> 4`.

   Measured: transposes of a slice 380 ms -> 22 ms; preparation 1.5 s -> about 0.1 s; 17.7 G Ir -> under 0.3 G.
2. **Radix sort, buffers reused.** Replace the 65,536-bucket counting sort by a **stable two-pass LSD radix sort** (8 + 8 bits,
   `u32` offsets) over the compact entries, in a `Scratch` reused across stages and slices (no per-stage 30 MB allocation).
   Stability on input in picture order puts **every bucket in picture-index order**, which item 3 needs. **Measured on the
   real hashes, one slice, one thread, 16 chunks, both sides:** counting 561 ms; radix on 40-byte entries 448 ms; **radix on
   24-byte entries 309 ms (1.8x)**. The sort is memory-bound: spreading the 16 chunks over 1 / 2 / 3 / 4 / 6 / 8 / 12 threads
   gave 318 / 233 / 242 / 235 / 221 / 263 / 269 ms (1.4x at 2 threads, then flat). So the leverage is how often and where the
   sorting runs (items 5 and 6), not the thread count of one sort.
3. **The ordering rule applied structurally, exactly.** A due stored picture `i` can pair only with queries of index `> i`:
   a suffix of the query bucket, starting at the first query entry whose picture index is `> i`. Both sides are
   index-ordered, so for each (query bucket, probe) pair one pointer finds it. The pointer never moves back: it stays put
   between stored entries of the same picture `i`, since several of its orientations can share a stored bucket. The vector
   loop starts at the pointer; a not-due stored picture takes the whole bucket. `consider` keeps its check
   (defence in depth, identical output). **Measured (prototype replaying the loops on the real hashes; its count of 1,780,480,860
   vector iterations equals callgrind's `y += 4` line):** per 50,000-picture slice the iterations fall to 15%, 42%, 68%, 91% of
   today's; 52% over the pass.
4. **A diagonal chunk grouping.** The pigeonhole argument holds for any partition of the 256 bits into 16 groups of 16 and
   Hamming distance is invariant under a bit permutation, so the join permutes every entry's bits before chunking. Today's
   groups (16 contiguous bits = one row of the 16x16 coefficient grid) are skewed: on 30,000 real pictures queries x stored
   candidates are **1.279x** a uniform model's; `group = (row + col) % 16` gives **1.040x** (a random permutation 1.05x, a
   strided one 1.20x, a 4x4 interleave 1.16x), flatter buckets (worst chunk 1.10x against 1.42x), same pairs. Applied once
   per entry as it is built (rotate each 16-bit row by its row index, then the word-level transpose), stored and query alike
   and before `matched_earlier`; `chunk()` stays a shift and mask; `PAIRING_VERSION` stays. Measured with items 3 and 5:
   1.50 G iterations today -> **0.785 G** (-56% against today's 1.78 G), 476 M (stored x probe) setups instead of 791 M.
5. **One 200,000-picture slice (`SLICE`, was 50,000).** Each slice re-sorts the whole stored side and re-runs every
   (stored x probe) setup. Prototype counts for the whole pass (diagonal grouping, ordering skip), setups / vector iterations /
   scan Ir (fit 25 Ir per iteration + 12 Ir per setup, which reproduces the measured 54.6 G): 25,000: 766 M / 931 M / 32.5 G;
   **50,000: 476 M / 785 M / 25.3 G; 100,000: 289 M / 708 M / 21.2 G; 200,000 (one slice for this library): 191 M / 669 M /
   19.0 G**; and the stored side is sorted once instead of 4 times (-0.63 s single-threaded). Costs:
   - A sorted query copy is 8 x 200,000 x 20 B = 32 MB (8 MB at 50,000).
   - Results are written only when a slice's join ends, so a stop discards at most one join (about 0.5 s at 12 workers,
     about 2.5 s on one, by the Targets) and the owed rows stay owed.
   - **The slice size is a parameter of the pass** (`pairs::Settings { slice }`, default `SLICE`), not only a `const`:
     this library is one slice at 200,000, so without it the multi-slice path and the write overlap (Part 3 item 2)
     would never run in the tests, on the real data, or in G5. The tests force small slices on small corpora, and
     `pairs_probe` takes `PG_PROBE_SLICE`.
   - **A paused join keeps its memory** (about 200 MB, item 6's budget) until it resumes or stops. That is accepted: a
     pause is short, and dropping and rebuilding the join on resume would cost more than it saves.

   Libraries above 200,000 pictures run several slices (item 6 and Part 3 item 2 keep them fast). The final value comes
   from G5 under the memory cap (the cap since removed; the second round kept 200,000, PROFILING.md "The settings
   again").
6. **The sort/scan pipeline.** The scan (about 0.35 s at 12 workers) is shorter than the sorts (0.45 s), so the join is
   sort-bound and the sorting runs **ahead of the scan on 1 - 2 sorter threads** (scoped; measured 1.4x for two), filling a
   ring of stage buffer sets (stored 15 MB + queries 30 MB = 45 MB per set at one slice) while the workers scan the current
   stage; the permuted store is built over all workers. Sorters and workers share the thread budget. A sorter runs the same
   `start` closure first, so it takes the pass's `BelowNormal` priority (`runner.rs:596`); otherwise sorting at normal
   priority would compete with the GUI. It obeys the same `control` closure as the workers (never a flag of its own), so a
   stop or pause reaches it. (Superseded by the second round's step 5: one thread team sorts and scans, a thread taking
   a sort's task while the sort is behind, in one set of buffers; no sorter count is tuned.) Ring depth and sorter count
   were chosen by G3 under the memory cap (each extra set is 45 MB; each sorter's scratch is one buffer of the larger side,
   30 MB). Per-stage thread spawn is kept: 0.48 ms per stage, 30.8 ms over 64 stages (3.5% of the scan), not worth
   persistent workers.

   **Memory budget at one slice, all 187,804 owed** (today's Rust heap is 247.7 MB, its peak commit 415 MB):

   | Item | Size |
   |---|---|
   | Loaded work (ids, owed flags, unpermuted 4-orientation blobs) | 26 MB |
   | Permuted store (item 1) | 48 MB |
   | Ring, 2 sets x 45 MB | 90 MB |
   | Sorter scratch, 1 - 2 x 30 MB | 30 - 60 MB |
   | Offsets, `rank`, pairs | about 3 MB |
   | **Rust heap** | **about 195 - 225 MB** |

   Add the binary, the stacks and a bounded page cache (Part 3 item 1): **peak commit about 260 - 290 MB**, against the
   330 MB cap (removed in the second round; measured after it: 172 - 174 MiB). The plan as first written (40-byte base arrays, side-by-side 24-byte records) came to a 260 - 296 MB heap
   and 330 - 363 MB of commit, over the cap; that is why item 1 has one permuted store. A ring of 3 sets or a 400,000
   slice is admitted only if G3 and G5 show it inside the cap.

   **How the budget grows with the library.** The cap was this library's. The stored side grows with every picture
   whatever the slice: about 128 B loaded + 128 B permuted + 2 x 80 B in the ring (4 entries x 20 B) + up to 80 B of
   scratch, so about 0.45 - 0.5 KB per picture. The query side is bounded by the slice. A 1,000,000-picture library
   therefore needs about 0.5 GB at the default settings (measured after the second round on a grown 1,000,000-picture
   catalog: 462 MiB); today's design needs more (it grows the same way, plus the
   copies item 1 removes). That is stated in `BOUNDED_MEMORY_PLAN.md` (Docs), not hidden; bounding the stored side
   would take a chunk-partitioned stored sort, its own plan if a library that size appears.

   **A few-owed pass** (item 7) holds the loaded work, the permuted store and the tiny query side: about 55 MB of heap,
   an expected 100 - 150 MB of commit (today 366 MB), measured in G6.
7. **A few-owed path in the same engine** (the common case after an import round, 1 - 3,000 owed pictures: 1.3 - 1.5 s
   today). When the owed pictures' query entries are few (threshold from G6, about 20,000 entries), the stored side is **not
   sorted at all**: for each of the 16 chunks the (tiny) query side is sorted into buckets with a bitmap of the 65,536
   chunk values (8 KB per chunk, 128 KB for all 16, in L2). Then **one linear pass over the permuted store** reads each
   stored entry once. For every chunk it tests the entry's chunk value and its 16 one-bit neighbours (17 bit tests)
   against that chunk's bitmap, and calls the same `consider` on a hit, in chunk order. One pass rather than 16 passes
   over 24 MB. The pass is split by stored index range over the workers (read-only). Same pigeonhole argument, same candidates, same `consider` (so the same ordering rule and
   output); only candidate generation differs. Estimated 751 k x 16 x 17 = 204 M bit tests: about 0.15 s on one thread, 0.03 s
   on 6 workers, against 0.21 s of sorting plus the stages: the pass for a few files lands near 0.35 s.
8. **Kernel loop nest, selected by measurement (G4).** Candidates: the current one (query vectors, stored broadcast) with the
   suffix start; the swapped one (stored vectors, query broadcast: mean bucket 11.5 stored against 6.1 queries, fewer padding
   lanes) with the stored side split into not-due and due arrays so the due ones are a prefix; register-tiling the query
   vectors across the 17 probes. All kernels stay behind the same dispatch and the same tests (identical hits to brute force);
   the winner replaces the rest. No better AVX2 popcount exists for this shape (nibble table, two shuffles per word, no
   accumulation across candidates, so Harley-Seal does not apply; a one-word pre-test rejects only about 45%: a random 64-bit
   distance has mean 32, sd 4, against the threshold 31). AVX-512 VPOPCNTQ is not built (Not doing).
9. **Exactness, checked three ways:** brute force at every due share, kernel, thread count and both paths (existing tests,
   extended for the ordering suffix: stored indices below, equal to and above the queries, mixed due/not-due buckets, bucket
   boundaries); the radix sort against the counting sort on random and real chunk values; and the **real-data oracle**: the pass
   on the scratch catalog reproduces `e2e\after_pairs.txt` (5,277 pairs) exactly.

## Part 3: the pass and its SQL

1. **The load** (`pairing_work`) on its own read connection, which `Catalog::reader(cache_kib)` opens. `Catalog` does
   not keep its path (`core.rs:32-38`), so the reader takes the main connection's own file name (`Connection::path()`).
   It opens it read-only (`SQLITE_OPEN_READ_ONLY`, no create), so it needs no schema check: the main connection was
   checked at open, and the `Catalog`'s shared hold on the file (`take_shared`) already keeps a rebuild out. It sets
   only `busy_timeout` and the bounded `cache_size`, and reads in one transaction (the one snapshot `pairing_work`
   needs). **An in-memory catalog (the unit tests) reads on the main connection:** a second connection to `:memory:`
   is another, empty database, and the tests would pass on nothing. The query: `FROM analysis a CROSS JOIN files f` (removes the temp B-tree),
   rows decoded from `ValueRef` blobs straight into the structure of arrays (ids, owed flags, the 4 orientations), the legacy
   count (reliable quality, NULL blob) folded into the same scan, `Vec`s sized from the first count (today's 17 growth steps
   allocate 159 MB for an 80 MB vector), "nothing owed" tested with `has_owed_pairs()`. **Measured on one harness (Python, so
   decoding is slower than Rust's; relative):** today 436 ms plus a separate 69 ms legacy count; the planned shape (cross join,
   `LEFT JOIN pairs_owed`, 128-byte blob, no `pdq`, legacy folded in) 256 ms: -41%, and the count gone. In Rust about 0.37 s ->
   0.2 s. The read connection's `cache_size` is bounded (the pass reads `analysis` once, sequentially, and does not need
   the app's 256 MB), and the connection is closed after the load, which frees its cache. The value is picked in A by
   peak commit and load time at 16, 32, 64 and 256 MB; expected commit -100 MB. **The main connection is bounded too, for
   the pass's writes:** the guarded writes read every owed `analysis` row through it, which would fill its 256 MB cache.
   With one slice that happens after the join has freed its memory (harmless). With the write overlap (item 2) it adds to
   the join's memory. So the pass sets the main connection's `cache_size` to the same bound for its duration and restores
   the app's value afterwards, with `PRAGMA shrink_memory`. The restore is a drop guard, so an error, a stop or a panic in
   the pass also restores it; a `Catalog` method returns the guard (`Catalog::bounded_cache(kib)`). The peak is recorded with the overlap running, which on this
   library means a forced slice of 50,000 (4 slices, Part 2 item 5's parameter). A 400,000 slice is one slice here, the
   same as 200,000, so it cannot show the overlap. This matters beyond the peak: the runner keeps its `Catalog` across
   rounds (`runner.rs:378,427`), so today the cache the load fills (167 MB outside the Rust heap) stays in the app's
   process after the pass. A closed read connection and a shrunk main cache give it back.
   **Not a side table for the hashes:** measured on the migrated copy (WITHOUT ROWID `analysis_hashes`, blob moved out,
   `VACUUM`ed) the load is 294 ms against 244 ms and the file is the same size (177.5 against 176.3 MB).
2. **Writes** as Part 1 item 1. **The write overlap for multi-slice libraries (above 200,000 pictures):** a scoped thread runs
   `join(k+1)` while the calling thread (which owns `&mut Catalog`) writes slice `k`; with one slice there is nothing to
   overlap (a pair can come from any chunk, so no `done` mark is safe before the join ends). The join's control is
   `ctl.checkpoint_until(&abort)`; a write error (or a panic, through a drop guard) does `abort.store(true)` then `ctl.wake()`,
   so a join paused by the user is released and the error is not held until the resume (`control.rs:177,212`); the scope joins
   the thread before the pass returns and a panic in it resumes on the caller; the thread budget counts workers, sorters and
   this writer.
3. **`Catalog::prepare` stops repeating work** (0.41 G -> about 0.1 G Ir at every app and CLI start): `mark_unheld_missing` runs
   only when the build's held-extension set changed (a fingerprint in `meta.held_exts` via `set_mark`, like `taken_zone`;
   the scan is unchanged when it does run: -0.22 G). **The scan's file writes stamp the mark too.** Inside the
   transaction that inserts or revives `files` rows (`writers.rs:110`, and the take-overs in `pair.rs`), one `UPDATE meta
   SET value = ?1 WHERE key = 'held_exts' AND value IS NOT ?1` runs per transaction. Otherwise this sequence leaves unheld
   rows present for good:
   1. build A, holding more extensions, starts: mark = A;
   2. build B starts: mark differs, so B scans, marks A's extra rows missing, and sets mark = B;
   3. A, still running, scans and records or revives rows of its extra extensions;
   4. B starts again: the mark is still B, so it skips the scan, and the grid's list fails on those rows
      (`lifecycle.rs:113-117`).

   With the stamp, step 3 sets mark = A, and step 4 runs the scan. and `prepare` uses the cached `current_expected()` when the found version is the
   current one instead of building `expected(found)` again (-0.09 G).
4. **Observability, permanent.** `PairStats` gains `timings` (load, sort wait, scan, write, commit) from `Instant`s
   (`Duration` keeps its `Eq` derive); `gallery pair` prints them: `Line::PairDone` through `cli/src/lib.rs`,
   `client/src/text/cli.rs` (struct, formatting, tests) and `cli.ftl` (`en` only); the probe prints them too. This replaces the
   temporary timers used for the measurements.
5. **Follow-through found by reading:** `Picture` and `join` are used by `pairs.rs` and about 15 sites in `join.rs`'s tests (the
   corpus builders, `brute`, `run`, the `dump_join` record layout) and nowhere else; `PairingWork` by `catalog/src/lib.rs`, the
   writers test and `library/tests/pairs.rs`; `dups_state` by `fixture/state.rs`, `library/tests/pairs.rs`,
   `browse/read/tests.rs`, `schema.rs` and `writers.rs` tests. Each follows the new shapes: `fixture::state::dups_state`
   (a `String`) becomes `owes_pairs(c, name) -> bool` (a row in `pairs_owed`). `fixture::state::fail_pair_writes` (a
   trigger on `pdq_pairs`, `state.rs:125`) needs no change, and as a schema object the read connection sees it too.
   The GUI shows no count for the Pairing phase (`desktop/src/library.rs:188`), so a single slice changes nothing on
   screen.

## Part 4: order of work, harness and gates

One build at the end of each block (repo rule: implement the block fully, compile once), native release builds for every
measurement, Linux container only for `just perf`.

- **A. Harness and baselines first.** `pairs_probe` prints the timings; a script in `tools/profiling/` runs it at 1, 2, 4, 6, 12
  workers (4 interleaved reps, fresh catalog copy each, CPU load logged and runs over 30% flagged, medians) and the per-join
  callgrind run (`--zero-before/--dump-after` on the join's entry point: `pg_dedup::join::join` today, the `Joiner`'s
  per-slice method after block B, kept `#[inline(never)]` so the toggle has a symbol to match; `rust-src` linked at
  `/rustc/<hash>`, `parse_slices.py` kept); `dhat_run.ps1` + `dhat_parse.py` for the heap (profile: `cargo build --profile dhat --features dhat-heap`);
  `set_due.py`, `small_pass.ps1` (N owed, CLI waited for with `Start-Process -Wait`: `gallery.exe` detaches from the console),
  `peak_mem.ps1` (sampled peak commit). **The scratch catalog is rebuilt from `e2e\snap\catalog.sqlite.pre-v9.bak` by letting
  the CLI migrate it (v12, 5,277 pairs, 189,520 files); `hashes8.bin` is regenerated from it.** The "before" goes into
  `docs/reference/PROFILING.md`; `after_pairs.txt` is the oracle.
- **B. Data and the join's interface, one block (Part 1; Part 2 items 1 - 4 and 9).** B changes `PairingWork` to 4
  orientations, which the old `join` (`[Hash; 8]` per `Picture`) cannot take, so the two cannot compile apart. Then **C. Pass
  and SQL (Part 3 items 1, 3, 4, 5, Part 1 queue items)**, **D. Pipeline, slice size, few-owed path, kernel (Part 2 items
  5 - 8) and the write overlap (Part 3 item 2)**, **E. benches and docs.** In B, the radix sort writing three separate
  arrays is timed against the prototype's 309 ms (side-by-side records) before the kernel work.
- **Gates (each result is written into `docs/reference/PROFILING.md`, nothing is "estimated").** G1 and G2 are done (above). A
  gate sets a tuning value, not a go/no-go: **G3** sorter threads (1 - 2) and ring depth under the memory cap (replaced
  in the second round by one thread team, which tunes no count); **G4** the loop
  nest (Ir per owed candidate under callgrind, wall at 1 and 12 workers, synthetic corpus and real data); **G5** `SLICE` of
  50,000, 100,000 and 200,000 on this library (the prototype predicts 25.3 -> 21.2 -> 19.0 G Ir and one stored sort at
  200,000), with 400,000 tried on a synthetic 500,000-picture corpus (the real hashes plus bit-noised copies, from the
  `bench` feature), since 400,000 is one slice here; the smallest within 3% of the best, peak commit recorded with each,
  and with the write overlap running at the multi-slice settings; **G6** the few-owed path's bit-test pass and the crossover
  against sorting (a prototype on the real hashes sets the threshold); **the cache size** of the pass's connections;
  **G2b** the migration and `VACUUM` repeated under the app's pragmas (`temp_store = MEMORY`, `cache_size`, WAL), with
  peak commit and WAL size recorded. G2 ran under Python's defaults.

## Not doing, with the evidence

- **AVX-512 VPOPCNTQ kernel:** nothing here can run or verify it; an untested kernel is worse than none.
- **Other chunk counts:** 16 chunks at radius 1 is the cheapest of the family (11 at radius 2 measured and lost; 32 at radius 0
  gives 30x the candidates). The grouping was wrong, not the count (Part 2 item 4).
- **A thread cap at the physical cores:** stages take 1.16 s at 6 workers and 0.88 s at 12; the default (one worker per logical
  CPU) stays.
- **Persisting the sorted hash index between passes:** a second source of truth beside `analysis`, stale after every import; the
  few-owed path removes the cost it would save.
- **Chunk-outer join across slices** (stored side sorted once per chunk for all slices): up to 200,000 pictures it is the
  same as the one slice chosen (Part 2 item 5), which also gives up incremental writes there. The difference is above one
  slice: chunk-outer would hold every slice's query side at once and write nothing until all 16 chunks are done. Slices of
  200,000 with the write overlap keep memory bounded per slice and write each slice as it ends.
- **An index on `files(ext)` for `mark_unheld_missing`:** an index maintained on every file write to save 64 ms per start; the
  mark removes the scan instead.
- **A queue table for `borders_due`:** the borders UPDATE writes its result (`cut_*`) in the same statement, so there is no
  state-only write to remove; the saving there is Part 1 item 2's smaller row.
- **A side table for the hashes:** measured slower for the pass and the same size (Part 3 item 1); its possible benefit to
  other readers of `analysis` (grid and facets' cold reads) is unmeasured: its own plan if those are measured.
- **The import's SQL:** a flush is about 220 k Ir per file against `analyze`'s 63.5 M (0.35%): nothing to win.
- **The import's CPU (not this data):** `analyze` is decode 34%, resizes 30%, PDQ 26% (the box filter alone 21% of it, 65% of
  L1 read misses), ThumbHash 16%; that profile is from 2026-10-31 and predates the new resampler, so a fresh `just perf` comes
  before any change. Its own plan.

## Tests and verification

- `crates/dedup`: brute force at due shares 0, 10%, 50%, 100% x every kernel x threads 1..N x slices of 1 and the whole set x
  both candidate paths (the few-owed threshold forced each way); ordering-suffix cases; radix (three-array output) against
  counting sort; `consider` reading the permuted store equals brute force on orientations 4..7 of owed pictures; word-level
  against bit-level transpose on random and real hashes; the diagonal permutation preserves Hamming distances and the join on
  it equals brute force; both candidate paths return identical hits at the crossover; stop and pause return promptly with
  sorter threads running. (The symmetric-detection tests go with the fallback.)
- `crates/analysis`: the 4 returned hashes equal Meta's dihedral hasher's first 4, and `v+4` equals `transpose(v)` on real inputs.
- `crates/catalog`: TO_13 from a draft-v12 catalog (owed rows carried, `dups_state` and `pdq` gone, symmetric rows cut to 128
  bytes, a corrupted row nulled, reported as legacy and **left with no queue row**, the guards work on the blob, `VACUUM`
  ran, free-space check); **the same from a v12 file through a rebuild** (`Source::Carried`: the same owed rows, the blobs
  cut, no `dups_state`); the data step run twice changes nothing; **a failed `VACUUM` leaves the migration committed**
  and `vacuum_owed` set, and the next start retries it; `INSERT OR REPLACE` of an analysis clears the queue row
  through the cascade and `store_analysis` puts it back when the analysis is eligible; the
  table (FK cascade, `CARRIED`/rebuild copy, `present_pdq_pairs` still works); `store_analysis` owes / stops owing; `done` guard
  when the hash changed in between; `owe_pairs_again`; `pairing_work` one snapshot, legacy counted; `NOT_DUPLICATES` empty
  while anything present is owed and exact otherwise; `prepare` reusing the cached schema and running `mark_unheld_missing` once
  per extension-set change, **and again after a scan by a build with another set has stamped the mark**; the free-space
  paths (no room for the copy fails `prepare` untouched; no room for `VACUUM` migrates and sets `vacuum_owed`; unknown
  proceeds), with `space` faked through a seam; the pass's cache bound is restored after an error and after a stop;
  `Catalog::reader` on a file catalog reads the same rows as the main connection and sees a commit made just before;
  on an in-memory catalog it uses the main connection.
- `crates/platform`: `space` returns `free <= total` on a real folder (the existing `total_bytes` test, extended).
- `crates/library/tests/pairs.rs` (kept: the import owes and the pass makes the pairs; a stop before the pass writes nothing
  and the pairs stay owed; a batch that cannot be written stores nothing and the next pass makes it all) plus, with the
  slice forced small (Part 2 item 5): several slices give the pairs one slice gives; a stop between slices keeps the
  written slices and leaves the rest owed; a failed write while the next join runs stops it and returns the error,
  **including when the join is paused**; a path taken
  over by another file (`pair.rs:424`) leaves no queue row; two passes at once leave the same table; the runner's pass-order test
  (Pairing right after Import, before Borders and Thumbs; skipped after a preempt).
- **Real data end to end (scratch copies only):** `gallery pair --again` on a copy reproduces `after_pairs.txt` exactly; timings
  at 1, 2, 4, 6, 12 workers against the "before" table; callgrind per-join Ir: symmetry < 0.3 G (17.7 G), sorts < 1 G (2.6 G),
  scan <= 20 G (54.6 G; 19.0 G predicted), SQL writes < 1 G (2.8 G), `prepare` < 0.15 G (0.41 G), load < 0.8 G (1.49 G); the
  small pass (target 0.35 s) and peak commit (cap 330 MB, removed in the second round) against A's baselines; one fresh scan + import + pair of the
  pinterest root.
- Benches (`crates/bench`, gungraun, `just perf`): `pairing.join` (a deterministic synthetic corpus of about 20 k hashes with
  clusters, from a `pg-dedup` `bench` feature like `pg-scan`'s) and `pairing.write`; baselines with `just perf --update`, budgets
  in `docs/perf/budgets.json`.
- Clippy `--all-targets` clean and tests green at the end of each block.

## Docs

`docs/reference/PROFILING.md` (method, the per-join callgrind recipe, numbers before and after, each gate's result);
`crates/README.md` (dedup, import, catalog rows); `docs/plans/DECODE_ONCE_PLAN.md` §3.3-3.7 (the Dups pass is the queue table
`pairs_owed`, not a column: reconcile its TO_10 claims), `BACKGROUND_JOB_AUDIT_FIXES.md` §C item 2, `GUI_THREAD_PLAN.md`
Part 2a, `PLAN.md:147-151`, `BOUNDED_MEMORY_PLAN.md` D13/D16/D25 (plus the pass's per-picture memory, Part 2 item 6), `REVIEW.md` R-84/R-109/R-126, `HEAP_PROFILE.md` (no import
index); `cli.ftl` (the timings line). Commit only when asked, on `main`, no new branch.
