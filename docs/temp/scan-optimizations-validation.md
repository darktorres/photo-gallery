# Scan step: validation of the optimization list

Date: 2026-11-15. Scope: the scan step of import (`pg_scan::scan`) only. Pairing (`pg_catalog::pair`) is out of
scope and is not assessed here, except where an index or a trigger it depends on is also paid by the scan's writes.

The list was validated in four rounds the same day:
- a first validation against the code and one profile, with two experiments;
- a reevaluation that reparsed every profile and read the bundled SQLite's source;
- an adversarial pass that tested the new claims by experiment;
- a closing round that measured the main fix in the app itself, on 200,000 files, and in wall-clock time on Windows,
  and closed the remaining gaps.

Where a round corrected an earlier one, this report gives the corrected figure; **Corrections** lists what changed.

## Bottom line

The scan's main per-file cost on a first scan comes from two things together:
- the `files_added` trigger;
- a statement journal SQLite opens for every file's INSERT, because of the trigger and the upsert's own DO UPDATE
  clause.

Folding the trigger's values into a plain INSERT for files the writer didn't find removes both. Measured in a patched
copy of the app:

| | Shipped | Patched | Change |
|---|---:|---:|---:|
| First scan, 4,000 files (callgrind) | 895.0M Ir | 506.9M Ir | −43.4% |
| First scan, 200,000 files (callgrind) | 48.25G Ir | 28.15G Ir | −41.7% |
| First scan, 4,000 files (Windows, warm, median of 5) | 0.195 s | 0.096 s | −51% |
| First scan, 4,000 files (Windows, cold, median of 5) | 0.247 s | 0.141 s | −43% |
| First scan, 200,000 files (Windows, warm with dirty pages flushed, median of 5) | 13.09 s | 6.88 s | −47% |
| First scan, 200,000 files (Windows, CPU time of the same runs) | 10.81 s user | 4.84 s user | −55% |
| First scan, 200,000 files (Windows, cold, median of 10) | 15.26 s | 9.55 s | −37% |
| Rescan, 4,000 / 200,000 files (callgrind) | 158.0M / 8.23G Ir | 156.8M / 8.22G Ir | none (no new files) |
| Rescan, 200,000 files (Windows, cold, median of 5) | 3.29 s | (same code path) | |

Two build settings add to it:
- a larger lookaside (more slots): on Windows it halves the fixed write again;
- memory statistics off: −10%.

The rescan has its own costs:
- the `seen` UPDATE's journal and foreign-key check;
- an `in_root` query for every unchanged file;
- with cold caches, the listing: a third of a cold rescan, and 6.7× faster on 8 threads.

## Method

- **Scan probe:** `crates/library/examples/scan_probe.rs` (untracked). It adds a folder to a fresh catalog file, then
  times one `pg_scan::scan` call. With `PG_PROBE_AGAIN` set, it scans once untimed first, so the timed scan is a
  rescan.
- **Libraries:**
  - `target/scan-probe/lib`: 4,000 placeholder files in 80 folders.
  - The large shape, 400 folders of 500 files (200,000), made by `target/scan-probe/gen_large.py`: in the docker
    volume `pg-scan-large` for callgrind, and in `target/scan-probe/large` on NTFS for Windows.
  - The scan reads names, sizes and times only, so the files hold filler bytes. The synthetic `large` profile itself
    writes real pictures, which the scan never opens.
- **Callgrind:** release builds in the `pg-tools` container, `valgrind --tool=callgrind` (3.26), Ir only.
  `--toggle-collect` restricts costs to one function, but call counts stay whole-process. So every count below is
  from a whole-process run, or labelled as such.
- **SQLite:** 3.54.1, bundled by the workspace's rusqlite fork (`darktorres/rusqlite`, rev `9d76035`). Line numbers
  are in that checkout's `libsqlite3-sys/sqlite3/sqlite3.c`.
- **First experiments:** the scan probe with two temporary switches (since reverted). One was
  `sqlite3_config(SQLITE_CONFIG_MEMSTATUS, 0)`; the other dropped `files_added` after `Catalog::prepare`, through a
  python3 child process.
