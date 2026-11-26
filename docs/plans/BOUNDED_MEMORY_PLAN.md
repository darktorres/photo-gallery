# Bounded-memory catalog at 1B files: plan

This plan makes every library-wide pass, the grid and the Qt edge run in RAM that does not grow with the library. Results
stay identical (pairs, grid order, filters, counts) except where a deviation is stated. It covers SQLite and the schema,
the scanner and pairing, the work lists, write-back, thumbnails, borders, PDQ pairing, the Duplicates filter, the grid,
the folder tree and the Qt edge, and the verification harness.

## Status (2026-11-25)

**Not started.** No phase is built. The plan was written on 2026-11-07, and the code has moved since: about 620 commits,
including the removal of the command line, one lane per physical drive, a rebuilt pairing pass, a decode-free import, and
the removal of every memory cap. This version re-bases every step on the code at `14856c5f` plus the working tree of
2026-11-25. The 2026-11-07 text is in git: `git show 14856c5f:docs/plans/BOUNDED_MEMORY_PLAN.md`.

How this version treats the old text:

- **Step and decision ids are kept,** because other plans cite them (D9, D27, D29, D35, 7.9 and others). A step that is
  moot keeps its number, with one line saying why and what replaced it. Section 9 lists everything dropped.
- **A design that shipped differently is rewritten onto what shipped.** The plan does not bring back a design the code
  replaced for measured reasons (for example the per-scan seen set, the pairing queue and the thumbnail checkpoints).
- **Line numbers are cited as of 2026-11-25 and will drift.** Find code by symbol.

**Built since, by other plans (counted as done here):**

- The import stores header facts only; hashes and PDQ are the Hashes pass, and pairs the Pairing pass (`pg_import::hashes`,
  `pg_import::pairs`). The import-time PDQ session (`pdq_hashes`) is gone (D13).
- Missing-file writes are per row in `mark_missing` and in the removal's hide step (D10, partly).
- `recent_errors` returns the latest errors first (`crates/catalog/src/queue.rs`, part of 4.3).
- Ratings, turns, border marks and settings are written on a "window writes" thread, not the GUI thread
  (`crates/desktop/src/worker.rs`, part of D33 and 7.15).
- `files.changed` and `files.seq` with their indexes and stamping triggers exist (`docs/plans/DECODE_ONCE_PLAN.md` Phase 4,
  the first form of 7.1).
- Indeterminate progress: a total of 0 means unknown, and the lane row draws it (Q10).

## 1. Context, rules and decisions

### 1.1 Problem

At 1B files almost every library-wide pass still holds a working set that grows with the library. The holders found on
2026-11-25, with their cost at 1B (bytes per file are rough):

- **Work lists.** `Catalog::pending_import`, `pending_borders`, `pending_hashes` and `pending_placeholders`
  (`crates/catalog/src/work.rs`) count, then read the whole list into a `WorkList` (`crates/model/src/path.rs`): about
  90-120 B per due file, plus SQLite's sorter for the `ORDER BY volume, file_ref`. The placeholders list is due for every
  present file after a first import. About 100 GB per list.
- **The pipeline's input** is a slice (`pipeline::Pass { items: &[T] }`, `crates/jobs/src/pipeline.rs`), so a pass cannot
  take a paged list without an API change (1.8).
- **The thumbnail work list** is merged in a stream, then collected into `kept` and sorted in RAM
  (`crates/thumbs/src/pending.rs`): about 50-70 B per owed file. Every file is owed on a first pass, or after an eviction
  resets a height's checkpoints.
- **PDQ pairing** loads every picture's hashes and four id sets (`Catalog::pairing_work`, `crates/catalog/src/work.rs`),
  then builds the join's store (`pg_dedup::join::Joiner`): about 270 B per picture at the peak. The join also has a hard
  cap: `MAX_PICTURES = u32::MAX >> 4`, about 268M pictures (`crates/dedup/src/join.rs`). A 1B library fails with
  `TooManyPictures`.
- **The walk** keeps a `BTreeSet<FileIdentity>` of every identity it met (`crates/scan/src/lib.rs`, in the walk loop):
  25-35 B per file of the root. It also holds a whole folder's listing and rows at once, and each pending subfolder
  twice (the walk's stack and the workers' queue).
- **The seen set** is `temp.scan_seen(id PK, unpaired)` with a partial index (`crates/catalog/src/roots.rs`). SQLite's temp
  is in RAM (`temp_store = MEMORY`, `SETTINGS` in `crates/catalog/src/schema.rs`), so this costs 20-30 B per seen file in
  RAM too.
- **mark_missing** reads every folder of the root into a `Vec`, and the unseen rows into a `Vec<(i64, i64)>`, inside one
  snapshot (`crates/catalog/src/roots.rs`).
- **Removal** reads every present id under the root into a `Vec<i64>` (`Catalog::hide_pages`,
  `crates/catalog/src/removal.rs`): 8 GB at 1B. It then binds the root's whole folder list on every delete page.
- **The grid.** `list_grid` (`crates/catalog/src/browse/read/grid.rs`) builds a `Vec<Entry>` at 56 B per file. A sorted view
  also builds `(Value, Entry)` pairs and sorts them in RAM. The client (`crates/client/src/grid.rs`) keeps `entries`, a
  second copy in `held` while the viewer is open, and the row list. That is 60-130 GB on each side. Every import or scan
  reloads the list in full.
- **Filters and facets** scan the scope on every edit. The facets' `GROUP BY` sorts every row of the scope.
- **The folder tree** is held whole in the catalog's result, in the client and in the Qt model.
- **The version recomputes** each run as one transaction inside `prepare`: import, borders, taken zone, pairing, key
  algorithm, thumbnails, and the held-extensions sweep (`crates/catalog/src/lifecycle.rs`). The key recompute also loads
  every folder into RAM. `prepare` runs on the GUI thread, before any window shows.
- **The WAL** has no ceiling within a round: `wal_autocheckpoint = 0`, and the thread that wrote runs a PASSIVE checkpoint
  only at the end of its work.
- **The write-back queue** costs O(Q) per `next_ready`, plus a `count(*)` per job (O(Q²) per drain). Offline files go into
  an in-RAM `blocked` list that is bound as `NOT IN rarray`.
- **Counts.** `Catalog::status` runs 8 `count(*)` queries over the library.
- **Thumbnail cache upkeep.** The heal sums every entry in one transaction, at open, on the GUI thread. Eviction scans past
  the kept height's rows.

### 1.2 Rules

- **RAM does not grow with the library, and nothing is capped** (user decision, 2026-11-25; commit `14856c5f`). No
  structure may scale with the file count: lists are paged, sorts spill to disk, and sets live in the catalog or in
  scratch files. Working sets are fixed constants chosen for speed. There is no byte budget, no semaphore of shares, no
  reservation derived from installed RAM, no SQLite heap limit and no job-object ceiling. The whole-process figure in §2
  is an estimate for the reader, not an enforced or asserted ceiling. A pass is bounded by its count of files in flight
  (`Options::slots`), which is backpressure, not a memory cap.
- **Results are identical:** pairs, grid order, filters and counts. Where they are not, the deviation is stated in the
  step and collected in §7 (Q-D).
- **Disk and time are stated at 1B.**
- **The schema is edited in place until the first release.** `RELEASED` is empty (`crates/catalog/src/classify.rs`),
  `MIGRATIONS` is empty, and there is no back-compat (`docs/plans/HEADLESS_SESSION_PLAN.md`, decisions): a catalog made by
  an older build is carried by a rebuild or made again. The post-freeze machinery (2.3) is needed only once a release
  freezes the baseline.
- **Process.** File edits with the editing tools, validation batched at the end of the work, native Windows builds, commits
  only when asked, on the checked-out branch, no new branches. `docs/plans/CONSISTENCY_PROGRAM_PLAN.md` (its rules
  paragraph) states these in full.

### 1.3 Decisions taken by the user

On 2026-11-07, still standing:

- PDQ radius stays exactly 31.
- Batched commits are accepted.
- Exact global grid layout, provisional briefly while a list builds.
- The Q-D deviation set (§7) is accepted as a whole. Moot entries are struck there.
- Thumbnail cache-full memo: Q-T4 (a).
- Range-filter indexes are opt-in per column: Q-G1 (a), step 7.17.
- The NTFS USN-journal incremental rescan is included: Q-S1 (b), step 3.9. It is the one journal pass; the MFT plan
  keeps the full MFT read and the helper (Q-N1, decided 2026-11-25).
- **Value truth across scan sources.** Every rung treats directory-index values (size, mtime and creation time, as a
  directory listing returns them) as the truth. That covers the walk, step 3.9, the per-session MFT helper, and any
  service.
  - MFT rungs read `$INDEX_ROOT` and `$INDEX_ALLOCATION` raw (an LCN-sorted sweep with fixups), not
    `$STANDARD_INFORMATION` and `$DATA`.
  - MFT rungs filter folders by whether the client's token has FILE_LIST_DIRECTORY access, so their output equals the
    walk's.
  - `docs/plans/NTFS_MFT_SCAN_PLAN.md` was to be amended to match, and was not until 2026-11-25: Q-N1 (decided) makes
    this plan's 3.9 the one journal pass, and the MFT plan's journal-pass section points here.
- **No volume service.** The ladder is: the per-session helper (MFT read where elevation is silent), then 3.9, then the
  walk. Revisit this if the app ever ships with an installer to non-admin users.

On 2026-11-07, now moot:

- "One BASELINE schema edit, then freeze": the baseline is edited in place until the first release (§1.2), so the plan's
  schema changes land as ordinary baseline edits.

Later decisions that bind this plan:

- **2026-11-24: the command line is removed,** with its `borders --csv`, `borders --import-review` and `dups` commands. The
  developer's driver is `pg-driver` (`crates/driver`), which runs the app's own service in its own home folder
  (`docs/plans/HEADLESS_SESSION_PLAN.md`).
- **2026-11-24: no back-compat** for flags, settings, preference files or catalogs (same plan).
- **2026-11-25: no memory caps** (`docs/plans/ADAPTIVE_READERS_PLAN.md`, status). It was confirmed for this plan the same
  day: keep the size-independence goal, drop every cap (§1.2).
- **2026-11-25: leases stay as they are** (`docs/plans/BACKGROUND_JOB_AUDIT_FIXES.md`: `scan_leases` with a heartbeat; no
  `work_leases`, fence token or counter heartbeat).
- **2026-11-25: the catalog model of `docs/plans/NTFS_MFT_SCAN_PLAN.md` (its Catalog section and step 1) is adopted:**
  - **a row per link:** a `links` table holds each name of a file (folder, name, order key and the listed values), so a
    hard-linked photo shows in each of its folders; ratings and analysis stay per file. A file's own size and mtime are
    those of its present link with the greatest (mtime, size), which no read order can change;
  - **volume-relative paths:** a `volumes` table keyed by serial; folders and roots are (volume, path inside it); the
    drive letter is never stored; an unplugged volume is offline, not missing;
  - **a `folders` row for every directory,** with its identity (volume, file_ref) and `parent_id`.
- **2026-11-25: the order of the schema changes:** `docs/plans/CATALOG_SIZE_PLAN.md` Phase 2, then the MFT plan's step 1,
  then this plan's Phase 2 (Q-N3).

Every other open question takes the recommended option unless the user says otherwise (§7).

### 1.4 What changed in the code since 2026-11-07

These facts decide how each step is re-based:

- **Lanes.** The background job runs one lane per physical drive (`crates/library/src/lane.rs`; `lanes.rs` decides them).
  Each lane runs its own round over its volumes: `Scans, Import, Thumbs, Placeholders, Hashes, Pairing, Borders`, in
  `PassMask` order. Passes on different drives run at once, and CPU is shared through `CpuGate`
  (`crates/jobs/src/governor.rs`). Pairing runs on one lane at a time (`PairGate`). Removals and crops are services on
  their own threads, not passes; a removal parks the lanes of its folder's volumes (`crates/library/src/coordinator.rs`,
  `removals.rs`).
- **Seen set.** `files.last_scan` and `files_fresh` are gone (`docs/plans/SCAN_OPTIMIZATION_PLAN.md` Phase 3). A walk
  records what it saw in `temp.scan_seen`, and unseen is `not_seen!()` (`crates/catalog/src/sql.rs`).
- **Root scope.** `folders.path_key` starts with the case-folded drive or UNC share (`crates/model/src/rules.rs`), so
  `under_root!` (a `path_key` range, `crates/catalog/src/sql.rs`) is drive-exact and case-exact as the rules define.
  `files.order_key` is still drive-free.
- **Pairing.** PDQ pairs are owed through a `pairs_owed` queue, and hashes through `hashes_owed`, guarded by
  `meta.pairing_epoch` (`crates/catalog/src/baseline.sql`). The pass reloads every stored picture and joins each slice of
  200,000 owed pictures against them in RAM (`crates/import/src/pairs.rs`, `pg_dedup::join`). An owed picture is present
  and not pending; a stored one is any picture with hashes, missing and pending included. Readers see
  `present_pdq_pairs`.
- **Removal.** Removing a folder hides its files by per-row `missing = 1` writes while its lanes are parked, then deletes
  them for good in pages (`crates/catalog/src/removal.rs`, steps A to E). `add_root` refuses a folder whose removal is
  pending (`RemovalPending`).
- **Thumbnails.** The cache keeps `checkpoints(height, volume, seq)`. A round reads the changed candidates through
  `files_seq` and merges them with the cache's entries (`crates/thumbs/src/pending.rs`;
  `docs/plans/BACKGROUND_JOB_AUDIT_FIXES.md` §10). The catalog is no longer ATTACHed on that path.
- **Placeholders** are their own pass after Thumbs (`crates/thumbs/src/placeholders.rs`), over `pending_placeholders`.
- **Schema.** One baseline file (`crates/catalog/src/baseline.sql`), `BASELINE_SCHEMA = 1`, `MIGRATIONS = &[]`.
  `prepare` decides through `classify` (`Open`, `Upgrade`, `RebuildCarry`, `MoveAsideThenCreate`, `Refuse`).
- **SQLite build.** The rusqlite fork's `photo-gallery-defaults` feature sets `SQLITE_DEFAULT_MEMSTATUS=0` and lookaside
  `1200,200` (`vendor/README.md`). `.cargo/config.toml` is gone. rusqlite 0.43 has the `hooks` feature on.
- **Connections.** Every connection uses `SETTINGS` (8 MiB cache, `temp_store = MEMORY`, `busy_timeout = 5000`,
  `wal_autocheckpoint = 0`). Prepare and the pairing pass use 16 MiB, the roots reader 2 MiB, and the grid reader raises
  its own to 256 MiB per job (`crates/app/src/reader.rs`). The thumbnail cache's writer uses 16 MiB.
- **Filesystems.** On Windows, non-NTFS roots and SMB shares are refused when added and when scanned
  (`crates/scan/src/refusal.rs`). Opaque disks are refused (`docs/plans/STORAGE_AND_CLEANUPS_PLAN.md`).
- **Two app instances** can open one library: nothing refuses a second instance. Cross-process designs therefore still
  matter, but only between app instances. `pg-driver` never opens the user's library.
- **Startup.** `App::open` runs `prepare` and the thumbnail cache's open, heal included, on the GUI thread
  (`crates/desktop/src/app.rs`). The preparation is a builder with a progress report and no cancel
  (`crates/catalog/src/progress.rs`).

### 1.5 How to read the plan

- **D1-D38** are cross-area decisions (§3).
- **§5** lists alternatives that were considered and rejected, with the reason.
- **R1-R-USN4** are residuals: costs proven unavoidable (§6).
- **Q-…** are open questions, with the recommended option first (§7). **Q-D** is the set of accepted deviations.
- **Numbered steps** (for example 2.4 or 3.9) are the implementation units (§4).
- **Status words:** *moot* means the step's goal is met another way or no longer exists; *re-based* means its goal stands
  and its design follows today's code.

## 2. Target bounds at 1B

All values are for 1B files unless noted. "NVMe" means a warm or cold NVMe catalog volume. "Today" is the state at
2026-11-25.

- **Scan walk.**
  - Today: 25-35 GB identity set, 20-30 GB in-RAM seen set, a whole folder's listing and rows, two copies of each pending
    subfolder.
  - After: a few MiB of Rust plus the connection caches. The seen set and the identity set are temp tables on disk
    (about 30-45 B per entry). Frames spill only above 16K pending subfolders, at 8-20 B per frame.
  - Time: a no-change rescan walk reads every listed entry and probes the catalog once per entry: 200-400 GB of reads,
    1-4 h on NVMe (R-SCAN). A folder with more than 16K entries is listed twice. Full walks run for: a first scan, a 3.9
    fallback, a verification walk (Q-S5), a requested Full rescan, Linux roots, and detachable volumes; where the helper
    can elevate, a first scan and a Full rescan take the MFT read instead (`docs/plans/NTFS_MFT_SCAN_PLAN.md`).
  - With the MFT plan's model (§1.3) the walk also writes a folder row for every directory (once, on a first scan) and a
    link row per name.