- **Variant probe:** `crates/catalog/examples/stmt_journal_probe.rs` (untracked).
  - Each variant makes a fresh catalog with `Catalog::prepare` (the app's schema and triggers) and opens it raw with
    the catalog's connection settings.
  - It then writes the 4,000 files in one `BEGIN IMMEDIATE` transaction, as `write_batch` does: `ScanWriter`'s other
    statements prepared and held, and its `known` lookup before each write.
  - One variant per process (`stmt_journal_probe <variant>`). Callgrind collects `timed_part` only
    (`target/scan-probe/stmt_journal.sh`).
  - It prints the variant's wall time and its connection's lookaside counters (`sqlite3_db_status`).
  - `PG_PROBE_MEMSTAT=0` turns memory statistics off.
- **Patched app:** `target/scan-probe/tree`, a copy of `crates/` with two changes. Your working tree is not touched.
  - `files_added` is removed from the baseline schema.
  - `ScanWriter` writes files its `known` lookup didn't find (and displaced ones) with a plain INSERT. That INSERT
    sets `pending` = 1, and `seq` = `changed` = one `meta.seq` bump per batch.
  - In the container, the copy's `crates/` is mounted over `/work/crates`, with its own target dir
    (`target/scan-probe/app_level.sh`).
  - On Windows it was built with the main target dir, so dependencies were reused. The binaries are kept in
    `target/scan-probe/bin`.
- **Build options:** `target/scan-probe/fork` is a copy of the fork whose `build.rs` adds `-D` flags from
  `PG_SQLITE_FLAGS`. It is used through a patched `Cargo.toml` and `Cargo.lock` mounted in the container only
  (`build_variants.sh`, `fork_runs.sh`). LTO used `CARGO_PROFILE_RELEASE_LTO=fat` with one codegen unit.
- **Windows:** native release builds, in an elevated shell.
  - Warm runs (`win_timing.ps1`): the variant probe, 5 runs each with memory statistics on and off; the scan probe,
    shipped and patched interleaved, 5 runs on the small library and 3, then 5 more, on the large one; and the lister
    alone, `crates/platform/examples/list_probe.rs` (untracked).
  - Flushed-warm and cold runs: `target/scan-probe/purge.ps1` empties the file cache before each run, as RAMMap
    does: the system cache's working set (`SetSystemFileCacheSize`), the modified list (written out), then the standby
    list (`NtSetSystemInformation`). With `-FlushOnly` it writes the modified pages out and keeps the cache.
    `win_cpu.ps1` records each run's CPU time with its wall time, flushed-warm and cold. `win_cold.ps1` runs cold
    listing (first pass on 1, 2, 4 and 8 threads), cold first scans, and cold rescans.
  - For a cold rescan, the scan probe's `PG_PROBE_BETWEEN` runs the purge between its untimed first walk and the timed
    one.
  - A purge does make the reads cold: the first listing of the large library took 1,177 ms after one, against 43 ms
    warm.
- **Parsing:** `target/scan-probe/cgx.py` (a call-edge parser that keeps objects, so PLT stubs can be followed to their
  callers), and `recheck.py` to `recheck6.py`.

## Measured results

### First experiments (scan probe, 4,000 files, whole process)

| Pass | `pg_scan::scan` | Change | Process | Change |
|---|---:|---:|---:|---:|
| Baseline | 895.0M | | 1,081.3M | |
| Memory statistics off (`sqlite3_config`) | 800.6M | −94.4M (−10.5%) | 956.2M | −125.0M (−11.6%) |
| `files_added` dropped | 678.8M | −216.2M (−24.2%) | 868.8M | −212.5M (−19.6%) |

- **The trigger's 216.2M is all in `write_batch`** (610.1M → 393.5M). Pairing is unchanged (259.9M → 260.2M).
- **The 3.7M gap between scan and process** in the trigger pass is the experiment's own cost. The child process changed
  the schema, so `add_folder` reloaded it (0.08M → 3.84M).
- **Memory statistics off** also saves 30.4M in `Catalog::prepare`.

### The fix in the app (patched copy, callgrind)

| Run | Scan | `write_batch` | Pairing | `prior_states` | `mark_missing` | Statement journal |
|---|---:|---:|---:|---:|---:|---:|
| Shipped, 4k, first | 895.0M | 610.1M | 260.0M | 9.2M | 5.4M | 215.1M |
| Patched, 4k, first | 506.9M | 222.1M | 259.9M | 9.2M | 5.4M | 1.8M |
| Shipped, 200k, first | 48.25G | 32.69G | 13.98G | 0.92G | 0.18G | 10.89G |
| Patched, 200k, first | 28.15G | 12.60G | 13.97G | 0.92G | 0.18G | 0.06G |
| Shipped, 4k, rescan | 158.0M | 121.3M | 7.6M | 13.7M | 5.0M | 11.9M |
| Patched, 4k, rescan | 156.8M | 120.1M | 7.6M | 13.7M | 5.1M | 12.4M |
| Shipped, 200k, rescan | 8.23G | 6.42G | 0.41G | 0.73G | 0.18G | 0.60G |
| Patched, 200k, rescan | 8.22G | 6.41G | 0.41G | 0.72G | 0.18G | 0.60G |

- **First scan:** `write_batch` falls 63.6% (4k) and 61.5% (200k). The journal all but goes.
- **Scale:** the cost per file grows with the library. The first scan's `write_batch` is 152.5k Ir per file at 4k and
  163k at 200k (shipped), and 55.5k and 63k (patched).
- **After the fix, pairing is the largest part of a first scan at 200k:** 13.97G of 28.15G (50%). Pairing is out of
  scope.
- **The rescan is unchanged,** since it writes no new files. Its per-file cost at 200k is 41k Ir. `write_batch` is 32k of
  that: in `upsert_file`, the `seen` UPDATE is 14.8k per file, the `known` lookup 11.5k, and the `in_root` query 4.6k.

### The fix on Windows (wall clock)

| Run | Shipped | Patched |
|---|---:|---:|
| 4k, first scan, warm (5 runs) | 0.195 s | 0.096 s (−51%) |
| 4k, first scan, cold (5 runs) | 0.247 s | 0.141 s (−43%) |
| 4k, rescan, warm (5 runs) | 0.056 s | 0.056 s |
| 4k, rescan, cold (5 runs) | 0.138 s | (same code path) |
| 200k, first scan, warm, back to back (5 runs) | 12.81 s | 7.91 s (−38%) |
| 200k, first scan, warm with dirty pages flushed (5 runs) | 13.09 s | 6.88 s (−47%) |
| 200k, first scan, cold (10 runs) | 15.26 s | 9.55 s (−37%) |
| 200k, rescan, warm (3 runs) | 2.27 s | 2.65 s (noise: the code path is the same) |
| 200k, rescan, cold (5 runs) | 3.29 s | (same code path) |

- **CPU time** of the flushed-warm 200k first scans: shipped 10.81 s user and 0.97 s kernel, patched 4.84 s and 0.92 s.
  Cold, the same within 0.4 s. About 1.1 to 1.3 s of each run is off the CPU.
- **Back-to-back runs are noisy:** one patched run took 36.8 s, against 7 to 9 s for the others. Each run writes a
  fresh catalog, and the cache writes its pages back while later runs go. Flushing the dirty pages before each run
  narrows the spread.
- **Cold adds about 2.2 s (shipped) and 2.7 s (patched)** to a 200k first scan. About 1.1 s of that is the listing (see
  **The Windows lister**); the rest is the other cold reads, the program's own code pages among them.
- **Wall time tracks the instruction counts:** at 200k the shipped scan runs about 3.8 billion instructions a second
  (241k Ir per file in about 64 µs), so warm it is CPU-bound.

### Variant probe (4,000 files, one batch)

Instruction counts are inside the timed part. Windows times are medians of 5 native runs, with memory statistics on.

| Variant | Ir | Statement journal | Lookaside miss path | Windows |
|---|---:|---:|---:|---:|
| a: the app's upsert | 578.7M | 202.4M | 31.9M | 136.8 ms |
| b: plain INSERT, trigger kept | 571.3M | 200.1M | 31.7M | 136.6 ms |
| **c: plain INSERT, trigger folded** | **197.9M** | **0** | 13.1M | **44.4 ms** |
| d: upsert, `files_added` dropped | 356.5M | 150.3M | 13.3M | 84.3 ms |
| f: as d, foreign keys off | 346.4M | 152.5M | 11.8M | 84.4 ms |
| g: upsert, every trigger on `files` dropped | 338.0M | 148.9M | 3.7M | 65.5 ms |
| h: as g, foreign keys off | 173.5M | 0 | 2.3M | 22.6 ms |
| t1: upsert, `files_added` with its `meta` bump only | 396.8M | 169.3M | 16.9M | 104.3 ms |
| t2: upsert, `files_added` with its `files` UPDATE only | 541.3M | 177.9M | 29.1M | 128.1 ms |
| k: one upsert statement for the batch, trigger kept | 468.8M | 3.1M | 6.0M | 53.8 ms |
| k2: one folded INSERT for the batch (`json_each`) | 340.4M | 1.9M | 6.7M | 38.9 ms |
| i: changed files, through the upsert | 638.0M | 142.5M | 45.6M | 146.7 ms |
| j: changed files, plain UPDATE of the known row | 602.3M | 143.0M | 42.3M | 149.0 ms |
| e: `seen` UPDATE per file | 100.2M | 30.8M | 2.2M | 16.4 ms |
| e2: as e, foreign keys off | 61.6M | 0 | 2.2M | 7.7 ms |
| e3: one `seen` UPDATE for the batch | 66.0M | 1.4M | 0.05M | 8.5 ms |
| la-a-200: as a, lookaside 1200 × 200 | 528.8M | 203.1M | 3.3M | 103.7 ms |
| la-c-200: as c, lookaside 1200 × 200 | 175.1M | 0 | 0.01M | 21.6 ms |
| a, memory statistics off | 497.3M | 157.3M | 12.9M | 122.5 ms |
| c, memory statistics off | 182.9M | 0 | 4.8M | 40.3 ms |
| e, memory statistics off | 86.2M | 19.7M | 0.8M | 14.5 ms |

- **Calibration:** variant a costs 578.7M against `upsert_file`'s 603.2M in the scan, and its journal 202.4M against
  213.9M. The probe is within 5% of the app. The app-level run then confirmed the probe's prediction: c − a = −380.8M in
  the probe, and −388.0M in the app's `write_batch`.
- **Lookaside:** 500 and 1,000 slots gave the same as 200, within 1% in instruction counts and noise on Windows. With
  200 slots, the full-pool misses go to zero.
- **Trigger split:** of the trigger's 222.2M (a − d), its `files` UPDATE alone adds 184.8M (t2 − d), and its `meta` bump
  alone 40.3M (t1 − d). On Windows: 44 ms and 20 ms.
- **Indexes** (each dropped from c, in Ir; Windows differences were within noise, 0 to 10%):

  | Index | Saving |
  |---|---:|
  | `files_order` | 20.9M |
  | `files_to_import` | 17.9M |
  | `files_changed` | 16.4M |
  | `files_seq` | 16.3M |
  | `files_fresh` | 16.2M |
  | `files_folder` | 16.0M |
  | `files_present` | 14.5M |
  | `files_ref` | 12.4M |

  Together, 131M: two thirds of the fixed write.

### Build options (scan probe, 4,000 files, whole process)

| Build | Scan | Change in scan | Process |
|---|---:|---:|---:|
| Fork copy, no extra flags | 896.1M | | 1,082.3M |
| `-DSQLITE_DEFAULT_MEMSTATUS=0` | 805.7M | −90.4M (−10.1%) | 961.3M |
| … and `-DSQLITE_DEFAULT_LOOKASIDE=1200,200` | 784.6M | −111.5M (−12.4%) | 940.4M |
| … and the `SQLITE_OMIT_*` set below | 787.7M | +3.1M on the line above | 943.0M |
| Shipped tree, fat LTO, one codegen unit | 888.4M (vs 895.0M) | −6.6M (−0.7%) | 1,072.8M |

The `SQLITE_OMIT_*` set was `-DSQLITE_OMIT_DEPRECATED -DSQLITE_OMIT_SHARED_CACHE -DSQLITE_LIKE_DOESNT_MATCH_BLOBS
-DSQLITE_MAX_EXPR_DEPTH=0 -DSQLITE_USE_ALLOCA`. Every build scanned the 4,000 files the same way.

### The Windows lister

| Library | 1 thread | 2 threads | 4 threads | 8 threads |
|---|---:|---:|---:|---:|
| 4k (80 folders), warm | 3.07 ms (767 ns/file) | 1.70 ms | 1.29 ms | 1.11 ms |
| 200k (400 folders), warm | 38.1 ms (190 ns/file) | 26.4 ms | 12.5 ms | 8.6 ms (43 ns/file) |
| 200k (400 folders), cold (median of 5) | 1,091 ms (5.5 µs/file) | 491 ms | 268 ms | 164 ms |

- **Warm**, listing all 200,000 files takes 38 ms on one thread, 0.3% of the first scan.
- **Cold** it takes 1.09 s: 7% of a cold first scan (15.3 s), and a third of a cold rescan (3.29 s). Eight threads list
  it 6.7× faster.

## Mechanisms

### The statement journal

- **The rule.** SQLite opens a statement journal when a statement is compiled as both multi-write and may-abort
  (`usesStmtJournal = isMultiWrite && mayAbort`, `sqlite3.c:90179`), and runs inside a transaction, as each batch does.
  Each page the statement first changes is copied into the journal, and the copies are freed when it ends.
- **The size.** On the 4,000-file first scan that is about 58,800 page images, about 15 per file, since each file is its
  own statement. Inside `write_batch`:
  - the copies cost 156.5M: of that, chunk mallocs 99.8M and `memcpy` 32.4M;
  - freeing them costs 57.4M;
  - together 213.9M, 23.9% of the scan.
- **Why the scan's upsert gets one.** It is may-abort through its NOT NULL columns (`sqlite3.c:141347`). It is
  multi-write for two independent reasons:
  - the `files_added` trigger (`sqlite3Insert`, `pSelect || pTrigger`, `sqlite3.c:140344`);
  - its DO UPDATE branch. That branch is compiled into the INSERT's own `Parse` (`sqlite3UpsertDoUpdate`), and it sets
    FK columns (`folder_id`, `last_scan`) and columns with AFTER UPDATE triggers (`sqlite3Update`,
    `pTrigger || hasFK`, `sqlite3.c:161071`).
- **The variants confirm each cause.** Variant f (foreign keys off) and variant g (no triggers) each keep the journal;
  only h (neither) loses it.
- **Why a plain INSERT doesn't.** A single-row INSERT that isn't multi-write checks a missing parent row at once, without
  a journal (`fkLookupParent`, near `sqlite3.c:138283`). So with no trigger, it has none (variant c).
- **Why the copies cost so much.** `temp_store = MEMORY` keeps the journal in memory (`nStmtSpill = -1`,
  `sqlite3.c:63541`), in 1 KB chunks (`MEMJOURNAL_DFLT_FILECHUNKSIZE`, `sqlite3.c:109836`). So each 4 KB page image is
  about four mallocs.
- **What `memcpy` is spent on.** Of the process's 48.8M `memcpy`, 41.2M runs through PLT stub `0x40af030`, and 32.4M of
  that is these copies.

### The trigger

- **What it runs.** For each new file, `files_added` bumps `meta.seq` (a single hot row) and updates the new row's
  `pending`, `seq` and `changed`.
- **What it adds.** 4 B-tree inserts per file (`sqlite3BtreeInsert` 74,593 → 58,593, whole process) and 222M per 4,000
  files (a − d), of which 52M is its share of the journal.
- **Its parts.** The `files` UPDATE is most of it (184.8M); the `meta` bump is 40.3M.

### Memory statistics

- **Where the cost is.** `sqlite3Malloc` and `sqlite3_free` take a global mutex when memory statistics are on, the
  default (`sqlite3.c:32126`). That is 590,349 locks each on the 4,000-file scan.
- **The leftover mutex calls.** The 580,853 `sqlite3_mutex_enter` calls left with statistics off are no-ops (2.6M in
  all). The connection's mutex is NULL, since rusqlite opens with `SQLITE_OPEN_NO_MUTEX` (fork `src/lib.rs:1327`).
- **Nothing depends on the statistics.**
  - No code in `crates/` or in the rusqlite fork calls `sqlite3_memory_used`, `sqlite3_memory_highwater`,
    `sqlite3_status`, or a soft or hard heap limit.
  - The app's `release_memory` (`app/src/reader.rs:182`, `sqlite3_db_release_memory`) shrinks the page cache and doesn't
    read the statistics.
- **The flag works.** `-DSQLITE_DEFAULT_MEMSTATUS=0` in the fork's `build.rs` saves what the runtime call saved (90.4M
  against 94.4M), with no `unsafe` code.