- **Journal rescan (NTFS, 3.9).**
  - Today: a full walk.
  - After: about 140 MiB, constant in n: 1 MiB read buffer, at most 40 MiB record groups, 24 MiB dirty set, 24 MiB intents,
    16 MiB mirror cache, plus the walk's bounds for in-place sub-walks. These are fixed working sets, not shares of a cap.
  - Disk: a folder row per directory (the MFT plan's model, 1-7 GB at 10-50M directories) plus `folder_journal` beside it
    (about 60-100 B per directory). Scratch about 60 B per journal record.
  - Time: O(volume records + dirty-folder entries + sub-walk entries + affected rows). No change: at most 0.2 s. 10K
    changes: 1-3 s. A 1M-record day: 45-100 s.
- **Scan identity pairing** (`crates/catalog/src/pair.rs`).
  - Today: reads the fresh rows from `temp.scan_seen` in 1,000-id batches, so RAM is O(changes). Exceptions: on a
    renumbered volume every row rebinds, and the replaced and deleted id lists become O(root). The `unpaired_with_data`
    count reads every unpaired row under the root on each scan.
  - After: about 1 MB, plus the spill of replaced and deleted ids (16 B per row, 3.5). A stop takes effect between pages.
- **mark_missing and removal.**
  - Today: 8 GB of ids (removal), plus O(folders) and O(unseen) vectors (mark_missing).
  - After: about 1 MB. WAL at most about 26 MiB per commit.
  - Time: removal takes 3-80 h on NVMe at 1B, while the lanes of its volumes are parked (R8). It resumes where it stopped.
- **Work lists** (import, hashes, placeholders, borders).
  - Today: 90-120 GB each.
  - After: at most a 256 KiB page plus an 8 MiB folder cache. About 1M fetches at about 1 ms each.
- **Version recomputes** (import, borders, taken zone, pairing, key algorithm, thumbnails, held extensions).
  - Today: one transaction each inside `prepare`, on the GUI thread: 200-300 GB of WAL for the import bump, and hours
    with no window. The key recompute holds every folder in RAM.
  - After: `prepare` is O(1) for these. Pass A runs 1-2 h in the background, and reads stay exact throughout (D20).
- **Status, writer counts, pass totals.**
  - Today: `count(*)` over the library: 30-90 min, longer with the pairs term.
  - After: one `counts` row, about 1 ms per call. A writer drain is O(Q).
- **Write-back queue.**
  - Today: O(Q²) per drain, whole-file reads (RAM O(file size)).
  - After: `next_ready` is O(10 log Q). Offline: at most 16 root probes plus 16 folder probes per 30 s, off the drain thread.
    Streaming copy and hash with 1 MiB buffers.
- **PDQ pairing.**
  - Today: about 270 GB at the peak, and a hard failure above 268M pictures.
  - After: O(slice): the owed slice plus a fixed read window over an on-disk store. The store's disk size and the time
    figures come from the Phase 5 design round (5.0). The 2026-11-07 figures (2.1 TB of sheets, about 26 core-days for a
    first import, pair lag at most 70 min) were for a layout that is not the shipped join's.
- **Duplicates filter.**
  - Today: EXISTS probes, O(hash run or pair degree) per row.
  - After: O(log n) per row through `hash_runs` and `pdq_near` (D27). Disk 2-20 GB.
- **Thumbnail pass.**
  - Today: 50-70 GB owed list on a first pass or after an eviction; the incremental merge reads entries across every
    height.
  - After: a page of candidates plus a page of cache probes, and an ExtSort spill when an incremental round's owed set
    passes one page.
  - Time: a first pass per height reads N rows (14-28 h at 1B), and resumes after any number of stops.
- **Thumbnail cache upkeep.**
  - Today: heal O(entries) on the GUI thread; eviction scans past the kept height.
  - After: O(heights) at open. Heal runs in 65,536-id chunks on the writer. Eviction costs O(H log E + victims).
- **Grid list, sorted list, refresh.**
  - Today: 56 GB in the reader plus 60-120 GB in the client; a sorted view peaks at 90-130 GB; every refresh reloads.
  - After: about 11 MiB in the client, plus a window of at most 9.6 MB worst case. A build is a fixed working set (about
    330 MiB transient), and the run lives on disk: 80 GB in name order, 96 GB sorted.
  - Time: a cold build takes 20-30 min warm (hours cold) in the background, with a live top-K preview. A refresh costs
    O(changes × log n).
- **Name filter.**
  - Today: a scan of the scope per edit.
  - After: `names.sqlite`, 30-80 GB. O(candidates + lag + sort) for every needle length.
- **Range-filter indexes** (opt-in, 7.17): 20-30 GB per opted-in column, built online in 1-2 h per column.
- **Folder tree.**
  - Today: O(folders) three times (catalog result, client, Qt model).
  - After: under 4 MiB plus about 175 KiB of slots. 1.5 GB of per-generation files at 10M folders. Build 10-30 s.
- **Facets.**
  - Today: a `GROUP BY` sorter over the scope.
  - After: under 70 MiB. 12 GB contribution log, transient.
- **Crops.**
  - Today: the id list comes from the client's entries (8 B per bordered file).
  - After: under 2 MiB, streamed from the catalog.
- **In-app folder picker** (non-Windows only).
  - Today: `pg_platform::dir::folders_in` lists the whole folder on the GUI thread.
  - After: at most 4 MiB in RAM, then a spill-sorted file on a worker. Off the GUI thread.
- **Qt/QML edge.**
  - Today: i32 rows; an absolute `contentY` that float32 misplaces past about 645K rows.
  - After: slots at most floor((viewH + 3000) / min_pitch) + 4. Per frame O(slots).
- **Catalog rebuild** (mismatch or corruption).
  - Today: one transaction, WAL about the DB size, about 3× disk, days of random index inserts, before any window shows.
  - After: about 274 MiB plus the largest index / 32768 for SQLite, O(1) Rust. Disk: kept + building + sorter, at most
    2 × the largest index (R1). 6-12 h on NVMe, resumable, behind a stoppable progress window.
- **Startup.**
  - Today: `prepare` and the cache heal on the GUI thread.
  - After: a normal start is unchanged. A rebuild or a long step shows a stoppable progress window.
- **SQLite temp and WAL.**
  - Today: temp in RAM; no WAL ceiling within a round.
  - After: temp on disk. WAL at most a 512 MiB ceiling plus one batch (at most 26 MiB). No read transaction exceeds 2 s
    (D31).
- **Whole process (an estimate, not a ceiling).**
  - Connection caches: 8 MiB per open connection, 16 MiB for prepare and pairing, 256 MiB for the grid reader while it
    runs, 16 MiB for the cache writer: under 0.5 GiB at a typical connection count.
  - Per lane: its pass's files in flight (`Options::slots`, about the slot count × the file size), plus the pass's fixed
    working set (largest: the journal pass's 140 MiB, or the PDQ slice).
  - Plus the grid build's transient, facets, the client window and tree, and the picker: about 0.45 GiB.
  - All of it is constant in n. Phase 8 records the measured peak beside the estimate and asserts only that it is flat
    between N and 10N.

## 3. Cross-area decisions

**D1. Schema route. Re-based.**

- Until the first release, every schema change in this plan is an edit to `crates/catalog/src/baseline.sql`. A catalog
  of another layout takes `classify`'s `RebuildCarry`, or is made again; there is no migration (§1.2).
- After the release, `MIGRATIONS` gains `Step` kinds for large tables: `RebuildTable` (exclusive), `Shadow` (an online
  access path) and `Upgrade` (an online recompute or recount), beside today's `Sql` and `Rust`. A guard test refuses
  `CREATE INDEX` on `LARGE_TABLES` in migrations (2.3).
- Why: the 2026-11-07 reason (one coordinated edit, then a freeze, because three areas each claimed `MIGRATIONS[0]`)
  is gone with the migration chain. `files.last_scan`, whose FK forced a 12-step rebuild, no longer exists.

**D2. Temp policy. Not done.** `SQLITE_TEMP_STORE=0` in the fork's `photo-gallery-defaults` feature, and
`temp_store = MEMORY` removed from `SETTINGS` and from the thumbnail cache's legacy read
(`crates/thumbcache/src/reader.rs`). Why: temp data must not occupy RAM; today that includes the scan's seen set.

**D3. Connections. Re-based.** Every connection says what it is for (a `Role`), so the harness can label its census and
the authorizer can allow its statements. Caches stay fixed per role at today's values (§1.4). A role is not a cap, and
caches are not derived from RAM. Roles: Keeper, Pool, Grid (the 256 MiB reader), Inbox, Lane (one per drive),
Coordinator, Pairing, Prepare, Rebuild phases, Upgrade, Removal, RootsReader, ThumbWriter, ThumbReader, Names
(`names.sqlite`, 7.16), Watch (the USN watch's read-only reader, 3.9).

**D4. Connection home. Re-based.** `pg_schema` does not exist. Connection setup stays in `pg_catalog::core` (`open`,
`read_only`, `register`) and the cache's own in `pg_thumbcache`. The parts that need FFI the safe rusqlite API lacks
(`sqlite3_db_cacheflush`, the CACHE_WRITE status, `sqlite3_temp_directory`) go in `pg_platform::sqlite`, beside
`openable_folder` and `attach_bytes`. `pg-catalog` and `pg-dbfile` forbid unsafe code, so they cannot host them. The WAL
hook comes through rusqlite's `hooks` feature.

**D5. Work lists. Re-based.** Keyset cursors for import, hashes, placeholders and borders (4.1). The thumbnail pass keeps
its checkpoints and pages its rounds (4.6). There is no retry list: Retry marks rows pending
(`crates/catalog/src/ledger.rs`).

**D6. Cursor snapshot semantics.** A cursor yields **the subset of today's start snapshot that still qualifies when
visited and has not been re-stamped** (`seq ≤ S0`). Skipped rows go to the next pass, and `lower_total` keeps
`done == total`. This is a stated deviation (§7 Q-D). Exactness would need either an O(n) start materialization or a
snapshot held for the whole pass; both were rejected (§5).

**D7. `analysis_removed`. Re-based.** Today `analysis_removed` sets `pending = 1`, and `analysis_removed_stamp` stamps
`changed` (`crates/catalog/src/baseline.sql`). With `borders_due` on files (2.1), the one removal body also clears
`files.borders_due` and stamps `seq`. One definition for every removal path.

**D8. `pending` stamping. Re-based.** `pending` becomes 0, 1 or 2, where 2 is "stale since an Upgrade Pass A" (2.5).
Writers set `pending` only. Today `files_pending_changed` recomputes it, and `files_source_changed` stamps `seq` and
`changed` on size, mtime, turns or missing changes. A new trigger, `files_pending_stamp` (AFTER UPDATE OF pending, WHEN
old = 0 AND new ≠ 0), stamps `changed` and `seq`. Upgrade Pass A suppresses only that stamp, through an in-transaction
`counts.bulk` flag. Every CASE test reads `pending <> 0`. The 2026-11-07 `pend_batch` duties are moot (D25).

**D9. Root scope. Moot.** `folders.path_key` includes the drive, so `under_root!` is a drive-exact range. Scan pairing
reads its fresh rows from the scan's own seen set, so it never needs a key range over files. There are no anchors, no
`unpaired_anchor` and no `files_unpaired_*` indexes. What remains: `folders.match_key` and `files.order_key` are
drive-free, so any reader that ranges over `order_key` to mean "under a root" must add the `under_root!` folder test.
`file_by_path` already does (`crates/catalog/src/lookup.rs`). After the MFT plan's step 1 (§1.3) a root is
(volume, path inside it), so `under_root!` becomes a volume plus a key range, exact by volume instead of by drive letter,
and `order_key` lives on `links`.

**D10. Missing writes. Partly done.** Per-row writes in `mark_missing` (`crates/catalog/src/roots.rs`) and in the
removal's hide step. One folder-wide write remains: `mark_unheld_missing` (`crates/catalog/src/lifecycle.rs`,
`WHERE ext = ?`), which becomes a paged per-row Upgrade step (2.5). An authorizer allowlist under the `instrumented`
feature enforces the rule (3.8). Why: a folder-wide UPDATE of `missing` fans out through every trigger in one
transaction. With links (§1.3), a sweep deletes unseen links per row, then flips `files.missing` per row for the files
left with no present link; both kinds of statement are on the allowlist.

**D11. Dups derivation. Re-based.** The identity counters derive from `hash_runs`, and all counters live in the `counts`
row. Group tables are deferred with M3 (Q-N2).

**D12. Facets.** Chunked, with a contribution log.

**D13. Import flush. Done, by another route.** The import stores header facts only, and the Hashes and Pairing passes do
the rest. There is no in-RAM hash session.

**D14. Spill. Re-based.** A new crate, `pg-spill`: ExtSort, SpillQueue and SpillStack (mark and truncate), PagedFile, the
name codec, ReorderSpill and ScratchRoot. Out of disk space is a typed error (Q4). There is **no RAM budget:** each user
takes a fixed sort buffer (16 MiB by default, sized for speed), and nothing shares out or waits on memory. ScratchRoot
keeps a **disk** reservation ledger, because disk is the resource that can run out. Why: the 2026-11-07 interactive and
background RAM shares are a memory cap (§1.2).

**D15. Guard toolkit.** One toolkit in `pg-catalog`'s `fixture` feature (`crates/catalog/src/fixture/plan.rs`). The order
is fixed: the toolkit (1.3) comes before keyset (1.5).

**D16. Leases. Moot, replaced.** Leases stay as `docs/plans/BACKGROUND_JOB_AUDIT_FIXES.md` left them: `scan_leases(id,
root, holder, heartbeat)`, a wall-clock heartbeat, expiry at `SCAN_LEASE_SECS`, and `HeldLease::hold` as the first
statement of each guarded transaction (`crates/catalog/src/lease.rs`). A step that needs exclusivity between two app
instances (Upgrade 2.5, the tier-1 writer in Phase 5) adds a kind to that table, with the same rules. There is no
general `work_leases`, fence token or counter heartbeat.

**D17. Schedule. Re-based.** The lane round stays: `Scans, Import, Thumbs, Placeholders, Hashes, Pairing, Borders`, per
lane. The 2026-11-07 problem (an unsliced removal ahead of Scans starving requested scans) is gone: removals are a
service that parks only its own volumes' lanes, and a request preempts only its own lane. Added:

- **Upgrade** is a catalog-wide service thread in the coordinator, like removals. The passes it gates return
  `UpgradePending`, and the lane skips them until the step lands (D32).
- **Names** (7.16) and **tier-1 upkeep** (Phase 5) are catalog-wide, so they run behind a gate like `PairGate`: one lane
  at a time.
- A pass that returns `Busy{until}` is skipped, and the lane wakes itself at the earliest deadline (it already does for
  `BUSY_RETRY` and the failure backoff).

**D18. WAL. Re-based.** `journal_size_limit` 64 MiB. WriteBatch commits at a frame budget F = 16 MiB / page size (through
`sqlite3_db_cacheflush` and CACHE_WRITE). When the live WAL (`pnLog` of a PASSIVE checkpoint) is above
WAL_CEILING = 512 MiB, it runs a PASSIVE checkpoint, then RESTART with a 50 ms busy budget, then sleeps 100 ms
(stoppable). Today `wal_autocheckpoint = 0`, and checkpoints run at the end of a writer's work and when a session save
finds the WAL over its size (`Catalog::checkpoint_passive`, `checkpoint_if_wal_over`). Those stay; the batch ceiling adds
the bound inside a long write.

**D19. Open API.** `Catalog::open(path, rules, Role)` everywhere (about 15 production call sites). New errors:
`Rebuilding`, `Migrating`, `UpgradePending{steps}`, `RebuildBlocked`, `KeptChanged`, `NoSpace`, `OutOfSpace`.

**D20. Version recomputes.** They become Upgrade steps. `prepare` writes the new mark and step keys in O(1). Pass A
rewrites the flags in id order with no index writes (tri-state flags). Every reader goes through cursor-relative
`pending_now!` and `taken_now!`, so it is exact. Fixed seeded step keys. The steps: import, borders, taken zone, pairing
(`owe_pairs_again`), key algorithm (`recompute_keys`), thumbnails, and held extensions (`mark_unheld_missing`).

**D21. Temp dir.** `<catalog>.scratch/sqlite` (one ScratchRoot name), created first. On Windows it is aliased with
`openable_folder` (in the data folder's `sqlite-aliases`, which `Layout` names); on Unix the path bytes go to
`sqlite3_temp_directory`. Why: Windows temp paths need the aliased form.

**D22. Counters.** One seeded `counts` row: status, work totals, write queue, PDQ bands, identity. Each family has its own
recount cursor, `2^62` when idle. Present pairs = Σ bands. A definition change is an Upgrade recount step.

**D23. Rebuild protocol.** Move aside at Start into rollback-journal mode (refused if the file is in use). Manifest plus
owner heartbeat. Chunked copy with FKs off. Indexes from exact text. Ordered FK merge. Derived state from sequential
merges and ExtSort. Classified errors with corruption salvage. Swap by file identity. The `Rebuilt` notice stays as today
(posted once by `App::open`), and the manifest decides it. Why (§5): the change counter protects the kept file.

**D24. Uniqueness.** Every UNIQUE or PK constraint on a large rowid table becomes a named unique index, and every FK child
column is indexed, both enforced by lints. Today's inline constraints: `files UNIQUE(volume, file_ref)`,
`folders.path_key`, `roots.path`, `root_removals.path`, `file_failures(file_id, pass)`, `notices.key`,
`folder_failures.path_key`, `file_writes PK(file_id, kind)`. Only the large tables need it (`files`, `folders`,
`file_failures`, `file_writes`). Why: the rebuild's ordered merges drop and recreate them.

**D25. PDQ pair definition. Re-based on the owed queue.** The definition is today's: each owed picture (present, not
pending) pairs with every stored picture (any picture with hashes, missing and pending included), and readers filter
through `present_pdq_pairs`. The queue makes the result independent of when the pairing runs. The 2026-11-07 session
clock (`meta.pdq_batch`, `files.pend_batch`, `formed`) is moot: it existed to reproduce an import-time index load that no
longer exists.

**D26. PDQ storage. Re-opened.** An on-disk store of the stored side replaces the in-RAM `Joiner` store (Phase 5). The
2026-11-07 forms (thin and fat sheets, the heavy core with exactly-once emission, attribute planes, HEAVY_ENTER = 512)
were designed for 8 × 32-bit chunks. The shipped join uses 16 chunks of 16 bits, with 17 probes per chunk on permuted
diagonals (`crates/dedup/src/join.rs`). The layout is decided in the Phase 5 design round (5.0). Plane P is moot (D25).

**D27. Duplicates filter.** Pure SQL. `hash_runs.present` with `present_xor`, and `pdq_near.n` over pairs, replace today's
EXISTS forms (`crates/query/src/lib.rs`). Duplicate-status flips stamp `files.changed`, so Duplicates views get deltas.
The heavy-file part (`pdq_heavy.near`) waits on 5.0.

**D28. Ordered sinks.** A non-blocking spilling reorder buffer (`Order::Index{window_rows}`). There is no byte window:
the RAM window is a row count, and rows past it spill (1.8). The pipeline carries the item through to the sink on every
path.

**D29. Qt numeric edge.** Ranks and counts as exact f64 (n < 2^48 by the SQLite file format). File ids as decimal
strings (today they cross as `f64`, `crates/desktop/src/shell.rs`). The scene graph sees only viewport-relative
px < 2^14, and the slot layer sits outside the Flickable's content item (P10). Rectangles snap to whole pixels. Every
queued rank is an `ItemRef`. Why: float32 misplacement past about 2^24 px rules out absolute positions in the scene graph.

**D30. View layer.** `WindowView.qml` (a plain Flickable plus a non-moving slot layer that is a sibling of its content
item, P10) over a Rust `SlotWindow`. The ListView behaviours the views relied on are ported from Qt 6.12 and pinned by a
golden table. `WheelListView.qml` is used today by `GalleryGrid.qml`, `FolderTree.qml` and `FolderPickerDialog.qml`.

**D31. Read snapshots.** READ_TX_MAX = 2 s for every catalog and cache read, except the rebuild's building file. Long sets
are read by converging paged reads, which give an exact snapshot at their final instant. The harness enforces a census.
Today's long snapshots: `mark_missing`'s read, `status`, the work lists and `list_grid`.

**D32. Gates.** `Catalog::gate(Need)` is checked inside the catalog API (the work lists, crops, pairing), never in
callers.

**D33. GUI writes. Partly done.** Ratings, turns, border marks and settings go through the window-writes thread
(`crates/desktop/src/worker.rs`). Still on the GUI thread: `save_view` and `delete_view` (`crates/desktop/src/shell.rs`),
`add_folder` and `remove_folder` (`crates/desktop/src/library.rs`), and the thumbnail cache's open and heal. Missing: the
commit stamp and the client overlay held until it is covered (7.15). `docs/plans/GUI_THREAD_PLAN.md` overlaps this.

**D34. Pending removals. Moot, replaced.** Removal hides its files by per-row `missing = 1` writes while its lanes are
parked, so every reader already excludes them through `missing = 0`. There is no reader mask, and no as-of counts. What
remains is the RAM of the hide and delete steps (3.4).

**D35. Thumbnail pass state. Re-based on the checkpoints.** The state is today's `checkpoints(height, volume, seq)` plus
the eviction count in `checked`. The 2026-11-07 per-height laps, `todo`, `lost` and walk cursor are not built: the
checkpoint already makes an incremental round O(changes). What 4.6 adds is paging, so the first pass and a reset height
are bounded too. A cache layout change bumps `CACHE_VERSION` and starts the cache over, as its rule says
(`crates/model/src/versions.rs`).

**D36. Startup.** `prepare` is O(1) except for a rebuild. A rebuild runs on a worker behind a stoppable progress window.
The cache's heal leaves the open (4.5).

**D37. USN journal rescan. Re-based; the one journal pass (Q-N1).** State-based: the journal only names dirty folders.
Listings through the walk's own `visit_folder` are the truth. Per-root cursor (seeded by an MFT read), per-directory state
in a side table of `folders` (the 2026-11-07 `usn_dirs` mirror, now that every directory has a folder row), census,
ordered replay through the same writer, staleness and access rules, the own-write bracket, unprivileged read gated by a
self-test. With a row per link, there is no winner link to choose (3.9). Details in 3.9 and §5.

**D38. Seen floor. Re-based on the seen set.** A walk's unseen set is `not_seen!()` over the root, as today. A journal
pass's unseen set is the rows of its scopes (relisted folders, sub-walked subtrees and dropped refs) that it did not
claim. Its claims go into the same seen sets, so `mark_missing` runs the same statement over the scopes instead of the
whole root. After the MFT plan's step 1 the sets are two, keyed by scan: `temp.scan_seen_links(scan, id)` and
`temp.scan_seen_files(scan, id, unpaired)`; the sweep deletes unseen links, then marks missing the files left with none
(D10). There is no `file_seen`, `scans.floor` or sentinel scan. Why: SCAN_OPTIMIZATION Phase 3 measured the per-row
seen stamp as most of a rescan's cost and removed it.

## 4. Phases

**Commit policy.** One commit per phase when a commit is asked for (§1.2). After all of them, one
`just build && just lint && just test`, then fix forward. Forward dependencies are marked with `→`.

**Oracles.** Every equivalence oracle is the old code or SQL kept under `cfg(test)` or the `fixture` feature.

**Order across plans.** `docs/plans/CATALOG_SIZE_PLAN.md` (the `mfolders` table and the `path_name_key` expression index)
also rewrites `files`. With in-place baseline edits the two no longer need one coordinated migration (its D5). The
catalog-size plan lands first (Q-N3, decided 2026-11-25), and its key replaces `order_key` in 3.6, 4.1 and 7.4. Then
`docs/plans/NTFS_MFT_SCAN_PLAN.md` step 1 (§1.3: links, volumes, volume-relative paths, a folder row per directory), then
this plan's Phase 2. So this plan's statements are written against the catalog that step leaves:

- what names a place (folder, name, order key, the listed values) is on `links`; identity, `missing`, `pending`, `seq`,
  `changed`, `borders_due` and the per-file data stay on `files`;
- the grid, the tree, facets, the name filter and crops read links (one entry per link); work lists, thumbnails, PDQ,
  the Duplicates filter and the counters read files;
- a root is (volume, path inside it), and `under_root!` a volume plus a key range;
- the scan's seen sets are the two of D38.

The steps below name today's tables where they describe today's code, and that catalog where they describe the change.

### Phase 1: Shared foundations (no behaviour change)

**1.1 Temp storage on disk.**

- **Files:** the fork's `libsqlite3-sys/build.rs` (`photo-gallery-defaults`), `vendor/README.md`,
  `crates/catalog/src/schema.rs` (`SETTINGS`), `crates/catalog/src/core.rs` (the compile-option test),
  `crates/thumbcache/src/reader.rs`, `docs/plans/DATABASE.md`.
- **Change:**
  - `SQLITE_TEMP_STORE=0` beside `SQLITE_DEFAULT_MEMSTATUS=0` and the lookaside in the fork's feature, pinned by the same
    test.
  - `SETTINGS` drops `temp_store = MEMORY` and gains `journal_size_limit = 67108864`.
  - Every production ORDER BY is total.
  - Measure the first-scan and rescan benches before and after (`crates/bench`). `temp.scan_seen` moves from RAM to a temp
    file, which is mostly page cache, but SCAN_OPTIMIZATION's measurements assumed RAM.
- **Tests:** the compile option; a 400k-row sort raises the heap highwater by less than 8 MiB; a 200k-row trigger UPDATE
  journals on disk; a lint (`no_memory_temp`) refuses `temp_store = MEMORY`.

**1.2 Connection setup, roles, hooks, URIs.**

- **Files:** `crates/catalog/src/core.rs`, `crates/catalog/src/types.rs`, `crates/platform/src/sqlite.rs`,
  `crates/thumbcache/src/schema.rs`, every `Catalog::open` caller (D19), `crates/app/src/catalogs.rs`.
- **Change:**
  - `open(path, rules, Role)` sets `SETTINGS` and the role's cache.
  - One composed progress handler: today's `cancel_when` plus, under `instrumented`, a step counter.
  - Under `instrumented`: the authorizer (an SQLITE_UPDATE on `files.missing` is denied unless the statement's tag is
    allowlisted, 3.8) and read-transaction span recording (D31).
  - A WAL hook that feeds the WAL high-water sampler and WriteBatch (D18).
  - **attach_uri.** An ATTACH of the catalog uses a percent-encoded `file:` URI with `mode=ro` or `mode=rw`, never CREATE.
    Today's only callers are the rebuild's carried source and the thumbnail cache's legacy `pending_for`, which 4.6
    deletes.
- **Tests:** per-role cache size; cancel still interrupts while steps count; an `attach_uri` to an absent path creates
  nothing; the authorizer denies a crafted folder-wide `missing` UPDATE.

**1.3 Guard toolkit** (before keyset).

- **Files:** `crates/catalog/src/fixture/plan.rs` (new), and the existing ad-hoc EQP tests converted (for example
  `the_next_job_comes_off_the_queue_index`, `crates/catalog/src/queue.rs`).
- **Change:** `Plan::of` (EQP plus an opcode census, subprograms included); `assert_streaming`, `assert_temps`,
  `assert_bounded(allow)`, `assert_write_fanout`; `steps_of`; `with_billion_stats`. Every guard runs with no stats, and
  again with 1B-shaped stats.
- **Tests:** the toolkit's own, on crafted statements.

**1.4 Write discipline.**

- **Files:** `crates/catalog/src/batch.rs` (new), `crates/catalog/src/transaction.rs`.
- **Change:**
  - **Gate:** a FIFO ticket per catalog path, so lanes, the coordinator and services take the write lock in arrival
    order. Today's `Patience` loop retries BEGIN IMMEDIATE every 50 ms, which is not FIFO.
  - **WriteBatch** opens lazily and commits at `rows`, at 250 ms, or when the transaction's frames reach F. Every 64
    driving rows it calls `sqlite3_db_cacheflush` and reads the CACHE_WRITE delta, which over-counts frames, so commits come
    early, never late. Bound: at most (F + 64·P) pages, about 26 MiB, with P (pages per row) pinned in 3.8. It replaces the
    fixed `WRITE_BATCH` counts (1,000 and 500).
  - **Before each batch:** the WAL ceiling of D18. Foreground write transactions are exempt.
  - The lock yields for 2 ms after being held for 1 s.
- **Tests:** gate FIFO under 8 threads; commits at rows, age and frames; `set_rating` succeeds within 150 ms during a batch
  loop and while a reader holds a 5 s snapshot; a second app instance grows the WAL past the ceiling and this one waits.

**1.5 Keyset and subtree primitives.**

- **Files:** `crates/catalog/src/keyset.rs` (new), `crates/catalog/src/subtree.rs` (new), `crates/catalog/src/sql.rs`.
- **Change:**
  - `KeySeek`: NULL-aware multi-key seeks, plus a single-key form. Rule: exactly one bounded lower limit per indexed
    column.
  - `FolderPages`: the root's folders in `path_key` order, paged, through today's `under_root!` range. It replaces the
    `Vec` of folder ids (`folders_under`, `folder_ids_under`).
  - `SkipMerge` over the folders a scan could not list (today the skipped set of `mark_missing` and `folder_failures`),
    with `reseed(cursor_path)`, O(depth) per reseed.
  - The `paged(read_page, apply_write)` driver.
- **Tests:** drains equal `ORDER BY` at page sizes 1, 2, 7 and 1024; SkipMerge equals the in-RAM skip set, including
  sibling, nested, UNC and drive cases, and after a reseed; no TEMP B-TREE.

**1.6 `pg-spill` and platform primitives.**

- **Files:** `crates/spill/src/{lib, extsort, spill, paged, codec, reorder, scratch}.rs` (new crate);
  `crates/platform/src/{file.rs (Staged, read_at and write_at), volume.rs (mount_root, detachable),
  process.rs (peak_private_bytes)}`; `crates/platform/src/usn.rs` and `token.rs` (new; the MFT plan's `crates/volume/src/usn.rs` becomes this, Q-N1);
  `crates/platform/src/dir.rs` (`probe_entry`, `first_call`).
- **Change:**
  - **ExtSort:** stable, fan-in 64, `merge_reduce`, a fixed sort buffer per user (D14).
  - **SpillQueue and SpillStack** (`mark` and `truncate_to` in O(1)).
  - **PagedFile.**
  - **Codec:** 64 KiB blocks that decode independently. A frame record holds the name in WTF-8 or UTF-16LE (the shorter),
    front-coded against the previous name, a zigzag varint id delta, and the volume only when it differs. Worst case
    27 + 2·units B.
  - **ReorderSpill:** 64 MiB segments with live counts, plus a PagedFile offset table of 12 B per index.
  - **ScratchRoot** `<catalog>.scratch/`: a disk reservation ledger, checked against free space
    (`pg_platform::volume::space`) before any work, with priorities interactive > pass > maintenance. Every owner is named
    (SQLite temp, scan frames, journal runs, reorder, picker, tier-1, grid runs, tree generations, facet log). Startup
    sweeps owners that are not live.
  - **Staged** (write, fsync, rename).
  - **mount_root:** lexical: a drive, `\\server\share\`, or the library root on POSIX.
  - **detachable():** builds on the bus type and seek-penalty read in `pg_platform::drive` (USB, 1394, SD, MMC, Virtual,
    FileBackedVirtual), plus RemovableMedia and IOCTL_STORAGE_GET_HOTPLUG_INFO.
  - **peak_private_bytes:** a measurement for Phase 8, not a limit.
  - **USN platform** (Windows only; every other OS returns Unsupported; behind `trait Journal` with `Listing`, so a fake
    volume can stand in). The 2026-11-07 specification stands; the helper's privileged reads (the MFT plan) go through
    the same `Journal` trait:
    - `JournalHandle::open(X:\)`: FILE_READ_ATTRIBUTES | SYNCHRONIZE, share RWD, BACKUP_SEMANTICS | OVERLAPPED.
    - `query()` → `{journal_id, first_usn, next_usn, lowest_valid_usn, max_usn, max_size, alloc_delta}`. Codes 1179 →
      NotActive, 1178 → Deleting, other → Unavailable.
    - `read(start, journal_id, end, each)`: READ_USN_JOURNAL_DATA_V1, ReasonMask !0, versions 2-3, through
      FSCTL_READ_UNPRIVILEGED_USN_JOURNAL (0x903AB); the privileged read only when elevated. Fixed 1 MiB buffer. Safe
      parser (length bounds, alignment 8, V3 high id bits 0, USN strictly increasing, else Malformed). Codes 1181 →
      EntryDeleted, 87 → BadStart.
    - `wait_read` (overlapped, cancellable), `resize` (FSCTL_CREATE_USN_JOURNAL, elevated, keeps the id),
      `open_by_id` (`Live{end_of_file, last_write, creation, attributes, number_of_links}`; codes 87, 2, 3 → Gone; 5 →
      Denied), `links()`, `dir_path()`.
    - `probe_entry(dir, name)`: a new directory handle per name, looping until an exact UTF-16 match.
    - `first_call(dir)`: `Lister::open` plus one listing call, the walk's own access check.
    - `access_projection()`: BLAKE3 over TokenUser, enabled and deny-only group SIDs except SE_GROUP_LOGON_ID groups,
      restricted SIDs, AppContainer SID and capabilities, the integrity SID, and the enabled state of SeBackup, SeRestore
      and SeChangeNotify.
    - `unpriv_selftest(dir)`: Verified, Filtered or Unsupported.
    - Collation: a 64K upcase table, `collate`, a prefix-free `collation_key`, `is_table_independent`. It shares its table
      with `CATALOG_SIZE_PLAN.md`'s `path_name_key`, which embeds a versioned table instead of the running OS's: use that
      one.
    - VolumeGuard: a message-only window with DBT_DEVTYP_HANDLE registration.
- **Tests:** ExtSort equals a stable sort; SpillStack proptest with mark and truncate; codec proptest (non-UTF-8 names,
  unpaired surrogates, CJK, random 64-bit ids, block edges; 1M NTFS-like frames average at most 20 B); ReorderSpill
  deletes empty segments; the ledger refuses an oversize reservation before work starts and never sweeps a live owner;
  the USN platform tests of 2026-11-07 (parser vectors and fuzzing, real NTFS unelevated, probe_entry cases, links, the
  self-test on build 19045, the projection across logons, detachable on a VHD, a volume lock while a wait is armed).
- **Dropped:** the segment stack (only the borders CSV used it), ConcurrentReader (the pipeline's readers are tuned per
  drive, `docs/plans/ADAPTIVE_READERS_PLAN.md`), the RAM Budget, and `run_in_job`.

**1.7 pg-model shared types.**

- **Files:** `crates/model/src/{num.rs, ids.rs, grid.rs, path.rs, pdq.rs}`.
- **Change:**
  - **FolderCache:** an LRU of folder paths (8 MiB), replacing `WorkList`'s whole `BTreeMap` of folders.
  - **Numeric contract:** `Rank(u64)` (serde-transparent), `GlobalRow`, `ListEpoch`, `ItemRef{rank, epoch, file}`;
    `LayoutY(u128)` in 2^-32 px with `from_local` and `to_local`; `MAX_CATALOG_BYTES = 2^48` (u32 page numbers, page size
    at most 65536); `rank_to_qml`, `rank_from_qml`; `FileId::to_decimal` and `parse_decimal`.
  - **PDQ:** `PairKey`, `Band`, and the band partition of 0..=31 as a const assertion. `MIN_QUALITY` and `MATCH_DISTANCE`
    exist (`crates/model/src/pdq.rs`). The heavy threshold waits on 5.0.
- **Tests:** round trips at 0, 2^31 ± 1, 2^32 + 7, 2^46 and 2^48 − 1; `rank_from_qml` rejects NaN, infinity, fractions
  and 2^53; LayoutY associativity is exact.

**1.8 pg-jobs pipeline and progress.**

- **Files:** `crates/jobs/src/{pipeline.rs, control.rs}`, `crates/model/src/progress.rs`, every pass crate's call
  (`pg_import`, `pg_thumbs`, `pg_borders`, `pg_crop`), `crates/wording/src/*`, `crates/wording/locales/en/text.ftl`.
- **Change:**
  - **A feed instead of a slice.** `run` and `batched` take a `Feed` that yields items in pages (a keyset cursor, 4.1),
    beside today's slice form for small lists. The pipeline reads the next page when its slots free up.
  - `run(...) -> Ended{Exhausted, Stopped, SinkStopped, FeedFailed}`, with an explicit shutdown order. Today `run`
    returns `Result<(), Failed>` and `batched` returns `(W, Result<(), Halt<E>>)`.
  - **The item travels with its result to the sink on every path,** including unread items and failed opens. `batched`'s
    flush already gets `Done = (&T, Result)`. A stop sinks the unstarted items as `Stopped` rather than dropping them, so
    a cursor can keep its position.
  - **`Options.order: Order::{Completion, Index{window_rows}}`.** In Index mode results inside the window go into a ring;
    the rest go to ReorderSpill, and workers never block. The head index is always admitted. A stop or Exhausted end
    flushes the held and spilled rows in index order. A sink returning false is final.
  - Control gains `lower_total` and `raise_total` (today only `begin`, `add` and `set_done`).
  - `Phase` (`pg_model::progress`) gains `Upgrade`, `Rebuild`, `ScanJournal`, `AccessReprobe`, `Names`, and the tier-1
    phases of 5.0. `Pairing` and `Removing` exist.
  - Messages: the passes and notices above, `PreparingCatalog`, `PreparingPhase`, and the journal notices
    (`JournalInactive`, `JournalWrapped`, `JournalBaseWrapped{needed}`).
- **Tests:** paged items are read once, in grant order; one 500 ms-slow head among 10^5 fast items: RAM holds at most
  `window_rows` results, the spill is deleted afterwards, sink indices strictly increase, workers stay busy; a run of
  failed opens with window 1; a stop during 10^6 empty fetches ends within one fetch; no sink call after a false;
  FeedFailed comes after the in-flight sinks.

**1.9 Lease helper. Moot** (D16). A new lease kind reuses `crates/catalog/src/lease.rs`.

### Phase 2: Schema, rebuild, upgrade and startup

**2.1 Baseline objects** (edits to `crates/catalog/src/baseline.sql`).

- **files:**
  - `borders_due INTEGER NOT NULL DEFAULT 0` (0, 1 or 2) moves here from `analysis`, with
    `files_borders_due(volume, file_ref & 0xFFFFFFFFFFFF) WHERE missing = 0 AND pending = 0 AND borders_due = 1`, in the
    form of today's `files_to_import`. `analysis.borders_due` and `analysis_borders_due` go.
  - `pending` is documented as 0, 1 or 2 (D8). `files_to_import` stays `WHERE missing = 0 AND pending = 1`.
  - `UNIQUE(volume, file_ref)` becomes the named index `files_identity` (D24).
  - `files_present_order(volume, file_ref & 0xFFFFFFFFFFFF) WHERE missing = 0`. Today's `files_present` is keyed by
    `folder_id`, and `files_order` by `order_key`, so neither serves the (volume, place) walk.
  - `files_unpaired(id) WHERE unpaired = 1`, for the `unpaired_with_data` count (3.2), joined to the root's links.
- **links** (from the MFT plan's step 1): `links_present(folder_id) WHERE` the file is present, for the tree and the
  removal's pages; `links_order` (the grid's name order, today's `files_order`).
- **folders:** `present` (a count of present links, maintained by three triggers), and `folders_shown`. The path key
  UNIQUE becomes a named index. The MFT plan's step 1 adds `volume`, `file_ref`, `parent_id` and the path inside the
  volume.
- **file_writes:** `parked_folder`; `file_writes_next` is replaced by `file_writes_ready((temp IS NULL), attempts, queued)
  WHERE attempts < 5 AND parked_folder IS NULL`, `file_writes_failed(queued, file_id, kind) WHERE error IS NOT NULL` and
  `file_writes_parked(parked_folder) WHERE parked_folder IS NOT NULL`. New tables
  `write_parked_roots(root_id PK, path UNIQUE, recheck_at, state)` and
  `write_parked_folders(folder_id PK, root_id, recheck_at)` with an index on `(root_id, recheck_at)`.
- **scans:** `kind` (0 walk, 1 journal) and `usn_state BLOB` (3.9). A pairing continuation (3.2) needs no column: it lives
  as long as the lane's connection.
- **root_removals:** `cursor` (the hide and delete steps' watermark, 3.4).
- **files:** `own_from`, `own_usn` and `content_gen`, from the MFT plan (3.9, Q-N1).
- **USN journal tables** (3.9): `usn_builds`, `usn_volumes`, `usn_roots`, `usn_entries`, `usn_stale`, `usn_wake`, and
  scratch `usn_drop` and `usn_scope`, as specified on 2026-11-07, minus every `file_seen` and floor column. Changed by
  the links model: `usn_dirs` becomes `folder_journal(folder_id PK → folders, state, error, entries, media, reparse_files,
  counted, wkey)`, a side table of the folder row each directory now has (its ref and parent are the folder's own
  columns); `usn_entries` loses kind 1 (multi-link), since links are rows; the census loses `extra_links`. Every journal
  table is NOT_CARRIED.
- **Grid:** `files_log(seq PK, id, what)` (7.1), where `id` names the link a grid entry is.
- **PDQ:** `pdq_indexed(seq PK, file_id UNIQUE)`, the store order of the on-disk index, and the tables of 5.0.
  `pdq_pairs` gains `CHECK(distance BETWEEN 0 AND 31)`. `pdq_near(file_id PK → files CASCADE, n)`.
- **Dups:** `hash_runs(kind, hash, total, present, present_xor, PK(kind, hash)) WITHOUT ROWID`. Group tables are deferred with M3 (Q-N2).
- **Shared:** the `counts` row (D22): files, present, pending, borders_due, analyzed, failed, failed_retryable, writes,
  writes_failed, writes_left, band_0 to band_3, the identity counters, the per-family recount cursors, and `bulk`.
- **Meta seeds:** `view_epoch`, `log_horizon`, `thumb_epoch`, `upgrade_<step>` per D20 step, `bulk_walk_import`,
  `bulk_walk_borders`. Every key is seeded and never added or deleted at run time.
- **Triggers:** grid (`files_removed`, `files_renamed`, `analysis_cut_changed`, `analysis_shape_changed`); D8's
  `files_pending_stamp`; `folders.present`; `pdq_indexed_removed`; dups (`hash_runs` and `pdq_near` upkeep, band
  counters); counters (about 12, each gated by its family cursor); exit triggers that turn flag 2 into 1 on a key change
  or a missing flip; the write-queue unpark trigger (`files AFTER UPDATE OF folder_id, missing`); the USN wake triggers
  (3.9).
- **Classes:** CARRIED and NOT_CARRIED cover every table (`crates/catalog/src/rebuild.rs`). Tables added since 2026-11-07
  that need a class: `pairs_owed`, `hashes_owed`, `placeholders`, `folder_failures`, `ledger_ack`, `file_ratings`,
  `border_marks` (and `scan_leases`, which is TRANSIENT). SMALL_TABLES: meta, session, views, roots, root_removals, scans,
  usn_builds, usn_volumes, usn_roots.
- **Tests:** `CARRIED ∪ NOT_CARRIED ∪ TRANSIENT == tables`; the seeded key set; lints (autoindexes only on SMALL_TABLES;
  every FK child column indexed; no upsert on analysis; no OR REPLACE on files or file_writes); each partial-index
  `INDEXED BY` text prepares.

**2.2 Catalog rebuild: chunked, resumable, crash-safe.**

- **Files:** `crates/catalog/src/rebuild.rs` (today `make_replacement`, `carry_rows`, `copy_tables`, `CARRIED`,
  `TRANSIENT`; the old `copy_tables` is kept under `cfg(test)` as the oracle), `crates/catalog/src/core.rs` (the
  preparation), `pg_dbfile` (`move_aside`), `crates/platform/src/{sqlite.rs, file.rs}`, `docs/plans/DATABASE.md`.
- **Change.** A state machine driven by `<catalog>.rebuild`, a manifest written by Staged: the build digest, the reason,
  the kept base name and the building path, the kept file's change counter and size, the built file's OS identity, and
  the phase and counts. Phases Start → Copy → Index → Verify → Derive → Finish → Swap; each step is one transaction on
  `.building` that also writes its progress row.
  1. **Detect.** In the preparation, under `pg_dbfile`'s lock: if a manifest exists, resume (a different digest restarts
     at Copy); otherwise run `classify`.
  2. **Start.** Checkpoint the old file (TRUNCATE, busy = 0). `PRAGMA journal_mode = DELETE` must return `delete`, which
     succeeds only when no other connection is open; otherwise the result is "catalog in use" (deviation h). Check free
     space (step 9). Choose the first free `.replaced[.n]` base, record it, rename the main file. A foreign file at the
     path gives `RebuildBlocked{path}`.
  3. **Connections.** Building: `journal_mode = DELETE`, `synchronous = FULL`, `locking_mode = EXCLUSIVE`, foreign keys
     off. Cache 64 MiB for Copy, Verify and Derive; 128 MiB for Index. Source:
     `ATTACH attach_uri(kept, Ro) AS carried`, 16 MiB. Kept integrity (no WAL, no hot journal, change counter and size
     unchanged) is checked at every resume, phase boundary and error, and before Swap; otherwise `KeptChanged`.
  4. **Copy.** Each CARRIED table in batches of at most 100k rows by rowid or KeySeek PK, as appends. Today's per-table
     conversions (`CARRIED`'s specs) run inline. `pdq_pairs` is filtered to `distance BETWEEN 0 AND 31`.
  5. **Index.** One saved index per transaction, from its exact text.
  6. **Verify.** For each FK child, a paged, resumable, ordered merge. An orphan is a data error.
  7. **Derive.** `folders.present`, the `counts` row, `hash_runs`, `pdq_near` and the bands, by sequential merges and
     ExtSort. Derived state is recomputed from rows, never carried (deviation d).
  8. **Errors.** Environmental (FULL, IOERR, CANTOPEN, stop, NoSpace, KeptChanged): Err, and the manifest is kept. Data
     (UNIQUE, FK, CHECK, NOT NULL): `Carried::Nothing`, as today. CORRUPT or NOTADB with an unchanged counter: retry once,
     then salvage by bisecting the table-key ranges, and report the rows dropped.
  9. **Free space.** Needed = Σ remaining outputs + 2 × the largest remaining index + 1 GiB, checked at Start, at each
     phase and at each resume.
  10. **Finish.** `locking_mode = NORMAL`; one transaction for meta, `view_epoch += 1`, `log_horizon`, the `counts` row,
      and the triggers from their saved text.
  11. **Swap.** Rename, verify identity on resume, delete the manifest. The `Rebuilt` notice is posted as today.
- **Deviations** (§7 Q-D): (a) the catalog path is absent during the build; (b) the kept file is left in rollback mode;
  (c) environmental failures are resumable, where today they give an empty catalog; (d) derived counters are recomputed;
  (e) a crash between the move and the rename no longer empties the catalog; (f) a foreign file gives RebuildBlocked;
  (g) the Rebuilt notice is shown at least once; (h) "catalog in use" is refused; (i) a data error's text can name a
  different first violation; (j) corruption is salvaged by range.
- **Tests:** the oracle at batch sizes 1, 2, 7 and 100k (full dumps, the schema set, the derived oracles); `fail_after(n)`
  at every commit, manifest write and rename; a half-done move resumes; a child-process kill resumes; an orphan or
  duplicate gives Nothing; FULL keeps the manifest; a write to kept gives KeptChanged; a corrupt page is salvaged; after
  Swap there is no journal; after a rebuild an NTFS root's next scan walks; scale: flat heaps, temp at most 2.2 × the
  largest index, no WAL.

**2.3 Post-freeze migration machinery.** Built when the first release makes `RELEASED` non-empty, or earlier if 7.17 is
built (it needs Shadow).

- **RebuildTable** (exclusive, in the preparation, behind the startup window): the first transaction sets
  `user_version = (1 << 30) | (V + 1)` so older builds refuse; batches copy into `T__new` and delete from T; then rename,
  indexes from exact text, the FK merge, views and triggers, and the flag cleared. Other processes get `Migrating`.
- **Shadow** (online, an Upgrade step): `sh_<name>` (WITHOUT ROWID) plus `sh_<name>_log`; phase R (short snapshots,
  in-RAM chunk sorts, fsynced run files), phase M (K-way merge-append through WriteBatch), phase L (replay the log; ready
  in the same transaction). `difference_allowing(names)` keeps `mismatch()` None while configured shadows exist.
- **Tests:** RebuildTable(files) equals a reference at batch sizes 1, 7 and 100k; `fail_after` at every commit; an
  older-build fixture refuses and its file is byte-unchanged; Shadow equals `SELECT key, id ... ORDER BY` under
  concurrent writes and kills; `set_rating` stays within 150 ms.

**2.4 Writers adapted to the new columns.**

- **Files:** `crates/catalog/src/writers.rs`, `crates/catalog/src/pair.rs`, `crates/catalog/src/work.rs`, the fixtures
  (`fixture/raw.rs`, `fixture/state.rs`), `tools/shots/fixture.py`.
- **Change:**
  - **AnalysisWriter:** `borders_due` goes to files. `store_failure` clears `borders_due`. A hash store assigns the
    `pdq_indexed` seq when quality is at least `MIN_QUALITY` and a dihedral exists.
  - **BordersWriter** cut and crop source move to files.
  - **ScanWriter** (plain INSERT plus guarded UPDATE by id, and displace): `folders.present` follows through its triggers.
  - **mark_pending** sets `pending = 1` only (the D8 trigger stamps).
- **Tests:** the existing suites; `each_user_visible_change_names_its_file`, extended and schema-derived, so every
  view-visible column has a stamping trigger; the dups trigger proptests.

**2.5 Upgrade: version recomputes off the preparation, exact reads.**

- **Files:** `crates/catalog/src/{lifecycle.rs, upgrade.rs (new), work.rs}`, `crates/query/src/lib.rs` (`status_sql`, the
  taken conditions and sort key), `crates/library/src/coordinator.rs` (the Upgrade service), `crates/driver`.
- **Change:**
  - **The preparation.** In one IMMEDIATE transaction, compare each version mark (import, borders, taken zone, pairing,
    key algorithm, thumbnails, held extensions, counter families). For each that differs, write the new mark, set the
    step key to its start phase, and for import bump `thumb_epoch` and reset `bulk_walk_import`. Cost O(1).
  - **The Upgrade service** runs the steps in id-keyset batches through WriteBatch, under a lease kind for exclusivity
    between app instances (D16).
  - **Import Pass A:** `UPDATE files SET pending = CASE WHEN <stale> THEN (CASE WHEN missing = 0 AND pending <> 1 THEN 2
    ELSE 1 END) ELSE 0 END ...`, with `counts.bulk = 1` inside the transaction. No index writes, no seq or changed stamps.
  - **Borders Pass A:** missing rows and rows with pending ≠ 0 get `borders_due = coalesce(has_borders, 0)`; present rows
    with pending = 0 go to 2 or 0.
  - **Taken step:** batches over the taken index; the zone fingerprint is re-read each batch.
  - **Pairing step:** `owe_pairs_again` in pages (an INSERT into `pairs_owed` per picture page).
  - **Key step:** `recompute_keys` in `path_key` pages, holding one page of folders, not all of them.
  - **Held-extensions step:** `mark_unheld_missing` per row, paged (D10).
  - **Count steps:** per-family recounts (2.6). **Shadow steps** (2.3, 7.17).
  - **Gates (D32):** `Catalog::gate` returns `UpgradePending{steps}` from the import, hashes and borders lists (import,
    borders), crops (import, borders) and pairing (import, pairing). Scans, removals, thumbnails and writes are not gated.
  - **Exact reads.** `pending_now!(f)` = `CASE WHEN f.id < ?c THEN f.pending <> 0 ELSE <stale_of!(f)> END`, and
    `taken_now!(a)` likewise. Every reader of these values uses them, and a grep guard enforces it.
  - **Driver:** `pg-driver run` waits for the steps its passes need.
- **Deviations:** ungated passes may run before the steps finish (ordering only); the status line shows upgrade progress
  instead of counts; a process time-zone change during a taken step restarts it (impossible on Windows).
- **Tests:** the oracle (today's statements) at batch sizes 1, 7 and 100k, with stops and re-bumps; keys mismatch-free
  at every stop; the preparation is O(1) on 1M rows; gates fire inside the API; exact-read proptest: status filters, the
  taken filter and sort, and the grid list equal an instant full recompute at every cursor position under concurrent
  writes.

**2.6 Counters and status.**

- **Files:** `crates/catalog/src/baseline.sql` (triggers), `crates/catalog/src/lookup.rs` (`status`),
  `crates/catalog/src/queue.rs` (`writes_remaining`, `left_to_write`), `crates/catalog/src/work.rs` (totals),
  `crates/writeback/src/lib.rs` (`drain_queue`), `crates/driver`.
- **Change:**
  - Triggers keep the `counts` row exact under INSERT, upsert, REPLACE (analysis, through BEFORE INSERT) and cascades
    (files BEFORE DELETE subtracts first). Each adjusts only when its row key is below its family's cursor.
  - `status()` reads one row instead of 8 `count(*)`.
  - `drain_queue` reads `counts.writes_left` instead of `writes_remaining()` per job.
  - Totals: borders total = `counts.borders_due`; import total = `counts.pending + counts.failed_retryable`, read with S0
    in the cursor's first transaction.
- **Tests:** a proptest against `old_status` (`cfg(test)`) after every step; pin tests for upsert and REPLACE trigger
  semantics; recount one family while others write; VM steps for `status()` are O(1); golden `pg-driver status` output.

**2.7 Startup off the GUI thread.**

- **Files:** `crates/desktop/src/{app.rs, lib.rs, preparing.rs (new)}`, `crates/desktop/qml/Preparing.qml` (new),
  `crates/desktop/cpp/gpu_device.cpp`, `crates/app/src/app.rs` (`App::open`), `crates/catalog/src/progress.rs` (a cancel),
  `crates/driver`.
- **Change:**
  - `run()` starts the GPU device and the Qt app, then spawns the open worker: the preparation, the cache's open (without
    the heal, 4.5), the pools, the reader.
  - Within 300 ms it proceeds as today. Otherwise it loads `Preparing.qml` in a separate engine and shows it, with
    progress queued to the Qt thread.
  - Close or Quit cancels the preparation; the next start resumes.
  - The driver prints progress lines to stderr.
- **Tests:** a slow stub preparation shows the window after 300 ms; cancel exits within 100 ms; one `show_windows` over
  Main's roots; a fatal error shows today's message; `tst_preparing.qml`.

### Phase 3: Scanner and pairing

**Files:** `crates/scan/src/{lib.rs, sink.rs (new), order.rs (new), journal.rs (new)}`, `crates/platform/src/{dir.rs,
usn.rs, token.rs}`, `crates/catalog/src/{roots.rs, pair.rs, removal.rs, subtree.rs, usn.rs (new)}`,
`crates/testkit/src/ntfs.rs` (new), `crates/library/src/{lane.rs, passes.rs, removals.rs}`, `crates/desktop/src/library.rs`
(the Full rescan action), `crates/driver`.

**3.1 Walk.**

- **Identity set.** The walk's `BTreeSet<FileIdentity>` becomes a temp table on the lane's connection,
  `temp.scan_ids(volume, file_ref, PK) WITHOUT ROWID`, probed with INSERT OR IGNORE (on disk after 1.1).
- **Inline pass.** Up to 16,384 kept entries in the reused Vec, with the folder's rows read before the listing, as today
  (`docs/plans/SCAN_OPTIMIZATION_PLAN.md` Phase F).
- **Validate pass,** when a folder overflows: O(1) state (no-id flag, I/O result, count). A failure leaves the folder
  unreadable, as today.
- **Relist pass.** It streams each entry through the per-entry decision, in listing order, through WriteBatch, with an
  identity probe per entry instead of the folder's whole `FolderRows`. Phase F's guarantee (rows read before the listing,
  identity reads safe for hard links) must be re-argued for the per-entry probe before it is built. On failure:
  `truncate_to(mark)`, the folder recorded as failed, no descent; rows already recorded stay.
- **Frames.** Beyond 16K pending subfolders they go to a codec SpillStack. The workers' look-ahead queue holds jobs for
  the next frames only, so it stays inside its `AHEAD` window instead of holding a second copy of every pending folder.
- **Out of space** (scratch ENOSPC or SQLITE_FULL): drop the SpillStack, roll back the open batch, return
  `OutOfSpace{Scratch|Catalog}`. The result is the state a crash would leave.
- **Stop and pause** inside a listing: `Listing::each` returns ControlFlow, and the pass's checkpoint runs every 1,024
  entries. Today the walk checks only at the stack pop.
- **For 3.9:** `visit_folder(lister, frame, mode Walk|Dirty, sink) -> FolderTally`, extracted with today's per-entry
  decisions unchanged; sinks `DirectSink` (the writer through WriteBatch) and `SortedSink` (ExtSort by walk_key);
  `walk_key` (order.rs): a folder is `wk(parent) ++ 0x01 ++ invert(collation_key(name))`, a file
  `wk(D) ++ 0x00 ++ collation_key(name)`, so lexicographic order equals the LIFO walk's; the collation check on every
  listing; mirror upkeep and the base cursor (3.9).
- **Deviations:** (a) a relist failure keeps the rows recorded before it; (b) a stop during a relist leaves part of the
  folder recorded; (c) out of space is typed.

**3.2 Scan identity pairing.**

- Today (`crates/catalog/src/pair.rs`): the fresh rows come from `temp.scan_seen` through its unpaired index, in 1,000-id
  batches; the tiers run per batch; `ambiguous` is a BTreeSet; `Pairing.{replaced_ids, deleted_ids, temps}` go back to
  `after_pairing` (`crates/library/src/passes.rs`).
- **Change:**
  - **Stoppable pages.** `in_batches` checks the pass's Control between pages. The position (tier, last id) lives in the
    pass's state for as long as the lane's connection (and with it `temp.scan_seen`) lives; a resume skips ids ≤ the
    position in the current tier. A restart walks the root again, as today.
  - **The ambiguous set** spills to a temp table when it passes one page.
  - **The outbox** (3.5) takes the replaced and deleted ids.
  - **`unpaired_with_data`** reads through `files_unpaired(folder_id)` joined with the root's folders, O(that root's
    unpaired rows), not every row under the root (Q-S2 a).
- **Tests:** a stop between pages, then resume, equals the uninterrupted result; a renumbered volume pairs with O(page)
  RAM; the differential proptest against `reference_pair` at page sizes 1, 2 and 1000.

**3.3 mark_missing.**

- Today (`crates/catalog/src/roots.rs`): a `Vec` of every folder of the root and a `Vec` of the unseen rows, read in one
  snapshot, then per-row writes in batches of 1,000.
- **Change:** `FolderPages` over the root plus `SkipMerge` for the folders the scan could not list; per folder, the unseen
  rows by keyset on id with `not_seen!()`, in short read transactions (D31); per-row writes through WriteBatch. A journal
  pass runs the same statement over its scopes (D38, 3.9).
- **Tests:** equals the in-RAM oracle on nested, sibling, UNC and drive roots; a 25,000-row single folder pages at most
  one page per read.

**3.4 Root removal.**

- Today (`crates/catalog/src/removal.rs`): steps A to E. `begin_removal` takes the root and its leases; `hide_pages` reads
  every present id under the root into a `Vec`, then writes `missing = 1` per row; `drive_one` deletes in `LIMIT 1000`
  pages by a folder rarray; `finish_folders` drops empty folders, `scans` rows and the marker; `shrink` returns free
  pages. The lanes of the folder's volumes stay parked until the files are hidden.
- **Change:**
  - `hide_pages` pages ids by keyset within `FolderPages`, writing as it reads.
  - `drive_one` pages folders by `path_key` and keeps `root_removals.cursor` as a forward watermark, so kept (deferred)
    rows are not read again on each page.
  - The service also pages out the root's journal rows (`folder_journal`, `usn_*`, 3.9).
  - **With links** (§1.3): the hide step deletes the root's links in pages, and marks missing only the files left with no
    present link, so a file with a link under another root stays present.
- `add_root` keeps refusing a folder whose removal is pending (the shipped decision; the 2026-11-07 "cancel" is dropped).
- **Tests:** a 25,000-row single folder pages at most one page per read; kill and resume; a nested root; a same-named
  folder on another drive stays.

**3.5 Outbox.** When the replaced and deleted ids of a pairing pass exceed one page, they go to a temp table on the
lane's connection and are drained in pages to `after_pairing`, which forgets their thumbnails through `ForgetMany`
(chunks of 256, `crates/thumbcache/src/writer.rs`).

**3.6 `import --again`. Moot** (the command line is gone). `file_by_path` already iterates `folders.match_key` across drives
until the rules' `same` check matches (`crates/catalog/src/lookup.rs`).

**3.7 Housekeeping.** `prune_files_log()` (LOG_KEEP 1M, down to 512k, in paged deletes, advancing `log_horizon`) runs at
the end of each Scans pass (→ 7.1). `begin_scan` already empties `temp.scan_seen` and purges expired leases.

**3.8 Missing-writer allowlist and fan-out** (D10).

- **Allowlisted statements:** ScanWriter's displace and its changed-file UPDATE (`crates/catalog/src/writers.rs`); pairing's
  `take_identity` (`crates/catalog/src/pair.rs`); per-row `mark_missing` (`roots.rs`); the removal's hide (`removal.rs`);
  the paged held-extensions step (2.5); fixture-only statements in `fixture/raw.rs` and `fixture/state.rs`.
- A source test matches only SET clauses: `UPDATE\s+files\s+SET[^;]*?\bmissing\s*=`.
- The fan-out c = Δ`sqlite3_total_changes` per toggle, at degree 0, 1, 10 and 1000, is pinned to the derivation from the
  trigger text. P (pages per row) is pinned.

**3.9 Journal rescan** (D37, D38; Q-S1 (b); Q-N1)

The journal only says where to look. Listings are the truth, and results replay in walk order through the same writer.
This is the app's one journal pass (Q-N1): `docs/plans/NTFS_MFT_SCAN_PLAN.md` keeps the full MFT read, the helper and
`ResizeJournal`, and its own journal-pass section yields to this step. The 2026-11-07 design stands with these changes:

- **Taken from the MFT plan:**
  - **The own-write bracket.** Write-back records `files.own_from` and `files.own_usn` around its rewrite (its "Our own
    writes: the bracket rule"). A journal record for the file inside the bracket is the app's own and makes nothing
    dirty, so a write-back burst does not turn into relists. A record outside it is external, as before.
  - **Data-change reasons** (`DATA_OVERWRITE`, `DATA_EXTEND`, `DATA_TRUNCATION`) mark the file's folder dirty in R2, even
    when size and mtime come back unchanged. What the relist then records is still the directory-index values (value
    truth). A rewrite that restores both size and mtime is caught only if the catalog keeps a content generation: that is
    the MFT plan's `files.content_gen`, adopted with this step, bumped by an external data-change reason.
  - **An MFT read seeds the cursors.** A full MFT read stores its start USN (U0) as the cursor of every root it covered,
    in `usn_roots`, instead of a per-volume position. Roots with equal cursors still share one read (R1).

- **The links model (§1.3).** Every listed name of a media file is a link row, so:
  - **no winner link.** The W2 inline rule (after the claim probe, `links()` and the link with the minimum walk key
    upserts, the others count as duplicates; ordinals listed for two non-ASCII links in one folder; abort to walk on
    Denied) is dropped, and so are `usn_entries` kind 1 and the census's `extra_links`. A relisted folder writes its links;
    a dropped name deletes its link; the file's values follow the greatest (mtime, size) rule over its links, whatever the
    order of the replay.
  - **hard-link changes still need `links()`** for one thing: a HARD_LINK_CHANGE record names one parent, and the other
    folders holding the file's links must be relisted too. R2's rule ("every link parent in the root becomes dirty")
    stays; only the winner choice goes. Deviation J-a stays: a link's listed values can still be stale.
  - **per-directory state** is `folder_journal`, beside the folder row of each directory (2.1).
- **Seen sets instead of a floor.** A journal pass J claims its links and files in the two seen sets, as a walk does (D38).
  Its unseen set is what it did not claim in its scopes: `usn_drop` (dropped refs) and `usn_scope` (relisted folders,
  folder-only; sub-walked subtrees). The sweep (3.3) runs over those scopes. The `stamp_dropped` and `stamp_scope` writes,
  the sentinel scan and `scans.floor` are gone.
- **Resume.** Because the seen set is a temp table, J resumes only within the lane connection's life. A stop or a restart
  in W1-W6 abandons J: nothing has been marked missing, the cursor is unchanged, and the next pass redoes it. This equals
  a stop plus a rescan (Q6). The 24 h sealed-file adoption is dropped.
- **Fallbacks.** On Windows only local NTFS roots exist (non-NTFS and SMB are refused), so the walk fallback list is:
  a first scan, a ladder failure, a verification walk, a requested Full rescan, Linux roots, detachable volumes. Where the
  helper can elevate, a first scan and a Full rescan take the MFT read instead of the walk (the MFT plan's source choice).
- **Offline volumes** are skipped: their lane has nothing to run, and the cursor waits until the volume returns; the
  ladder's journal-id and LowestValidUsn checks then decide between a journal pass and a walk.
- **Runner and app.** The Scans pass tries J and walks on `NeedWalk(reason)` in the same pass, recording the reason in
  `usn_roots.last_reason` and the log. Rescan uses the journal; a Full rescan action forces the walk. `pg-driver run
  --passes scans` gains `--walk`. The journal status and size commands become driver subcommands, not CLI ones.
- **Journal sizing with consent (Q-S6)** stands: the app offers an enlarge once per volume per size bucket, through the
  per-session helper's `ResizeJournal` request; it never resizes silently.

The rest of the 2026-11-07 specification is kept as the build reference for this step, read against these changes: the
R0 ladder, R-A access re-probe, R1 read, R2 classify, R3 dirty folders (Relist and Probe modes), R4 SubWalk plan, R5
recurring sets, the cost rung (Q-S4), R6 seal, W1-W7 replay, the equivalence invariant, the watch (Q-S7), the fake
volume in `crates/testkit/src/ntfs.rs`, the memory and disk figures (§2), and the oracle and fault suites
(`crates/scan/tests/{usn_oracle, usn_faults}.rs`). It is not repeated here: `git show
14856c5f:docs/plans/BOUNDED_MEMORY_PLAN.md`, step 3.9.

### Phase 4: Work lists, write-back, thumbnails

**4.1 Work-list cursors** (D5, D6, D31)

- **Files:** `crates/catalog/src/work.rs` (`pending_import`, `pending_hashes`, `pending_placeholders`, `pending_borders`),
  `crates/model/src/path.rs` (`WorkList`), the pass crates.
- **Change:**
  - Each list becomes a keyset `Feed` (1.8) in (volume, file_ref, id) order, over its partial index (`INDEXED BY`):
    import over `files_to_import`, borders over `files_borders_due`, placeholders over `files_present_order` with an
    anti-join on `placeholders`, examining at most 4,096 rows per fetch.
  - Hashes: `hashes_owed` is keyed by `file_id` alone, so it has no (volume, place) order. When the owed set fits one
    page it is read and sorted as today; beyond a page it goes through ExtSort by (volume, file_ref, id) first, so the
    pass still reads in disk order.
  - Paths come from the FolderCache (1.7), not a `BTreeMap` of every folder.
  - While `bulk_walk_<kind> ≠ 'done'`, a second stream walks `files_present_order` from the frontier and returns flag-2
    rows (2.5), merged in key order.
  - Rows with `seq > S0` are skipped and lower the total (D6). The total comes from `counts` (2.6).
  - The Pairing pass's list is Phase 5's.
- **Tests:** drains equal the old SQL at page sizes 1, 2, 7 and 1024, including bulk flags and a stop at every fetch;
  total examined ≤ n + stops × 4096; `done == total` at Exhausted; a mutation yields each id at most once.

**4.2 Borders pass** on the cursor. The failure set (`known`, `crates/borders/src/lib.rs`) becomes a per-page probe of
`file_failures`. The final flush runs on every `Ended`.

**4.3 Write-back queue.**

- **Files:** `crates/catalog/src/queue.rs`, `crates/writeback/src/lib.rs`, `crates/platform/src/volume.rs`.
- Today: `NEXT_READY` filters `attempts`, the settle time and `file_id NOT IN rarray(blocked)` and orders by
  `temp IS NULL, queued`, which is O(Q) per call; `drain_queue` keeps offline jobs in a `blocked` Vec and counts the queue
  per job; the copy, hash and verify read whole files into RAM.
- **Change:**
  - **next_ready:** at most 10 probes on `file_writes_ready`: for t in (0, 1), then a in 0..=4, the first job with
    `(temp IS NULL) = ?t AND attempts = ?a AND queued <= ?now - ?settle * (?a + 1)` by `queued, rowid`; take the minimum
    over the first t with a hit.
  - **Hierarchical parking** replaces `blocked`: an offline failure parks the job on its (mount root, folder); a prober
    thread checks at most 16 roots and 16 folders per tick, with PROBE_WAIT 5 s and at most one probe per root; a present
    folder unparks its oldest job as a probe; producers' upserts and the files trigger unpark.
  - **Streaming I/O:** sniff from a 4 KiB prefix; hash by streaming; copy through a 1 MiB buffer; verify through a
    read-only mapping, dropped before `remove_file`.
- **Deviations:** online jobs no longer wait behind offline ones; a share slower than PROBE_WAIT is parked, not counted
  as an attempt.
- **Tests:** the order oracle against today's `NEXT_READY`; VM steps with 10^6 rows all settling; 10^5 parked folders on
  an absent root cost one probe per 30 s; a hung probe does not stall the drain; identical bytes and digest, and peak RAM
  below 64 MiB on a 1 GB file.

**4.4 Thumbnail cache layout** (D35)

- **Files:** `crates/thumbcache/src/schema.rs`, `crates/model/src/versions.rs` (`CACHE_VERSION`).
- Today: `entries` (with `UNIQUE(file_id, height)`), `entries_lru`, `data`, `totals`, `limits`, `meta`, `checked` and
  `checkpoints`, and three totals triggers.
- **Change:** an index `entries_height(height, file_id)` for the merge, so a round reads only its height's entries;
  `height_totals(height PK, bytes, count)` maintained by the totals triggers; `entries_height_lru(height, last_used,
  bytes)` replacing `entries_lru`; heal cursor keys. A `CACHE_VERSION` bump starts existing caches over.
- **Tests:** counters equal `count(*)` per height; the merge's plan uses `entries_height`; a cache of the old layout starts
  over.

**4.5 Cache writer ops** (D35)

- **Files:** `crates/thumbcache/src/{writer.rs, maintenance.rs}`.
- **Change:**
  - **evict:** a K-way merge over per-height cursors on `entries_height_lru`, for heights other than the kept one, in the
    total order (last_used, height, bytes, id); at most 1,024 victims per step. A short-circuit: if Σ
    `height_totals.bytes` over the other heights is below the excess, return without walking.
  - **Heal:** `open` only queues `HealIfDue`. The writer runs 65,536-id chunks between ops, then fixes the totals and
    `height_totals`, and publishes `TotalsFixed`.
  - **Inbox:** a 32 MiB cap on PutOne payloads drops the oldest PutOne (regenerated on demand).
- **Deviations:** `TotalsFixed` may follow the open; a PutOne dropped under a sustained writer stall is regenerated later.
- **Tests:** the evict oracle (today's ORDER BY), with the kept height dominating; heal is exact under puts from another
  instance, and open returns in under 10 ms on 1M entries; the inbox cap.

**4.6 Thumbnail rounds, paged** (D35)

- **Files:** `crates/thumbs/src/{pending.rs, lib.rs, placeholders.rs}`, `crates/catalog/src/work.rs`
  (`thumb_candidates`), `crates/thumbcache/src/{lib.rs, reader.rs}`.
- Today: `pending::work` reads the volumes and the cache's marks, streams the candidates by id (through `files_seq` above
  a checkpoint), streams the height's entries by id, collects the owed items into `kept`, and sorts them by (volume,
  place).
- **Change:**
  - **No checkpoint for a volume** (a first pass, or a reset by eviction): walk `files_present_order` for that volume in
    keyset pages. The pages are already in (volume, place) order, so no sort is needed. Each page probes the cache for its
    ids through `entries_height`, and feeds the owed ones to the pipeline (1.8).
  - **A checkpoint:** the candidates above it are the changes. When they fit one page, merge and sort as today; beyond a
    page, the owed items go through ExtSort by (volume, place) before the feed.
  - The checkpoint advances as today: only when a round completes (`ThumbCache::checked`), and held back to the lowest
    change number of a file that must be tried again (`first_failed`). Today a stopped first pass starts over; with
    paging it resumes from a walk position kept in the cache (`checkpoints` gains a `walk` column for that height and
    volume), which the completed round clears.
  - The failure set (`known`) becomes a per-page probe of the catalog's failures.
  - **Cache-full memo (Q-T4 a):** before decoding, if the cache's state and the digest of the next batch's (id, seq) match
    a stored rejection whose candidates all succeeded, stop as cache-full without decoding.
  - **Delete the legacy path:** `pending_in` and `pending_for` (`crates/thumbcache/src/reader.rs`) and
    `ThumbCache::pending`, with their 128 MiB cache and in-RAM temp; port their tests and the bench
    (`crates/bench/benches/ir/catalog.rs`) to `pending::work`.
  - **Placeholders** take the same paged walk (4.1).
- **Tests:** the differential test against today's merge (ids, order) with and without checkpoints; a stop every page
  still advances; an eviction reset walks once; a cache-full round decodes 0 after the memo; plan guards.

**4.7 `thumbs::run`.** `needed` becomes `Option` (unknown while a first-pass walk is open, R-TH2), and every `Ended`
writes the failures and, when complete, the checkpoint, as today. The 2026-11-07 plan removed `first_failed` with its
laps; under the checkpoints it is what holds a checkpoint back past a file to retry. `docs/plans/IMPORT_FAILURES_PLAN.md`
(D-A) replaces it and `fold` with a `thumb_due` union branch of the work list; 4.6 pages that branch with the
candidates. The steal and lease parts are moot: the command line is gone, and the driver never opens the user's library.

**4.8 `borders --csv` and `--import-review`. Moot** (the command line is gone).

### Phase 5: PDQ pairing at 1B

Today (`crates/import/src/pairs.rs`, `crates/dedup/src/join.rs`, `Catalog::pairing_work`): each run loads every stored
picture's hashes and the identity, present, pending and owed id sets, builds the join's store (16 chunks of 16 bits, 4
stored orientations, sorted per stage on a thread team), and joins slices of 200,000 owed pictures, writing pairs in
transactions of 4,000 under the `pairing_epoch` guard. RAM is about 270 B per picture, and more than 268M pictures fail.

**5.0 Design round (before build).** The goal stands: RAM O(slice), the 268M cap gone, results equal to today's join. The
2026-11-07 tier-1 design (thin and fat sheets, EVEN31 and FULL31 layouts, the sparse and sweep join modes, the heavy core,
merges and debt, calibration) is the starting point, but it was designed for 8 × 32-bit chunks and an import-time clock.
Re-derive against the shipped join:

- the stored side on disk, in the join's own per-chunk sorted order, written incrementally in `pdq_indexed` seq order and
  merged (4-way, ratio 4) as it grows;
- a slice's join reads the stored side sequentially (sweep) or by probe blocks (sparse), chosen by a measured crossover,
  with the shipped kernels;
- dense clusters: measure the pair degree on the user's library and on planted clusters before deciding whether a heavy
  path (D26) is needed at all;
- the id sets of `pairing_work` (identity, present, pending, owed) become probes or a merge in id order, not in-RAM sets;
- each run stops reloading every picture: the store persists between runs, and only owed pictures are read.

The round produces the §2 disk and time figures, the residuals that replace R-PDQ1 to R-PDQ5, and the tests (the
brute-force oracle at every layer, crash points between transactions, a planted clique, today's join as the oracle). It
also decides whether the tier-1 writer needs a lease between app instances (D16).

**5.1-5.10.** Re-numbered by the design round. The 2026-11-07 text is in git for reference.

### Phase 6: The Duplicates filter, and duplicate groups

**6.5 Duplicates filter** (D27)

- Today: EXISTS over `analysis_file_hash`, `analysis_pixel_hash` and `pdq_pairs` (`crates/query/src/lib.rs`), and
  `NOT_DUPLICATES` adds owed-queue checks. Each probe walks a hash run or a file's pairs until it finds a present partner.
- **Change:** the hash part is `hash_runs.present − self ≥ 1`; the PDQ part is `pdq_near.n ≥ 1`; triggers keep both. A run
  crossing present 1 ↔ 2 stamps `files.changed` on both affected members, found in O(1) through `present_xor`; `pdq_near`
  crossing 0 ↔ 1 stamps its file. Duplicates views then take deltas, and the grid's "always rebuild" exception goes. The
  owed-queue checks of `NOT_DUPLICATES` stay.
- **Tests:** the filter equals today's EXISTS forms, including cascade deletes of present and missing files with twins;
  every status flip stamps.

**6.1-6.4, 6.6-6.8. Deferred** until `docs/plans/PLAN.md` M3 (duplicate review) is planned in detail (Q-N2, decided).
Today there is no grouping pass: `identical_edges`, `pdq_edges` and `UnionFind` have no caller outside their crates, and
the `dups` report they fed went with the command line. Groups are still a planned feature (`docs/plans/PLAN.md` M3,
`docs/plans/IMAGE_QUALITY_PLAN.md` phase 4), but grouped by multicut over verified tier 0-3 edges, not by connected
components over tier 0-1 pairs. When M3 is planned, these steps are redesigned with it, after 5.0. The 2026-11-07 design
(edge-granular checks, a disk-DSU build, the maintainer under its lease, the O(show) report, the overflow switch) is in
git as a starting point for the components layer.

### Phase 7: Grid, browse, tree, Qt edge, edits

**Probe findings (2026-11-11).** These are measurements, not decisions. Each finding sits under the step it informs. The
probes covered scene-graph precision (a render test, P10), sub-pixel snapping, the aspect distribution (3000 photo
headers), the window size, the density formula, `ItemRef` bursts under assumed flick speeds, `RankMap` cost from 1e6 to
1e9, and re-sync reach from 1e5 to 1e7 with three row caps. Still unmeasured: `ItemRef` queue depth in the real pipeline.
Recommended but not confirmed: a capped re-sync with a frozen tail (7.6), and the rule for a move from none (7.9). The grid
design is not final.

**Files:**

- `crates/runs/` (new: the on-disk grid runs)
- `crates/catalog/src/browse/read/{grid, facets, tree, scope}.rs`, and new `changes.rs`, `names.rs`, `range_index.rs`
- `crates/app/src/{reader.rs, views.rs, images.rs, library.rs}`
- `crates/client/src/{layout.rs, grid.rs, window (new), tree.rs, controller/*, facets.rs}`
- `crates/desktop/src/{gallery/*, shell.rs, rows.rs, folders.rs, picker.rs, worker.rs, library.rs}`
- QML: `WindowView.qml` (new; `WheelListView.qml` is deleted), `GalleryGrid.qml`, `Main.qml`, `FolderTree.qml`,
  `FolderPickerDialog.qml`, the viewer's and HUD's files, and the QML tests
- `crates/crop/src/lib.rs`

**7.1 Paged ChangeFeed.** `files.changed`, `files.seq`, their indexes and the stamping triggers exist
(`docs/plans/DECODE_ONCE_PLAN.md` Phase 4). `docs/plans/CATALOG_SIZE_PLAN.md` Phase 1 drops the `files_changed` index
(its D2, the owner's decision of 2026-11-19), so this step adds it back with the feed, unless the web client has already.
This step also adds `files_log` with the `files_removed` and `files_renamed`
triggers, the `log_horizon` and `view_epoch` seeds, `prune_files_log` (3.7), the duplicate-status stamps (6.5), stamped
grid results, the cursor-paged feed with the converging read (D31), and the behavioural schema test that every column
`grid_sql` reads is covered. Today the bus carries only coarse events (`Change::{Scanned, Imported, ...}`,
`crates/model/src/job.rs`), and nothing reads `changed` outside tests.

**7.2 Facets** as chunked steps, plus a contribution log. Today: one `GROUP BY` over the scope
(`crates/catalog/src/browse/read/facets.rs`).

**7.3 Reader lanes,** with List coalescing. Today `crates/app/src/reader.rs` is one thread that keeps the newest request
per kind and cancels the running one. Every catalog-sourced result carries its snapshot stamp s = max(changed) (D33).

**7.4 build_run.**

- Today `list_grid` builds a `Vec<Entry>` (56 B per entry); a sorted view builds `(Value, Entry)` pairs and sorts them in
  RAM. After the MFT plan's step 1 an entry is a link, and a run is built over `links_order`.
- Single-lower-bound steps of 65,536 examined rows into an on-disk run (`crates/runs`). The sidecar sort removes
  duplicates.
- **The post-build delta is a converging paged read** (D31).
- The name driver (7.16) and the range-index driver (7.17).
- **Sorted views:** a live top-K preview (K = 4096, a bounded heap over the rows examined so far, marked provisional) while
  the build runs. It re-anchors at the same top item when the build lands.

**7.5 refresh_run.** An uncapped two-phase delta (converging), a RankMap, and persistence across restarts. Today a refresh
is a full re-list (`Event::Reload`, `crates/client/src/controller.rs`) on `Change::Imported` and `Change::Scanned`.

- *Probe P9:* rank updates through a Fenwick tree are O(log n) and bit-equal to a full recompute. The row-layout delta
  regenerates the tail, so it inherits 7.6's invalidation problem. At 1e7 items with 1e4 changes it regenerates 104k to
  110k of about 120k rows, a 1.2 to 3× gain over a full relayout.
- *Probe RankMap:* a Fenwick over every slot of `i32` costs 5 bytes per slot, 4.66 GiB at 1e9 slots, which the scale rules
  out. A blocked form (u64 presence words, a Fenwick over per-word counts, popcount inside a word) gives checksums equal
  to the plain form at 1e6 and 1e7, passes brute-force checks at 1e6 through 1e9, and costs 0.19 bytes per slot (179 MiB
  at 1e9). Rank takes 13 to 83 ns, and update 12 to 77 ns. The universe's Fenwick should be the blocked form.

**7.6 Directory.** Today `crates/client/src/layout.rs` builds `Row{first, count, height}` (24 B) over the whole list.

- *Probe P4, not supported:* after one edit, the layout's re-sync with the old boundaries is heavy-tailed. On continuous
  aspects, over 1000 edits at 1e6 items, p99 is 103,112 items and 1,244 rows past the edit. On photo-like aspects, 69 of
  1000 edits never re-sync. Uniform 3:2 never re-syncs in 200 edits. Bounded invalidation is therefore not supported.
- *On the user's aspects* (3000 header aspects resampled to 1e6 items, probe `real`): 980 of 1000 edits re-sync. Median 796
  items and 9 rows past the edit; p99 264,079 items and 2,712 rows; max 444,031 items and 4,561 rows. The other 20 never
  re-sync.
- *Re-sync reach grows with the catalog* (probe `scale`): p99 items past the edit are 37k at 1e5, 148k at 1e6 and 448k at
  1e7 (4.5% of N). The max at 1e7 is 1.75M (17% of N). Exact regeneration stays cheap (47 to 138 µs per edit).
- A cap on the regenerated rows, with a frozen tail, keeps the y error within about one row pitch: max 65 px at 1e7 for
  caps 256 to 4096. The cost is membership: in the frozen tail, items sit in the wrong row until the background re-sync
  catches up. Recommended but not confirmed: cap at 4096 rows, which covers 95% of edits at 1e6 and 99% at 1e7.
- *Density formula (derived and checked):* ρ ≈ (W+S) / ((ā·T + S)(T + S)), where ā is the mean of the clamped aspects.
  Predictions against measurements at W 7680: the user's aspects (ā = 1.2557) 1.513 against 1.530; synthetic Photo 1.592
  against 1.607; Continuous 1.284 against 1.311; densest (ā = 0.05) 17.15 against 17.16. The clamp caps the density at
  17.15 items/px at W 7680.
- *Open:* the re-anchor that completes a capped re-sync in the background, and whether a frozen tail's membership error is
  acceptable in the UI.
- u128 fixed-point checkpoints, bit-identical across parallel, splice and sequential builds.
- `density.rs`: k_max, h_min, min_pitch and rho, from the formula and the catalog's own mean aspect.
- **ProvisionalGeometry,** and a re-anchor that keeps contentY.

**7.7 Client grid.** Today `crates/client/src/grid.rs` holds `entries`, `rows`, `spare_rows`, `held` (a second whole list
while the viewer is open) and `pending: BTreeMap<FileId, usize>`; versions are u64.

- *Probes P2 and P3, semantics hold, not yet implemented:* epoch-checked `ItemRef` dispatch misapplied 0 of 100k random
  interleavings, and the index-only control misapplied 99,964. `RefBook`, with two retained maps, resolves in-window
  references and rejects older ones without silently remapping them. The largest map was 78 KiB at 10k entries.
- *Probe P8:* a checkpoint is 32 B. The full row list at 1e9 items is 224 to 276 MiB. The 11 MiB client figure holds only
  with checkpoints every 4096 items (7.45 MiB at 1e9), not with the row list.
- Rank everywhere. **RefBook** (at most 2 retained RankMaps). The overlay folds stamped pendings until s ≥ c. `held` becomes
  a pinned run handle. `position_of`, `entries()` and the full relayout are removed.

**7.8 SlotWindow.**

- *Probe P6, golden rows for `listview_rules.json`:* (1) a same-count reset with no current row: a ListView goes to the
  top, while a Flickable keeps `contentY`, so the port saves and restores it. (2) A same-count reset with a current row: a
  ListView scrolls to show it. (3) A current item moved by reset: a ListView scrolls to its row index, and following by
  identity needs an explicit lookup. (4) A window origin move of +3 slots by reset: a ListView resets to the top, and the
  port needs `contentY -= 600`. (5) The same +3 move by removing the top three and appending three: a ListView keeps the
  visible content, and the port needs `contentY -= 600`. All five were run against a Qt 6.12 ListView (slotprobe run 2)
  and match. `golden/listview_rules.json` is not yet in the repo.
- Synchronous slot assignment for viewport ± 1500 px.
- *Window size:* the window holds 1,705 items at 1920×1080, 4,140 at 3840×2160, 4,698 at 5120×1440, and 11,462 at
  7680×4320 for the user's aspects. The clamp's densest case (aspect 0.05) holds up to 127,368 items at 7680×4320. The
  capacity bound is ρ_max·(vh + 2·1500) + 2·k_max, about 127,700 at 7680×4320.
- *Items entering per frame (assumed flick speeds):* at 10,000 px/s, at most 316 per frame at 7680×4320 on the user's
  aspects, and 86 at 1920×1080. A seek replaces the whole window (11,462 at 8K). This is the queue model only.
- Slab rebase only at idle, and only beyond 2^40.
- ListView rules ported from the Qt 6.12 sources (current remap, keys, follow, Contain and Beginning, reset), pinned by
  `golden/listview_rules.json`.
- The bar's seek is coalesced per frame.

**7.9 Desktop Qt edge.** Today rows cross as `int` (`crates/desktop/src/rows.rs` `to_row` and `qt_int`,
`crates/desktop/src/shell.rs` `to_i32`, the model's `row_count` and `set_current_item(i32)`), and file ids as `f64`.

- *Probe P1, absence at QML receivers:* a typed `int` receiver or parameter turns an absent value into 0, and `select(int)`
  given `undefined` or `null` selected row 0. Only a `var` or a Rust-checked `QVariant` keeps absence, so Rust must check
  before a value reaches a typed side.
- *Probe P5 (property level):* QML doubles round-trip exactly to 2^53 for integers, and `contentY` 2e11 reads back exactly.
  This does not show what the scene graph renders.
- *Probe P10 (rendered; supersedes P5's claim that a Flickable at 2e11 positions its children correctly):* a 1 px marker
  inside the Flickable's content item lands on the wrong row from `contentY` 2^24. Float32 spacing explains it: 2 px in
  [2^24, 2^25), 4 px in [2^25, 2^26). By 2^28 the markers collapse to one row. A sibling marker whose y is computed in
  double is exact at every offset tested, up to 2e11. The slot layer must be a sibling of the content item, never a
  child.
- *Probe sub-pixel:* a 1 px rectangle at y = 10.25 renders in row 10, and at 10.5 or 10.75 in row 11. Qt snaps positions to
  whole pixels, so float error does not reach the rendered result. Images and text were not tested.
- *Probe P7, read-only, re-checked 2026-11-25:* the controller cannot yet be the sole writer of the grid's current item.
  Click selection is written in QML (`Main.qml`, `onCellClicked: win.current = i`). The controller learns of changes by
  echo (`Main.qml` `onCurrentChanged` → `shell.currentChanged` → `Event::CurrentChanged`,
  `crates/client/src/controller/input.rs`). The viewer's index shares the field (`shell.currentChanged(index)` and
  `onClosed: shell.select(i)` in `Main.qml`). The anchor is the view's top item, which needs view geometry (`topItem()`,
  `GalleryGrid.qml`).
- *Rule for a move from none (recommended, not confirmed):* from none the position is −1, so a step of +d lands on row
  d − 1, clamped to the rows. Up from none lands on row 0.
- *Version width:* versions are `u64` in Rust, cross as `i64` with a checked conversion, and QML's `real` holds them
  exactly up to 2^53. A version above 2^53 fails loudly.
- f64 properties; a `viewedFile` string; `loadedAnchor`; the `lastFileIndex()` invokable; `save_last_file(Option)`
  (reconcile with `docs/plans/TYPE_MISMATCH_VALIDATION.md` item 1, which owns the saved last file).
- ItemRef stamping on every queue hop.
- `to_i32` and `qt_int` are deleted; i32 conversions that remain carry a `qt-edge-bound` reason.
  `docs/plans/QT_SENTINEL_SWEEP.md` follows D29's id form.

**7.10 QML WindowView.** A plain Flickable, plus a non-moving slots layer that is a sibling of its content item, plus a
Repeater. Typed scalar signals; doubles throughout; `'f',0` formatting; `file` strings.

**7.11 Folder tree.** Today the tree is held whole three times: `FolderTree` (`crates/model/src/tree.rs`), the client's
`Tree` (`expanded` and `visible`, `crates/client/src/tree.rs`) and the Qt model (`crates/desktop/src/folders.rs`), whose
ListView owns `currentIndex`.

- Streaming per-generation tree files; FolderModel as a `SlotWindow<Uniform>`; a model-owned `currentRow` with the ported
  rules and a native attached bar; a row LRU of at least 2·S. The tree stays merged across drives.

**7.12 Folder picker** (non-Windows). In RAM up to 4 MiB of names, synchronous; beyond that, spill-sorted on a worker into
a PagedFile with offsets, with generation cancel; a slot model. Today `pg_platform::dir::folders_in` runs on the GUI
thread (`crates/desktop/src/picker.rs`).

**7.13 Crops.** A streamed list from the catalog (bordered files in scope), with a prepass refusal and unordered resume.
Today the ids come from the client's entries (`crates/desktop/src/gallery/edits.rs`) and go to `save_crops` and
`cut_borders(Vec<FileId>)` (`crates/app/src/library.rs`).

**7.14 Live pages.**

**7.15 Edit worker.** Re-based on `crates/desktop/src/worker.rs` (the window-writes thread, an unbounded FIFO, drained by
`worker::stop`). It gains the remaining GUI-thread writes (`save_view`, `delete_view`, `add_folder`, `remove_folder`), a
FIFO cap of 1,024, and an ack carrying the commit stamp c, so the overlay (7.7) folds until c is covered. Deviation: an edit
shows at once and reverts if the write fails.

**7.16 Name index.** The `names.sqlite` FTS5 trigram file (contentless, detail=none), with names padded with two U+0001,
opened as the Names role. Fed by the ChangeFeed in the Names pass (D17). Needles of 1 or 2 characters go through
`fts5vocab` term-range prefix expansion. It is used when Σ doc < scope/8; otherwise the build scans. Today every edit scans
the scope with `pg_contains` (`crates/query/src/lib.rs`).

**7.17 Opt-in range indexes** (Q-G1 a).

- `gallery.json` `grid.range_indexes` lists any of size, mtime, taken, megapixels and duration.
- **Build.** Each listed column is a 2.3 Shadow access path, `sh_<col>(value, id) WITHOUT ROWID`, built online as an
  Upgrade step. A plain CREATE INDEX is ruled out after the freeze (the guard refuses it on LARGE_TABLES).
- **Upkeep:** triggers covered by `difference_allowing`, so `mismatch()` stays None.
- **Readers:** the grid build and facets use the shadow when the range's candidate estimate is below scope/8. Results are
  identical to the scan.
- **taken:** the taken step (2.5) marks `sh_taken` not ready and rebuilds it.
- **Opt-out:** one DROP TABLE.
- **Cost:** 20-30 GB per column at 1B; 1-2 h online.

**Tests:**

- `ids(build_run) == ids(list_grid_reference)` across sort key × direction × query (Duplicates, name needles of 1 to 5
  characters, each range index) × scope × rules;
- VM steps per step are O(step);
- the delta proptest, with 50 chained refreshes at page size 1, with writes between pages; Duplicates deltas;
- the window, grid, tree and density laws at 2^31 + 2^20 + 7 and 2^40;
- the RefBook proptest; the golden ListView table, reproduced in Rust and by a real ListView;
- `tst_windowview`, `tst_gridrow`, `tst_viewer`, `tst_media_view`, `tst_foldertree_rules`, `tst_folder_picker_dialog`;
- picker order equals `PathBuf::sort`;
- overlay folding: a turn plus a refresh, a rating plus a delta, an out-of-order arrival;
- crop refusal writes nothing;
- name index: lag, tombstones, reset;
- range indexes: results equal the scan in every column, direction and scope, during a build and after; opt-out and opt-in
  round trip; `set_rating` at most 150 ms during the build.

### Phase 8: Verification harness, ledger, lints, docs

- **Files:**
  - `crates/scale/{src/bin/synth.rs, tests/scale.rs, expectations.toml}` (new crate)
  - `crates/catalog/src/fixture/synth.rs` (new)
  - `crates/testkit/src/{alloc.rs, ntfs.rs}` (new)
  - `crates/scan/tests/{usn_oracle, usn_faults, usn_vhd}.rs`
  - `crates/desktop/tests/qt_edge_lint.rs`
  - `crates/driver` (a `dhat-heap` feature, like the GUI binary's; a peak-memory field in its run results)
  - `tools/compare_shots.py`, `tools/shots/asserts/*.json`, `docs/perf/sql-plans.toml`
  - `Justfile` (`plans`, `scale-smoke`, `scale`, `usn-vhd`, `scale-dhat`)
  - `.github/workflows/{ci, scale}.yml`
  - `docs/SCALE.md` (new), `docs/plans/DATABASE.md`, `docs/reference/{STORAGE, HEAP_PROFILE, PROFILING}.md`
    (`PROFILING.md`'s dhat line still names the removed `gallery import`), `crates/README.md`
- **8.1 Generator.** Synthesizes a catalog in a child process, with every trigger-maintained value checked against its
  recount oracle: `folders.present`, `borders_due`, `pdq_indexed`, the `counts` row, `hash_runs`, `pdq_near`,
  `view_epoch`, `pairs_owed` and `hashes_owed`, links with several links per file, a folder row per directory, and for the
  journal rows a `folder_journal` consistent with a fake volume tree. It spans several volumes on several drives, so lanes run at once. Today the only generators are the hand-listed
  fixture rows and `tools/profiling/synthetic_library.py`, which writes real image files (up to 200K).
- **8.2 Ledger** (`just plans`): statement tags; class, temps, fan-out and total order; "`folders.path_key` and
  `folders.match_key` are immutable"; the read-transaction census (max at most READ_TX_MAX, with offenders); the
  missing-writer allowlist (3.8); open findings.
- **8.3 Lints:** `qt_edge_lint`; autoindex, FK-index, no-upsert-on-analysis and no-REPLACE (2.1); partial-index literal
  terms; `pending_now` and `taken_now` usage; `no_memory_temp`. They join `tools/lint.py`.
- **8.4 Assert scenarios:** `--synthetic-list`, `--dump-state`, `--bar-at`; `$Asserts` in compare-shots, run on the
  working tree only and failing on mismatch: `huge-list` at 2,148,532,231 and `huge-keys` at 4,294,967,303.
- **8.5** The scale harness and matrix are in §8. Docs are updated, including the journal operations guide (the driver's
  journal commands, fsutil, Q-S5).
- **8.6 Calibration** of the 3.9 cost-rung constants c_e, c_d, c_p and c_o, and of the 5.0 crossover, from the scale runs,
  into `expectations.toml`.

## 5. Alternatives considered and rejected

Each entry was proposed and not adopted. The reason is given so that it is not proposed again without new evidence.

**Memory:**

- A fixed configured RAM budget, a semaphore of interactive and background shares, a reservation derived from installed
  RAM (W = clamp(RAM/32, 0.5, 4 GiB)), a SQLite heap limit, a job-object ceiling, and budgets exposed in `gallery.json`:
  rejected by the user on 2026-11-25. The app is for heavy users; caps cost throughput. Size independence is kept
  without them (§1.2).

**Schema, rebuild and migration:**

- Keep the old catalog file at its path during a rebuild (copy without delete, +1× disk): rejected. The kept file is moved
  aside and protected by its change counter instead.
- A write fence that keeps the path: rejected. The moved file is kept, and its change counter catches any commit, so the
  rebuild restarts at Copy (KeptChanged).
- A two-hop swap: unnecessary, because D24 turns out-of-order UNIQUE constraints into droppable indexes.
- An `immutable=1` source for the kept file: rejected.
- A stepped in-place recount after every change: partly overturned. D22 keeps per-family cursors for definition changes.
- A plain CREATE INDEX on a large table after the freeze: refused by the guard. Shadow (2.3) is used.
- An id-order shadow backfill: 1B random inserts.
- A durable sort for bulk_due: the walk is cheaper.
- Version recomputes that let desktop values go stale while they run, or that rewrite a pending batch through live
  indexes: rejected. D20 keeps reads exact and writes no index pages.
- A snapshot, or an O(n) materialization at start, to make the cursor exact: rejected. D6 is a stated deviation instead.
- The status pair counter as total minus touching-missing: replaced by Σ bands.
- One coordinated migration for this plan and the catalog-size plan: unnecessary while the baseline is edited in place.

**Scan:**

- A durable per-row seen stamp (`file_seen`, a per-pass floor): rejected. SCAN_OPTIMIZATION Phase 3 measured the per-row
  stamp as most of a rescan's cost; the per-scan temp set is kept, and moves to disk (D2, D38).
- A per-root partition of the unpaired set, or anchors on folders: unnecessary, because `path_key` ranges are
  drive-exact and scan pairing reads the scan's own seen set (D9).
- Persisting a journal pass's claims to resume it across a restart: rejected. A restart abandons the pass, which equals a
  stop plus a rescan (3.9).

**Thumbnails:**

- Per-height laps, `todo`, `lost` and a walk cursor for every round (the 2026-11-07 D35): not built. The shipped
  checkpoints make incremental rounds O(changes); paging (4.6) bounds the rest.
- A durability barrier, or synchronous=FULL, for the cache: not needed, because the cache is rebuildable.
- Skipping the decode at cache-full unconditionally: rejected. The memo (Q-T4 a) skips only while the stored rejection
  still matches.
- A snapshot of the cache taken at open: O(entries) at startup. The chunked heal on the writer was chosen.

**PDQ:**

- A per-run heavy bitmap (n/8 bytes of RAM): rejected.
- Query-major resolution: about 2 PB of work.
- The 2026-11-07 session clock (`pdq_batch`, `pend_batch`): moot, since the owed queue makes pairing independent of its
  timing (D25).
- Keeping the whole stored side in RAM: it is today's design, and it fails above 268M pictures.

**USN journal** (kept from 2026-11-07; each still holds unless marked):

- OpenFileById live values as the truth: the walk records directory-index values (§1.3, value truth).
- ~~A links table: changes the walk's semantics.~~ Overturned 2026-11-25: the user adopted the MFT plan's links model for
  the walk too (§1.3), so the walk's semantics change with it, and the journal pass drops its winner rules (3.9).
- Ids in record order: changes the ids.
- Sealing sub-walk entries: they need no scratch.
- Opening every moved entry: moves under a DropSubtree skip the opens.
- Hashing the whole token: a new logon would force a re-probe. The projection is logon-independent instead.
- Dropping every logon-type SID: only the per-session SID (SE_GROUP_LOGON_ID) is left out; well-known logon-type SIDs
  change access, so they are kept.
- The per-folder count check, and polling every Foreign identity: too costly.
- Reading `$UpCase`: not openable unprivileged.
- Base-time link counts: admin-only, or O(n) opens.
- An elevated helper process for scanning: the privileged read is used only when already elevated; the helper is for MFT
  reads and journal sizing.
- One cursor per volume (the MFT plan's `volumes.next_usn`): each root keeps its own cursor, because each root's base walk
  or MFT read ends at its own position; roots with equal cursors share reads (Q-N1).
- `OpenFileById` live values and a `links` table for journal passes (the MFT plan's 2026-11-14 journal-pass design):
  replaced by this plan's 3.9, under the value-truth decision (Q-N1).
- ReturnOnlyOnClose or reason filtering.
- Trusting detachable buses by default: Q-S3 (a) instead.
- Auto-resizing the journal: resizing needs consent (Q-S6).
- Carrying `usn_*` through a rebuild: each NTFS root walks once after a rebuild.
- An unfiltered watch: most records would be the app's own writes.
- ~~Directory identity on `folders` rows: folder rows are created lazily, so they cannot carry it; a separate `usn_dirs`
  mirror holds every directory.~~ Overturned 2026-11-25: with the MFT plan's model every directory has a folder row with
  its identity, and the journal's per-directory state is a side table of it (`folder_journal`, 2.1).
- An in-order DFS walk: changes row ids (R3). (Hard-link winners no longer exist with a row per link.)
- A volume service: §1.3.

**Work lists and grid:**

- A `changed > last` cursor alone: misses deletions and renames. `files_log` is required.

**Qt edge:**

- Owning Flickable inertia: relies on private Qt logic (`qquickflickable.cpp:1150-1170`, `:1498-1531`).
- A ScrollWindow QObject property: rejected in favour of a Rust SlotWindow.
- Model-driven scroll bars for all views.
- Capping the lead L: synchronous slot assignment for viewport ± 1500 px is used.

**Startup:**

- A preparation that blocks the window: replaced by an O(1) preparation plus a progress window (D36).

## 6. Residuals: what is proven impossible to do better

Each residual is a cost that no design in this plan avoids, with the argument for why.

1. **R1. Rebuild disk ≥ size(old) + size(new), about 2 TB at 1B.** The old file is kept whole, and the new one holds every
   carried row, so two copies coexist at the swap. Only an in-place migration avoids this (RebuildTable, 2.3, after the
   freeze).
2. **R2. Rebuild time is Θ(n log n): 6-12 h on NVMe. A resume loses at most one bulk statement (one CREATE INDEX, at most
   about 30 min).** Every row is rewritten (Ω(n)); a B-tree over unsorted keys needs Ω(n log n) comparisons; SQLite fills an
   index only through one CREATE INDEX statement.
3. **R3. Huge-folder frame scratch is Θ(pending subfolders), only above 16K subfolders, about 8-20 B per frame.** The walk
   pops frames LIFO (`stack.pop` and `stack.push` in the walk loop, `crates/scan/src/lib.rs`), so it visits subfolders in
   reverse listing order. Reversing an m-item one-pass stream with M words needs p·M ≥ m passes, and an in-order DFS would
   change row ids.
4. **R4. Moot** (the borders report went with the command line).
5. **R5. A missing toggle costs Θ(degree) reads, and `pdq_near` and band upkeep cost Θ(degree).** A toggle changes the
   presence of every incident pair; exact O(1)-report counters must move by that many.
6. **R6. CREATE INDEX holds one page buffer per PMA: index/32768, which is 1.6 MiB at 1B.** vdbesort opens every leaf PMA
   reader at merge time; no fixed PMA size makes the heap constant.
7. **R7. Moot** (the borders CSV).
8. **R8. Cold-cache removal and mark_missing cost Ω(uncorrelated indexes − 1) random reads per row: 3-80 h on NVMe,
   possibly months on a cold HDD.** SQLite B-trees have no batched update, and any order is sequential in at most one
   index. The parked lanes of a removal's volumes wait that long.
9. **R9. Moot.** The cross-OS lease concern (Windows and WSL sharing one catalog) went with the command line; the Linux
   container uses its own catalog.
10. **R-SCAN. A full walk reads every listed entry, plus one catalog identity probe per entry: 200-400 GB, 1-4 h at 1B.**
    It still runs for the fallback cases of 3.9. Without a filesystem change log, a change can be any entry, so every
    entry must be listed and compared with its stored state.
11. **R-PAIR. First-scan pairing costs about 20-25B B-tree probes.** Each fresh row needs its tier lookups, and the tiers'
    candidate sets have no common order (identity, folder and name, file_ref).
12. **R-PDQ. To be restated by the 5.0 design round.** Two bounds stand whatever the layout: a subquadratic exact Hamming
    join at d ≈ 8.5 log₂ n would refute SETH (Williams 2005; Alman and Williams 2015); and dense clusters cost Θ(k²) CPU,
    since the band counts determine every pair's band.
13. **R-TH1. A height with no checkpoint walks N rows once (14-28 h at 1B), resumable.** This covers a new height, a start
    over and an eviction reset. Nothing records which files lack a current entry at that height.
14. **R-TH2. The progress total is unknown while a first-pass walk is open.** A walk's owed count is a join across two
    databases.
15. **R-UP1. A version bump writes Ω(n) flag pages, about 1-2 h sequentially.** The flag lives in every row; a
    version-relative flag would need a permanent 30-40 GB index.
16. **R-RD. A converging read's final iteration is bounded by the rows changed during the previous iteration.**
17. **R-QT1. Flickable contentY drifts between idle frames. The double keeps 2^-11 px up to 2^40 px.** Reaching that takes
    about 4.6 years of continuous maximum-velocity flicking.
18. **R-QT2. Scrollbar resolution is about n/h items per bar pixel.** Exact reach is kept through the wheel, keys and
    reveal.
19. **R-QT3. Delegates per row are set by geometry: up to about 171K cells at 7680 px, T 60, 1:20 strips.** Only a product
    change to `clamp_aspect` or the minimum row height would lower it.
20. **R-Q1. Unindexed predicates cost O(scope) per query edit; every filter costs Θ(matches).** Range indexes are opt-in
    (7.17).
21. **R-RO. The ordered-sink spill holds up to the full output size in the extreme.** A row cannot be emitted before every
    earlier row exists.
22. **R-USN1. A journal pass lists a dirty folder of m entries in full (O(m)) in four cases:** a changed file denies
    FILE_READ_ATTRIBUTES to OpenFileById; a probe contradicts `open_by_id`; more than one intent-carrying name is
    non-ASCII; or m ≤ max(4096, 64k).
23. **R-USN2. Changes beyond the journal's MaximumSize between two passes, or during a base walk, force a full walk.**
    Resizing needs an administrator; the app offers a consented resize (Q-S6).
24. **R-USN3. A change of the token's access projection costs an O(dirs) access re-probe (about 5-25 min at 10-50M
    directories), instead of a walk.**
25. **R-USN4. Each pass costs O(volume records since the cursor) to classify, even for records outside the library.** The
    journal is per volume.

## 7. Open questions and accepted deviations

**Decided** (recorded so that later steps can cite them):

- PDQ radius stays exactly 31.
- **Q3:** missing files are ignored when readers count pairs (`present_pdq_pairs`), while pairs form against every stored
  picture (today's rule, D25).
- **Q4:** scratch lives on the catalog volume, with typed out-of-space errors.
- **Q6:** batched commits are accepted.
- **Q10:** indeterminate progress while an unsized step runs. Built.
- Exact global grid layout, provisional briefly while a list builds (7.4 and 7.6).
- **Q-D:** the deviation set below is accepted as a whole.
- **Q-T4 (a), cache-full memo** (4.6).
- **Q-G1 (a), range-filter indexes:** none by default, opt-in per column, built as online Shadow access paths (7.17).
- **Q-S1 (b), NTFS change journal:** step 3.9, with D37 and D38, the walk fallback, and deviations J-a to J-f.
- **Q-S6, journal sizing:** the app offers a resize with consent, through the per-session helper. It never resizes
  silently.
- **Memory:** no caps (§1.2). Q5 (budgets) and Q-M1 (exposing them) are moot.
- **Q-N1 (2026-11-25), the NTFS journal pass:** this plan's 3.9 is the one journal pass, under the value-truth decision.
  It adopts the MFT plan's own-write bracket, data-change reasons and `content_gen`, and an MFT read seeds the per-root
  cursors. `docs/plans/NTFS_MFT_SCAN_PLAN.md` keeps the full MFT read, the helper and `ResizeJournal`; its journal-pass
  section points here.
- **Q-N2 (2026-11-25), duplicate groups: deferred until `docs/plans/PLAN.md` M3 (duplicate review) is planned in
  detail.** Groups are a planned feature (M3's group view, keeper and keep / quarantine / "not a duplicate";
  `docs/plans/IMAGE_QUALITY_PLAN.md` phase 4), but PLAN.md groups by multicut over verified, signed tier 0-3 edges, with
  union-find only for sharding, while the 2026-11-07 Phase 6 built connected components over tier 0-1 pairs for the
  removed `dups` report. Phase 6's groups are designed with M3, after 5.0, and under the same rule: no structure holds a
  whole component's edges in RAM. The Duplicates filter (6.5) is not deferred.
- **Q-N3 (2026-11-25), order against the catalog-size plan:** `docs/plans/CATALOG_SIZE_PLAN.md`'s Phase 2 lands first,
  and this plan's `order_key` statements are written in its terms. Then the MFT plan's step 1 (links, volumes,
  volume-relative paths, a folder row per directory; decided the same day, §1.3), then this plan's Phase 2.

**Accepted deviations (Q-D).**

- **D6, cursor subset:** no failure row for a file deleted mid-pass. A file modified mid-pass waits for the next pass.
- **Scan:**
  - partial recording on a relist failure, or a stop inside a huge folder;
  - out of space is typed;
  - **(J-a)** A hard-linked or held-open media file keeps the values last recorded when its directory entry in the library
    either (i) was already stale at its root's last full walk (changed through a link that no walk of the root lists, or
    written while open, before that walk), or (ii) belongs to a file that denies FILE_READ_ATTRIBUTES to OpenFileById.
    This lasts until the entry is refreshed and its folder is next relisted (which then queues a verification walk), or
    until the next full walk.
  - **(J-b)** Writes that bypass the NTFS change journal are seen only when their folder is next relisted, or at the next
    full walk (Q-S5, 30 days by default): another OS's NTFS driver on any disk, offline edits, chkdsk /f repairs and
    self-healing, an image restore that keeps the journal id and position. Detachable buses always walk unless opted in.
  - **(J-c)** An entry modified while a journal pass runs is recorded with the values read; the next pass corrects it.
  - **(J-d)** A folder with no journal record since the cursor, which would now fail to open or list for a non-access
    reason, is treated as readable as last listed until it is next dirty, re-probed, or walked.
  - **(J-e)** An ACE that names a logon-session SID on a library folder is re-evaluated only when the folder or its parent
    has a journal record, or at the next full walk.
  - **(J-f, kept only if the real-NTFS journal test shows that NTFS writes no record at a POSIX unlink before the last
    close)** A file POSIX-deleted while another process holds it open stays present until that handle closes.
- **Rebuild:** (a) to (j), in 2.2.
- **Upgrade:** ordering of ungated passes; the status line shows progress; a zone restart (non-Windows only).
- **Thumbnails:** `TotalsFixed` after open; a dropped PutOne under a stall; cache-full memo: a source that changed on disk
  without a catalog or cache change is not re-tried until the next such change.
- **Duplicates views** reflect a status flip when its trigger commits.
- **GUI edits** are optimistic, and revert on failure.
- **Write-back:** offline parking cadence; PROBE_WAIT.
- ~~D34's as-of counts during a removal; the removal stamp order; `file_by_path` ties; the borders CLI deviations; the
  thumbnail steal and once-per-lap retry; the PDQ as-of note; concurrent CLI imports; legacy re-pairing.~~ Moot: the
  designs they qualified were replaced or removed.

**Open questions.** The recommended option applies unless the user says otherwise.

1. **Q-R1. Rebuild indexes.** (a) Regenerate every index from the rows, as today; this heals a corrupt index
   (recommended). (b) Copy unchanged tables' indexes through xfer: about 2× faster, but carries corruption over.
2. **Q-W1.** WAL_CEILING of 512 MiB (recommended), or larger.
3. **Q-W2.** PROBE_WAIT of 5 s (recommended), or longer.
4. **Q-S2. `unpaired_with_data` count.** (a) O(that root's unpaired rows), through `files_unpaired` (recommended). (b) A
   per-root counter, with triggers on every child table.
5. **Q-S3. Detachable volumes.** (a) Volumes whose bus type, hotplug or removable flags mark them detachable always walk,
   unless listed in `gallery.json` `journal.trust_detachable` (recommended). (b) Trust the journal on them by default.
6. **Q-S4. Cost crossover.** (a) When the estimated pass cost exceeds 0.8 × the walk's estimate, walk; results are
   identical (recommended). (b) Always use the journal when the ladder passes.
7. **Q-S5. Verification walks.** (a) A full walk per root every `journal.verify_days` (default 30, 0 = never), also queued
   at once by an unexplained difference in a relisted folder (recommended). (b) Never.
8. **Q-S7. Journal watch in the app.** (a) Watch each capable volume, and queue a Scans request 5 s after a relevant
   record; the watch never writes and releases the volume on lock or removal (recommended). (b) No watch.
9. **Q-G2.** Front-coded order keys for HDDs: not adopted (recommended). SQLite index pages cannot front-code.
10. **Q-G3.** Live mode ("N+", scrollbar off): not needed, since ProvisionalGeometry covers it (recommended: no).
11. **Q-G4.** Persist the current grid run across restarts (recommended: yes, as 7.5 has it).
12. **Q-SC1.** Where the weekly 10M and occasional 100M scale runs go: a self-hosted runner (recommended), or a local
    disk. Re-check the free space before choosing; a 1B library does not fit on D:.

## 8. Verification

**Harness:** `crates/scale/tests/scale.rs`, `#[ignore]`, `--test-threads=1`, driven through `pg_driver::Headless`.

- Catalogs at N and 10N come from a child `pg-scale synth`, cached by schema hash: smoke 100k and 1M, full 1M and 10M,
  manual 10M and 100M. They span two drives, so two lanes run at once.
- A counting global allocator (`crates/testkit/src/alloc.rs`); `pg-catalog/instrumented` (steps, authorizer,
  read-transaction spans); `peak_private_bytes` for the process.
- Samplers every 50 ms: the temp dir; `<catalog>.scratch/*` per owner; the PDQ store; grid runs; the WAL, with the WAL
  hook's frame high-water and WAL bytes per row.
- Journal rows run on the fake volume over synthesized catalogs. The real-NTFS and VHD suites are in `just test` and
  `just usn-vhd`.

**Assertion classes** (between N and 10N):

- **RAM:** Rust peak Δ ≤ max(4 MiB, 5%); the SQLite heap highwater flat; the process peak flat. No absolute ceiling: the
  measured peak is recorded beside the §2 estimate.
- **STEPS-N:** VM steps ratio ≤ 15 (O(n) passes).
- **STEPS-Δ:** O(changes).
- **STEPS-1:** flat (O(1), O(show), O(window), O(page)).
- **DISK:** the named sampler ≤ the stated bound.
- **WAL:** per-commit frames ≤ F + 64·P; live WAL ≤ ceiling + batch.
- **RTX:** max read transaction ≤ 2 s.

**Matrix: every §2 row has a scenario and an assertion.**

- **Scan walk.** First scan of one N-file folder, of N subfolders, of a balanced tree; a no-change rescan; a 10N-subfolder
  folder; validate and relist injection; a stop inside a listing; a first walk with mirror upkeep. RAM; STEPS-N; WAL;
  DISK (frames ≤ 20 B × N + one block; temp seen and identity sets; folder rows and `folder_journal` per directory).
- **Journal rescan.** A no-change pass at N and 10N; 10^4 and 10^6 records over N and 10N catalogs; a renamed folder of
  N/100 and of 0.5·N files; an R-A re-probe; a multi-root shared read; a stop in R and W, then the rerun. RAM; STEPS-1 for
  a no-change pass while the recurring sets are fixed; STEPS-Δ otherwise; WAL; RTX; DISK; equal to `--walk`.
- **Scan identity pairing.** Two roots; two drives with mirrored layouts; a drive root; save-overs; rebinds; ambiguous;
  moved; deleted; a renumbered volume; a stop between pages, then resume. RAM; STEPS (rows visited ≤ the scan's seen
  rows); 0 write transactions when nothing pairs; resume equals uninterrupted.
- **mark_missing and removal.** Remove an N-file single-folder root, with a same-named root on a second drive; delete N
  in one folder, then rescan; warm and cold timing; a scan request on another drive during a removal. RAM; WAL; STEPS-1
  per page; the other drive's lane runs on; time per row recorded (R8).
- **Work lists.** Import with a stub analyzer, hashes, placeholders and borders after a detector bump; a stop at every
  fetch. RAM; examined ≤ n + stops × 4096; done == total.
- **Version bump.** Bump import, borders, zone, pairing and key version at N and 10N, stopped at every batch; a reader
  during Pass A. Preparation STEPS-1; Pass A no index writes; RTX; exact reads equal the oracle.
- **Status and writer counts.** `pg-driver status`; a writer drain of 10^5. STEPS-1.
- **Write-back queue.** Q failed rows; 10^5 parked on an absent root. `next_ready` STEPS-1; at most one probe per 30 s.
- **PDQ.** Scenarios and assertions from the 5.0 design round; at least: a fixed 10k new pictures against N and 10N stores,
  a lone picture, planted clusters, the cap crossed (more than 2^28 pictures at manual size). RAM flat in N.
- **Duplicates filter.** A k-member, mostly-missing run. Filter cost flat in k.
- **Thumbnail pass.** A first pass, stopped every page; an eviction reset; 1% persistent failures; an offline volume
  holding 20%; cache-full × 20 (memo); a mass seq bump; two heights. STEPS-Δ or STEPS-1 per round; RAM; DISK.
- **Thumbnail cache upkeep.** Heal due at E and 10E; evict at the cap with the kept height dominating. Open STEPS-1; RTX;
  evict steps per victim flat.
- **Grid.** Name and sorted builds; deltas of 1k and 3M; window scroll at 2^31 + 2^20 + 7 (periodic); a Duplicates-filtered
  refresh; the top-K preview. RAM (client ≤ 11 MiB + window bound; build transient flat); STEPS-Δ for refresh; DISK runs;
  RTX.
- **Name filter.** Needles of 1, 2 and 5 characters, rare and common. STEPS ∝ candidates; scan fallback at scope/8.
- **Range-filter indexes.** Opt in size and taken at N and 10N; build under concurrent writes; a taken step; a selective
  and an unselective range; opt-out. Results equal the scan; STEPS ∝ candidates; DISK ≤ 30 B per row per column;
  `set_rating` ≤ 150 ms during the build.
- **Folder tree.** Build plus slot scroll at 3e9 rows. RAM ≤ 4 MiB + slots; merged counts exact.
- **Facets.** With concurrent writes. RAM ≤ 70 MiB; DISK log.
- **Crops.** A crop list of N. RAM ≤ 2 MiB.
- **Folder picker.** 3×10^6 synthetic names. RAM ≤ 20 MiB; order equals the oracle.
- **Qt/QML edge.** `huge-list` and `huge-keys` assert dumps; window laws. Exact strings; slots ≤ S.
- **Catalog rebuild.** A forced rebuild at N and 10N; kills at random points. RAM flat; DISK temp ≤ 2.2 × the largest
  index; no WAL; resume identical.
- **Post-freeze migration.** A synthetic RebuildTable(files) and Shadow. DISK ≤ start + batch + delta; `set_rating`
  ≤ 150 ms during Shadow.
- **Startup.** A normal open; an open needing a rebuild. Open STEPS-1; window after 300 ms; cancel ≤ 100 ms.
- **SQLite temp and WAL.** Every scenario. WAL; RTX; temp per owner.
- **Whole process.** All passes on two lanes in one process, including a journal pass and the watch. RAM (flat); the
  measured peak recorded beside the §2 estimate.

**Self-checks** (feature `self-check`): an injected O(n) Vec fails the Rust flatness check; a single-transaction 10N update
violates WAL; a 5 s reader violates RTX; an injected per-sub-walk-entry intent fails the journal DISK bound.

**Commands** (once, at the end):

```text
just build && just lint && just test   # unit, oracle, proptest, plan guards, qt-edge lint, golden ListView, journal oracle and faults
just lint-qml                          # qmlcachegen: no fallbacks
just plans                             # ledger: tags, plans, temps, fan-out, total order, read-tx census, allowlist
just shots                             # includes $Asserts (huge-list, huge-keys), failing on mismatch
just scale-smoke                       # 100k and 1M (also in Windows CI)
just scale                             # 1M and 10M, weekly
just usn-vhd                           # elevated: journal delete, recreate, wrap, rollback and clone; volume lock
just perf --update                     # the instruction-count gate (callgrind, Linux container)
just scale-dhat <pass> <size>
```

`just plans`, `just scale-smoke`, `just scale`, `just usn-vhd` and `just scale-dhat` are new (Phase 8).

**Oracles kept in-tree:**

- **Scan identity pairing:** `reference_pair`, `reference_mark_missing` and the reference walk.
- **Journal rescan:** `pg-driver run --passes scans --walk` at the same checkpoints (A and B catalogs; all-J, mixed and
  abandoned schedules), on the fake volume (with parent ambiguity) and on real NTFS.
- **Work lists:** the old `pending_*` SQL.
- **Upgrade:** today's `set_*_version` bodies, `owe_pairs_again`, `recompute_keys`, `mark_unheld_missing`.
- **Status:** `old_status`.
- **Write-back:** the old `NEXT_READY`.
- **Rebuild:** the `cfg(test)` `copy_tables`.
- **Thumbnails:** today's `pending::work` merge, and the old evict ORDER BY.
- **PDQ:** brute force, and today's in-RAM join.
- **Duplicates filter:** the EXISTS forms.
- **Grid:** `list_grid_reference` (today's `list_grid`).
- **Range-filter indexes:** the unindexed scan of the same query.
- **Tree:** today's `folder_tree`.
- **Facets:** the per-facet statements.
- **ListView:** the golden table, reproduced by a real ListView.

## 9. Dropped since 2026-11-07

Each was in the 2026-11-07 plan and is gone, with the reason:

- **The command line's work:** `borders --csv` and `--import-review` (4.8, R4, R7, Q-B1, Q-B2), `dups` and its waits,
  `status --wait`, `import --again` (3.6), `gallery journal`, the thumbnail steal (Q-T1), the CLI matrix variant. The
  command line was removed (2026-11-24).
- **Every memory cap:** the Budget shares (D14), the W reservation, role caches as a budget, `hard_limit`, `run_in_job`,
  the ABS assertion, Q5 and Q-M1. User decision, 2026-11-25.
- **The one coordinated BASELINE edit and freeze** (D1): the baseline is edited in place until the first release.
- **Anchors and `unpaired_anchor`** (D9, the `files_unpaired_*` indexes, the anchor lints): `path_key` is drive-exact, and
  scan pairing reads the scan's own seen set.
- **`file_seen`, `scans.floor` and the sentinel scan** (D38): the per-scan seen set replaced them.
- **The reader mask for pending removals** (D34, its status corrections, as-of counts and deviations): removal hides files
  by `missing` writes while its lanes are parked.
- **`work_leases`, the counter heartbeat, the fence token and the lease helper** (D16, 1.9, R9): leases stay as scan leases.
- **The PDQ session clock** (`pdq_batch`, `pend_batch`, `formed`, plane P, the legacy re-pair, Q-P3): the owed queue.
- **The 2026-11-07 tier-1 layout and its figures** (5.2-5.10, R-PDQ1 to R-PDQ5, Q-P1, Q-P2): to be re-derived (5.0).
- **The thumbnail laps, `todo`, `lost`, walk cursor, compaction and lease** (D35, 4.4-4.6 as designed, R-TH3, R-TH4,
  Q-T2, Q-T3, Q-T5, Q-T6): the checkpoints.
- **ConcurrentReader and the segment stack** (1.6): the adaptive readers, and the borders CSV.
- **The outbox table** (3.5 as designed): an in-process temp spill.
- **`pair_ambiguous`, `scan_skipped` and `scan_cleanup` tables:** today's `ambiguous` set (spilled when large) and
  `folder_failures`.
- **The `dups` groups report** (6.4): the command line. The rest of duplicate groups (6.1-6.3, 6.6-6.8) is deferred until M3 is planned (Q-N2).

## Critical files for implementation

- `crates/catalog/src/baseline.sql`
- `crates/catalog/src/core.rs`, `schema.rs`, `rebuild.rs`, `lifecycle.rs`, `work.rs`, `roots.rs`, `pair.rs`, `removal.rs`
- `crates/jobs/src/pipeline.rs`
- `crates/scan/src/lib.rs` and `journal.rs` (new)
- `crates/platform/src/usn.rs` (new)
- `crates/thumbs/src/pending.rs`, `crates/thumbcache/src/writer.rs`
- `crates/import/src/pairs.rs`, `crates/dedup/src/join.rs`
- `crates/client/src/grid.rs`, `crates/catalog/src/browse/read/grid.rs`
- `crates/library/src/lane.rs`, `coordinator.rs`