### Lookaside

- **The default pool is too small.** The default `1200,40` gives 30 slots of 1,200 bytes and 93 of 128
  (`sqlite3.c:24346`; the build sets no lookaside flag).
- **The misses are a full pool, not oversized allocations.** In the app-like write (variant a), `sqlite3_db_status`
  counts 157,281 misses with every slot taken and 8,012 for size, against 40,435 hits. All 123 slots were in use. The
  size misses are the trigger's: about two per file, and 12 without it.
- **200 slots is enough.** At 1200 × 200 the full-pool misses go to zero.
- **It matters far more on Windows.** The Windows heap is much more expensive than glibc's: there the same change saves
  24% (a) and 51% (c), against 9% and 12% in instruction counts.

### The rescan

- **The `seen` UPDATE** (`UPDATE files SET last_scan`) sets an FK column that is also NOT NULL, so it gets a statement
  journal per file. In the app it costs 14.2k Ir per file at 4k, 21% of that the journal.
  - Turning foreign keys off (e2) removes the journal and the parent lookup: −39% in Ir, −53% on Windows.
  - So does one statement for the batch (e3): −34% in Ir, −48% on Windows.
  - The probe's per-file figure (25k Ir) is higher than the app's (14.2k). The difference is not explained, so use the
    app's figure.
- **The `in_root` check** runs for every file whose row is present (`writers.rs`, `!hit.missing && !in_root...`). That
  includes every unchanged file on a rescan: 17.8M at 4k and 0.91G at 200k, 11% of the rescan. It is trivially true when
  the row's folder is the folder being written, so a `hit.folder_id != folder.id` guard before it would skip it on a
  rescan. Not measured.

## Design checks for the main fix

- **Only one production INSERT exists.** `INSERT INTO files` appears in production code only at `writers.rs:110`. The
  others are in tests (`schema.rs`, `roots.rs`, `pair.rs`, `ledger.rs`, `lifecycle.rs`) and in the `fixture` feature
  (`catalog/src/fixture.rs:102`), which other crates' tests use. They rely on the trigger for `pending`, `seq` and
  `changed`, so they would have to set them.
- **The readers of `seq` tolerate a shared per-batch value.**
  - The thumbnail pass reads rows with `seq` above its checkpoint, with no LIMIT, and checkpoints at the snapshot's
    `seq`, read in the same snapshot (`thumbcache/src/reader.rs:30`, `lib.rs:284`). So a batch sharing one `seq` is read
    whole.
  - The ledger conversion orders by `f.seq, f.id`, so ties are broken (`convert.rs:206`).
  - `changed` has no production reader: only `test_support.rs:147`. The schema says it is for a client's resync.
- **Dropping the trigger is a schema change.** The patched copy used fresh catalogs; a real change needs a migration.
- **What the measurement covers.** The displace branch also takes the plain INSERT in the copy, but no file in the test
  libraries is displaced, so that path was not exercised. The changed-file path keeps the upsert. A plain UPDATE there
  (variant j) saves 5.6% in Ir and nothing on Windows.
- **The probe matches the scan's connection.** The scan's connection has no hooks or progress handler; only the UI
  reader's connection sets one (`cancel_when`).

## Item verdicts

Numbering follows the optimization list; 3b and 29 are new.

| # | Item | Verdict | Evidence |
|---|---|---|---|
| 1 | Fold trigger values into the INSERT | **Confirmed, measured in the app** | With 3b: −43.4% of the scan at 4k, −41.7% at 200k, −51% and −38% on Windows. Needs 3b: folding alone keeps the journal (variant d) |
| 2 | Reserve `seq` per batch | **Confirmed** | One bump per batch in the patched copy. The `meta` bump is 40.3M of the trigger's 222M (t1). A shared value is safe for today's readers (see **Design checks**) |
| 3 | Set-based insert per batch | **Not worth it** | One statement per batch removes the journal, but its JSON costs more than the journal saved. In Ir, k2 is 340.4M against c's 197.9M. On Windows k2 is 12% faster than c, but c with lookaside (21.6 ms) beats both |
| 3b | Plain INSERT for files the `known` lookup didn't find | **Confirmed, measured in the app** | Together with item 1 (see 1). Alone it saves nothing (variant b) |
| 4 | `prepare_cached` for scan statements | Valid, small | `ScanWriter::new` prepares 10 statements per batch: 1.6M Ir per batch of up to 10,000 entries (32.6M for the 20 batches of the 200k rescan, 0.4% of it) |
| 5 | Reduce per-row index maintenance | **Measured** | 12.4M to 20.9M per index per 4,000 files; together 131M, two thirds of the fixed write. `files_changed` (16.4M) has no production reader yet. `files_fresh` exists for pairing. Dropping any index is a design decision |
| 6 | Batch the known-row lookup | Valid | 17.2M (1.9%) on a first scan. On a rescan it is 11.5k Ir per file (it finds and reads the row) |
| 7 | Merge `prior_states` into the known lookup | **Refuted** | `prior_states` reads in its own snapshot when the folder is listed, before the batch's write transaction, so a write-back committed in between is kept. Read inside the write transaction, it would be overwritten |
| 8 | Batch the identity fallback (`by_identity`) | Valid | 4,000 empty `query_row` calls on a first scan: 7.9M (0.9%) |
| 9 | Resolve folder ids once per batch | Valid, minor | 80 calls: 3.0M (0.3%) |
| 10 | ASCII fast path for extensions | Valid, minor | `Ext::of_name_bytes`: 1.9M (0.2%) |
| 11 | Reuse name buffers | Valid | Each entry's `Name` is allocated at listing and cloned into the batch (`scan/src/lib.rs:558`) |
| 12 | Profile the NTFS lister | **Measured** | Warm: 190 ns per file at 200k, 0.3% of the first scan. Cold: 5.5 µs per file, 1.09 s: 7% of a cold first scan, 33% of a cold rescan |
| 13 | `seen` `BTreeSet` to a hash set | Valid, minor | Membership only, never iterated; `FileIdentity` derives `Hash`. 1.6M (0.2%) |
| 14 | Clone of a folder key per file | **Refuted** | The 4,000 clones are the per-file `Name` (item 11) |
| 15 | Larger batches | Small | A batch is up to 10,000 entries. On the 200k rescan the 20 commits cost 25.8M and the 20 writers 32.6M, together 0.7% of that scan |
| 16 | The rescan's unchanged rows | **Measured** | They take the `seen` UPDATE, not the upsert: 14.2k Ir per file in the app, with a statement journal because `last_scan` is an FK column. One statement per batch: −48% on Windows (probe). Foreign keys off: −53% (probe). Not tried in the app |
| 17 | Skip `mark_missing` on folders with no missing rows | Small | 5.4M at 4k, 0.18G at 200k (0.4 to 2.2% of the scan) |
| 18 | Memory statistics off | **Confirmed, flag route measured** | −90.4M (−10.1%) with `-DSQLITE_DEFAULT_MEMSTATUS=0` in the fork. Windows: −10.5% (a), −9.2% (c). Nothing uses the statistics |
| 19 | SQLite lookaside | **Confirmed, sizes measured** | 1200 × 200 removes the full-pool misses: −50M (a) and −23M (c) in Ir, −24% and −51% on Windows. 500 and 1,000 slots add nothing. As a build flag it costs 240 KB per connection (48 KB now), for every connection the app opens. Setting it only on the writer's connection (raw `sqlite3_db_config`) is the alternative |
| 20 | Faster allocator | Valid, mostly downstream | glibc malloc and free are 14.0% of the process. The lookaside's Windows result shows allocations cost far more there. Items 1, 3b and 19 remove most SQLite allocations. A different allocator was not tried |
| 21 | LTO, `codegen-units` | **Measured: −0.7%** | Fat LTO with one codegen unit: scan 895.0M → 888.4M. SQLite is C, built by `cc`, and is not part of Rust's LTO |
| 22 | `SQLITE_OMIT_*` | **Measured: no gain** | The set in **Build options** changed the scan by +3.1M. JSON1 is in use, so it can't be omitted |
| 23 | mmap to reduce `memcpy` | **Refuted** | Two thirds of `memcpy` is statement-journal copies, which `mmap_size` doesn't touch |
| 24 | Smaller rows | **Corrected** | `files` has no `key` column. The name bytes appear three times per row: `name`, then in `order_key` lower-cased and then exact (`name.rs:120`) |
| 25 | Fewer trigger-maintained columns | Confirmed | Same as item 1 |
| 26 | Overlap listing with writing | **Pays cold, not warm** | The walk is single-threaded. Listing is 0.3% of a warm first scan, but a third of a cold rescan (1.09 of 3.29 s), which overlapping could hide behind the writes. Not tried |
| 27 | Parallel listing | **Pays cold, not warm** | 8 threads list 4.4× faster warm (38 → 8.6 ms at 200k) and 6.7× faster cold (1.09 → 0.16 s). On a cold rescan that is up to 0.9 s of 3.29 s (28%). Constrained by the W2 rules and the lease (listing is outside transactions already). Not tried in the scan |
| 28 | Cold-cache runs on Windows with `--drop-caches` | **Refuted** | `scan_timing.py --drop-caches` writes `/proc/sys/vm/drop_caches`: Linux only |
| 29 | Skip `in_root` when the row is in the folder being written | **New, measured cost** | `in_root` runs for every present row: 11% of a rescan (17.8M at 4k, 0.91G at 200k). A `hit.folder_id != folder.id` guard skips it for unchanged files. Saving not measured |

One more claim in the original list does not hold: running the probe on a catalog copy with the trigger dropped.
`Catalog::prepare` compares the schema (`schema.rs`, `mismatch`) and rebuilds a mismatched catalog, so the experiment
dropped the trigger after `prepare` instead.

## Corrections

What later rounds changed in earlier ones:

- glibc malloc and free are 14.0% of the process, not about 12%.
- The trigger's share is 216M (24% of the scan), not unknown.
- The statement journal, a hypothesis in the first drilldown, is measured: 213.9M, all in the scan's writes.
- The `memcpy` callers, called unresolvable, resolve one level up the PLT stub, to the journal.
- Lookaside, first called likely small, is a full pool, worth 24 to 51% on Windows.
- `files` has no `key` column, and the name appears three times per row, not twice.
- The drilldown's index list included `folders_key`, which is on `folders`.
- Counts quoted as "inside `write_batch`" from toggle runs were whole-process: 328,576 lookaside misses, 182,066
  cursor allocations, and "96% of heap calls". The costs were right.
- "Mostly cursor allocations" was not established and is dropped.
- Item 3 was upgraded on reasoning, and is downgraded by measurement.
- The probe's `seen` UPDATE cost (25k Ir per file) is above the app's (14.2k); use the app's.
- A measurement error of the closing round, caught and redone. The patched copy was built on Windows into the same
  target dir as the shipped tree. Cargo names a path package's artifacts by its place in its workspace, so the copy's
  `pg-catalog` replaced the shipped one's release artifacts. A later rebuild of the shipped scan probe then linked the
  patched catalog.
  - The first cold and CPU runs therefore compared patched with patched, and showed no difference. They are kept as
    `win/cold-invalid.tsv` and `win/cpu-invalid.tsv`, and not used.
  - The warm runs are unaffected: they used the shipped binary copied before the patched build.
  - So are the container runs, which used their own target dirs.
  - `pg-catalog`'s release artifacts were cleaned and rebuilt, and both binaries were checked for the trigger's text
    (shipped: present, patched: absent) before the runs were repeated.
- Small figures: the scan total is 895.0M, not 894.7M; the memory-statistics saving is 10.5%, not 10.6%; the trigger
  pass's process change is −19.6%; the trigger's journal share is 52M, from the probe (the earlier 47M mixed a
  whole-process run with a restricted one).

## Still unmeasured

- **Parallel or overlapped listing in the scan itself** (items 26, 27): the lister alone was measured cold, the scan with it
  was not.
- **Cold runs of the variant probe:** only the scan probe and the lister were run cold.
- **Fixes measured only in the probe, not in the app:** the rescan's fixes (item 16), lookaside, and memory statistics
  on Windows. Items 6, 8, 11, 13 and 29 were not tried at all.
- **The displace path** under the fix.
- **Windows warm-rescan noise:** 3 runs, 2.27 s against 2.65 s on the same code path.
- **A different allocator** (item 20).
- **Pairing,** which becomes half of a large first scan after the fix (out of scope).

## Suggested priority

1. **Fold `files_added` into a plain INSERT for new files** (items 1, 2, 3b, 25). Measured in the app: −42 to −43% in
   instruction counts, −37 to −51% wall time on Windows warm and cold, −55% CPU time.
   - Needs a migration that drops the trigger.
   - The test fixtures have to set `pending`, `seq` and `changed`.
   - One `seq` per batch is safe for today's readers.
2. **Lookaside 1200 × 200** (item 19). −51% of the fixed write on Windows. It is one flag in the fork's `build.rs`, or one
   `sqlite3_db_config` on the writer's connection, to avoid the memory on every connection.
3. **Memory statistics off** (item 18). −10%, one flag, and nothing depends on the statistics.
4. **The rescan:**
   - the `in_root` guard (item 29), a one-line change worth up to 11%;
   - then the `seen` UPDATE (item 16): one statement per batch, or no FK on `last_scan`;
   - for cold rescans, parallel or overlapped listing (items 26, 27): up to 28% of a cold rescan, and a larger change,
     bound by the W2 rules.
5. **Not worth doing as they stand:**
   - set-based inserts (item 3);
   - LTO (−0.7%) and `SQLITE_OMIT_*` (nothing);
   - a plain UPDATE for changed files.
6. **Index count** (item 5) is the largest remaining write cost after the fix, but every index has a reader except
   `files_changed`. That one is a product decision.

## Artifacts

All under `target/scan-probe/` unless noted. Nothing here is tracked.

- **Profiles:**
  - `exp_base.out`, `exp_memstat.out`, `exp_notrigger.out`, `exp_only_write_batch.out`, `exp_only_prepare.out`, `first.out`
  - `sj/*.out` (variants)
  - `app/*.out` (the shipped and patched runs; a rescan's last part is the file without a number)
  - `build/*.out` (build options)
- **Windows results:** `win/`:
  - warm runs: `variants-memstat1.tsv`, `variants-memstat0.tsv`, `scan.tsv`, `list-small.txt`, `list-large.txt`;
  - flushed-warm and cold runs: `cpu.tsv` and `cold.tsv`;
  - the discarded runs: `cpu-invalid.tsv`, `cold-invalid.tsv` (see **Corrections**).
- **Scripts:** `stmt_journal.sh`, `app_level.sh`, `build_variants.sh`, `fork_runs.sh`, `attribution.sh`,
  `win_timing.ps1`, `win_cpu.ps1`, `win_cold.ps1`, `purge.ps1` (elevated only), `gen_large.py`, `cgx.py`, `recheck.py`
  to `recheck6.py`, and the earlier `cg.py`, `callers.py`, `selfsum2.py`, `upsert_dup.py`.
- **Copies:** `tree/` (the patched crates), `fork/` (the rusqlite fork with the flag hook), `fork-Cargo.toml`,
  `fork-Cargo.lock`, `bin/` (the Windows binaries).
- **Libraries:** `lib/` (4,000 files) and `large/` (200,000 files, about 0.3 GB on disk).
- **Probes in the tree (untracked):** `crates/library/examples/scan_probe.rs`, `crates/catalog/examples/stmt_journal_probe.rs`,
  `crates/platform/examples/list_probe.rs`.
- **Docker volumes:** `pg-scan-large` (the large library), and `pg-cargo-target`'s `patched`, `lto` and `fork` target
  dirs.
