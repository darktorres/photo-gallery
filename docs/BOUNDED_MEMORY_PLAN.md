# Bounded-memory catalog at 1B files: one implementation plan (round 3)

## 1. Context

A code-only audit found that almost every library-wide pass holds its whole working set in RAM. At 1B files that set is about 0.9-1.2 TB. The PDQ index alone is 864-928 GB, and the work lists, grid lists, scan sets and duplicate maps each add roughly 100 GB. On top of that, SQLite runs with `temp_store=MEMORY` and 14 caches of 256 MiB.

Round 1 merged six reviewed area designs into one plan. Round 2 did four things:

- closed every residual and deferral left by round 1, through four gap designs (folder residuals, thumbnail eviction, PDQ structure, Qt cap);
- audited the crates nobody had audited (write/start path, shell/CLI);
- had two adversarial reviews of the merged plan;
- had every design reviewed and revised again.

Round 3 did two things:

- designed, reviewed and revised the NTFS USN-journal incremental rescan, because the user chose Q-S1 (b). It is now step 3.9.
- ran an adversarial review of the round-2 synthesizer's own decisions. It found two blockers: `pend_batch` was never cleared, and an order-key range is not a root. Both are fixed throughout.

This document replaces the round-2 plan. It keeps the same structure and folds everything in.

**Rules held throughout:**

- RAM is a fixed configured budget, independent of library size.
- Results are identical: pairs, groups, grid order, filters, counts and CLI output. Where they are not, the deviation is stated precisely: in the step, and collected in §7 Q-D.
- Disk and time are stated at 1B.
- Schema changes go through the migration system. That means one BASELINE edit now, then a freeze (D1).
- Implementation is completed in full, then built once at the end. Commits go on the checked-out branch, with no new branches.

Paths are relative to `D:/GitHub/photo-gallery/`. The user is editing the tree, so cited lines may drift by a few; find them by symbol.

**Decisions already taken by the user (2026-11-07):**

- PDQ radius stays exactly 31.
- One BASELINE schema edit, then freeze.
- Batched commits are accepted.
- Exact global grid layout, provisional briefly while a run builds.
- The Q-D deviation set is accepted.
- Thumbnail cache-full memo: Q-T4 (a).
- Range-filter indexes are opt-in per column: Q-G1 (a), step 7.17.
- The NTFS USN-journal incremental rescan is included: Q-S1 (b), step 3.9.
- **Value truth across scan sources:** every rung treats directory-index values (size, mtime and creation time, as a listing returns them) as the truth: the walk, 3.9, the per-session MFT helper, and the service if one is added.
  - MFT rungs therefore read `$INDEX_ROOT` and `$INDEX_ALLOCATION` raw (an LCN-sorted sweep with fixups) instead of `$STANDARD_INFORMATION` and `$DATA`.
  - MFT rungs also filter folders by whether the client's token has FILE_LIST_DIRECTORY access, so their output equals the walk's.
  - 3.9's `usn_stale` and deviation J-a stay.
  - `docs/NTFS_MFT_SCAN_PLAN.md` must be amended to match: its helper currently takes MFT-record values and sends every folder.
- **No volume service.** The ladder is: the per-session helper (MFT read where elevation is silent), then 3.9, then the walk. The service's main gain, following the journal while the app is closed, is mostly covered by sizing the journal with consent (Q-S6, decided as journal sizing through the helper, step 3.9 USN6). The rest of its gain is for non-admin and multi-user installs, which do not exist. Its costs:
  - an installer, which does not exist yet;
  - a SYSTEM attack surface;
  - pipe-server authentication;
  - emulating per-folder AccessCheck;
  - a durable dirty-ref store.

  Revisit this if the app ever ships with an installer to non-admin users. The reviewer's requirement list is in `C:\Users\Torres\.claude\projects\D--GitHub-photo-gallery\c2642be2-7931-4269-841c-6ecbe5ba1f3e\subagents\workflows\wf_67748169-c22\journal.jsonl`.

Every other open question takes the recommended option unless the user says otherwise.

**Phase 0 is the user's.** The in-flight S-17 Unit E work in the working tree is not part of this plan, and it will not be committed on the user's behalf.

**Source keys.** These name the design text each step comes from.

Round-1 sources, in `C:\Users\Torres\.claude\projects\D--GitHub-photo-gallery\c2642be2-7931-4269-841c-6ecbe5ba1f3e\subagents\workflows\wf_cba81dff-055\journal.jsonl`:

- `[r1 scan Sn]`, `[r1 grid Sn]`, `[r1 pdq Sn]`, `[r1 dups Sn]`, `[r1 wl Sn]`, `[r1 T n]`

Round-2 sources, in the agent transcripts under `...\subagents\workflows\wf_8cdcde0a-48d\`:

- `[F1-F6]`: gap folder-residuals
- `[TE1-TE6]`: gap thumb-eviction
- `[G1-G13]`: gap PDQ structure
- `[QT1-QT9]`: gap Qt cap. Its steps were named Q1-Q9; they are renamed here so they do not collide with the open questions.
- `[U1-U13]`: audit write/start
- `[S1-S17]`: audit shell/CLI
- `[PRF n]`: plan review, foundations, issue n
- `[PRX n]`: plan review, features, issue n

Round-3 sources, in `C:\Users\Torres\AppData\Local\Temp\claude\2c16f13ff155207f\c2642be2-7931-4269-841c-6ecbe5ba1f3e\scratchpad\r3.json`:

- `[USN1-USN8]`: the revised USN-journal design (`usn.revised.design.steps`)
- `[UR n]`: the USN design review, issue n (1-based)
- `[RR n]`: the review of the round-2 resolutions, issue n (1-based)
- `[RE n]`: that review's plan edit n (1-based)

## 2. Target bounds at 1B

All values are for 1B files unless noted. "NVMe" means a warm or cold NVMe catalog volume.

| Stage | RAM today | RAM after | Disk after | Time after |
|---|---|---|---|---|
| Scan walk | 25-38 GB seen set, plus 115-145 GB per giant folder | about 3 MB Rust plus the Writer cache | `file_seen` +14 GB (files shrinks about 10 GB). Entries need no scratch. Frames spill only above 16K pending subfolders, at 8-20 B per frame. On NTFS roots the mirror `usn_dirs` takes about 280 B per directory (2.8-14 GB at 10-50M directories) and is written on change only; the first walk adds about 3% more inserts. | A no-change rescan walk writes about 14 GB of WAL (was 200-400 GB). Reads: one identity probe per listed entry, 200-400 GB, about 1-4 h on NVMe (R-SCAN). A folder with more than 16K entries is listed twice, about 5-15 min per pass of 1B entries. **Full walks now run only for:** a first scan (including the first after a rebuild), a 3.9 fallback, a Q-S5 verification walk, a requested `scan --walk` or Full rescan, Linux/WSL roots, network shares, non-NTFS volumes and detachable volumes (§6 R-SCAN). |
| Journal rescan (NTFS, 3.9) | a full walk (R-SCAN) | ≤ about 140 MiB from the Budget background share, constant in n: 1 MiB read buffer, ≤ 40 MiB record groups, ≤ 24 MiB dirty set, ≤ 24 MiB intent runs, 16 MiB mirror cache, and the 3.1 bounds for in-place sub-walks | `usn_dirs` as above; `usn_entries` about 60 B per exception. Scratch: about 60 B per journal record, plus 150 B per dirty pair or non-subtree intent, plus 40 B per affected row or scope. Nothing per sub-walk entry. Deleted at W3 and at finish. | O(volume records + dirty-folder entries + sub-walk entries + affected rows + state-1 + stale + woken). No change ≤ 0.2 s. 10K changes 1-3 s. A 1M-record day about 45-100 s. A 10M-file import about 20-35 min. A renamed folder costs the walk's per-entry cost for its subtree. An access re-probe takes about 5-25 min at 10-50M directories. A 4 GiB journal holds about 30 days at 1M records/day. |
| Pairing | 370-410 GB + 8 + 10-20 GB | about 0.5 MB, plus the root's fresh-id spill (16 B per row) | `files_unpaired_*` is transient | First scan: 4 sequential passes, 0.8-1.6 TB of reads, plus about 20-25B B-tree probes (about 6-12 CPU-hours). A rescan with nothing new reads O(that root's unpaired rows) through the (anchor, key range) (D9), and does 0 write transactions. A stop takes effect between pages. |
| mark_missing / removal / `import --again` | 8-8.6 GB + O(root) FIFO + journal | about 0.4 MB | WAL ≤ about 26 MiB per commit | Removal takes 3-80 h on NVMe, in the background in ≤ 60 s slices, and is resumable in O(depth). It is hidden from readers immediately, path-exact (D34). Cold HDD is far slower (R8). |
| Import work list (+retry) | 94-124 GB (+90 GB retry sort) | ≤ 256 KiB page + 8 MiB FolderCache | Retry list: 80 B per failure, TEMP, paged | About 1M fetches at about 1 ms each. |
| Version bump (import, borders, taken zone) | One transaction: 200-300 GB WAL, -shm 0.5 GB, no window for hours | O(1) | No sidecar, no sort | `prepare` takes under 5 ms. Pass A runs 1-2 h sequentially in the background, and reads stay exact throughout (U1). |
| Status, writer counts, pass totals | count(*) taking O(n+P): 30-90 min, or 6-28 h for the pairs term | O(1) | One `counts` row | About 1 ms per call. A writer drain is O(Q). |
| Write-back queue | O(Q²) per drain, offline serialization | O(1) | A few B per job | `next_ready` is O(10 log Q). Offline: at most 16 root probes plus 16 folder probes per 30 s, off the drain thread. |
| PDQ tier 1 and pairing | 864-928 GB (+320 GB reserve) | ≤ W + 8.4 MB in the W holder (dhat Rust heap ≤ W + 2 MiB). W = clamp(RAM/32, 0.5, 4 GiB), so 2 GiB at 64 GB of RAM. W is one process-wide reservation. | 2.14 KB per light file, about 18 KB per heavy file, rows ≤ 512 per light file. That is 2.1 TB with no heavy files (150 GB at 70M), plus a 146 GB merge transient. Re-import peak about 4.4 TB. | First import: about 26 core-days of CPU, plus about 360 ns per heavy-heavy near-copy pair, plus about 370 TB of sequential I/O (+30 TB at 1% heavy). Pair lag ≤ about 70 min (1B fresh), ≤ about 5 min (70M). One image: 2-6 s on NVMe, about 21 s on SATA; on HDD a sweep (G5). |
| Borders list | 94 GB | about 1 MB + FolderCache | `files_borders_due` holds only flag-1 rows; a detector bump writes flag 2 with no index (U2) | Pass A 1-2 h, then the walk stream. |
| `gallery borders --csv` | 450-850 GB | about 0.3 GB + the pipeline | Scratch only for a directory above the sort budget. `<csv>.partial` up to the new CSV's size (about 150 GB) while the run is in progress (R7). | Two listings (15-60 min each, warm) against about 1B decodes. |
| `gallery borders --import-review` | 300-450 GB, plus O(entries × folder) lookups | ExtSort budget | About 100 GB of runs for 1B entries | Sort 20-40 min, plus 1B O(log n) probes, plus batched commits. |
| Thumbnail pass | 118-236 GB + 200 GB sort | about 208 MiB fixed caches + under 15 MiB Rust | State ≤ B members × about 90 B, about 5.6 GB worst (94 MB at 189K files). `lost` ≤ about 1.5% of the cap. | Eviction costs O(evicted). A cache-full round costs O(new events) + 1 memo probe. A first pass per height reads N rows (14-28 h), resumable across any number of preemptions. |
| Thumbnail cache upkeep | heal O(entries) on the GUI thread; evict walks every entry | O(heights) | `height_totals` table, `entries_height_lru` | Heal: about 10-30 s at 130M entries, chunked, in the background. Evict: O(H log E + victims). |
| Dups report and maintainer | 25-80 GB + 16 GB + 50 GB sorter | report O(show); maintainer ≤ about 300 MiB + W (heavy peel) | Derived tables 47 GB worst; `hash_runs` 6-60 GB; `pdq_near` 2-20 GB; `dup_log` ≤ CAP (about 1 GB) | Build 1-6 h, resumable. The Duplicates filter costs O(log n) per row. |
| Grid list / sorted list / refresh | 56 / 108-128 / 112 GB | about 11 MiB client + window ≤ 9.6 MB worst; build ≤ 330 MiB transient (256 MiB interactive share + 74 MiB) | Run 80 GB (name order) / 96 GB (sorted), ≤ about 300 GB worst transient | A cold build takes 20-30 min warm (hours cold) in the background, with a live top-K preview. A refresh, Duplicates-filtered views included, costs O(changes × log n). |
| Name filter | O(scope) scan per edit | the build's bounds + the Names role (16 MiB) | `names.sqlite` 30-80 GB | O(\|candidates\| + \|lag\| + sort) for every needle length. |
| Range-filter indexes (opt-in, 7.17) | O(scope) scan per edit | the build's bounds; Shadow build within the background share | 20-30 GB per opted-in column | Build 1-2 h online per column. A selective query costs O(candidates + sort). Opt-out is one DROP TABLE. |
| Folder tree | 2.3 GB + 0.4 GB + 170 MB | under 4 MiB, + about 175 KiB of slots | 1.5 GB of per-generation files at 10M folders | Build 10-30 s. |
| Facets | 50-80 GB GROUP BY sorter | under 70 MiB | 12 GB contribution log, transient | Chunked. |
| Crops / changes_since | 8 GB / 8 GB | under 2 MiB / 96 KiB per page | Crop id list: 8 B per bordered file | Streamed. |
| In-app folder picker | O(folder) on the GUI thread | ≤ 4 MiB + 16 MiB sort | Only for folders whose names total more than 4 MiB | Off the GUI thread. |
| Qt/QML edge | i32 caps; float32 misplacement past about 645K rows | slots ≤ floor((viewH+3000)/min_pitch)+4 | none | Per frame O(slots). |
| Catalog rebuild (mismatch or corruption) | One transaction: WAL about the DB size, 3x disk, days of random index inserts | ≈ 274 MiB + largest index / 32768 SQLite, O(1) Rust | kept + building + sorter ≤ 2 × largest index (R1) | 6-12 h on NVMe, resumable at chunk or index granularity, behind the startup window. Then one walk per NTFS root, because `usn_*` is not carried. |
| Post-freeze migration | n/a | as rebuild | RebuildTable: in place, start size + one batch + layout delta; Shadow: about the index size | RebuildTable: exclusive, 5-9 h for `files`. Shadow: online, 1-2 h. |
| Startup | prepare blocks before any window for hours | O(1) | none | A normal start is unchanged. A rebuild or exclusive migration shows a stoppable progress window. |
| SQLite connections, temp, WAL | 14 × 256 MiB; temp unbounded in RAM | role caches about 0.86 GiB (D3); temp on disk | WAL ≤ 512 MiB ceiling + one batch (≤ 26 MiB); -shm about 1 MiB; temp peaks: Listed tier gone, retry list 80 B × F, CREATE INDEX sorter ≤ 2 × the index, ExtSort runs as per row | No read transaction exceeds 2 s (D31). Crash recovery ≤ 0.5 s on NVMe. |
| **Whole process** | about 1-1.3 TB + unbounded temp | **≤ about 3.6 GiB at defaults (64 GB RAM); about 2.1 GiB minimum; constant in n** | per row | per row |

**Whole-process derivation.** Passes run one at a time in the runner. Caches fill lazily.

- **W** is one process-wide reservation. It is held in turn by the pair worker, the groups heavy peel (G11) and tier-1 merges or bulk build. The bulk build sorts in the background share only.
- **Defaults peak:** role caches 0.86 GiB + max over runner passes of (import pipeline 0.25 + W 2, or groups 0.3 + W 2 [G11 heavy adjacency], or thumbs 0.25, or a journal scan pass 0.14 inside the background share) + interactive sort share 0.25 + grid build beyond its share 0.07 + facets 0.07 + client, window, tree and picker 0.05 + journal watch 0.02 (a 16 MiB LRU plus 64 KiB per volume; its 4 MiB Watch cache is counted in the role caches). That is 0.86 + 2.3 + 0.25 + 0.19 ≈ 3.6 GiB.
- **Minimum peak:** W = 0.5 GiB, giving 0.86 + 0.8 + 0.25 + 0.19 ≈ 2.1 GiB, with the same terms as the defaults. This corrects round 1's "1.5 GiB" ([PRX6]) and round 2's term mismatch ([RR5]).
- Phase 8 asserts PeakPagefileUsage ≤ this sum.

## 3. Cross-area decisions

| # | Topic | Decision | Why / evidence |
|---|---|---|---|
| D1 | Schema route | **One coordinated BASELINE edit (Phase 2), then freeze.** After the freeze, `MIGRATIONS: &[Migration]` with `Migration::{Sql(O(1) DDL), Steps(&[Step])}`. The steps are `Step::RebuildTable` (exclusive, [F6]), `Step::Shadow` (online access path, [U12]), `Step::Upgrade` (online recompute or recount, [U1]/[U3]) and `Step::Sql`. A guard test refuses `CREATE INDEX` on `LARGE_TABLES` in migrations. | `schema.rs:18-24`: MIGRATIONS is empty. Correction from [PRF18]: dropping `analysis.borders_due` alone would need only DROP INDEX plus DROP COLUMN. `files.last_scan` has an FK (`schema.rs:292`), so a 12-step rebuild is needed for it. Three areas also claimed `MIGRATIONS[0]`. |
| D2 | Temp policy | `SQLITE_TEMP_STORE=0` compile flag, plus `temp_store=FILE` in SETTINGS. | Same as round 1. |
| D3 | Connection roles | Every connection has a role, including prepare's, Upgrade, Rebuild phases, Groups, GroupsScratch, Edits, ThumbPass/ThumbStore, the Ui reader, **Names** (`names.sqlite`) and **Watch** (the USN watch's read-only reader). Each pool is per role. | The values are in 1.2. [PRF12], [PRX17], [S11], [RR10]. |
| D4 | `connect()` home | `pg_schema::env`, which also owns `attach_uri` and the hooks. | No dependency cycle. |
| D5 | Work lists | Keyset cursors for import and borders. The thumbnail Listed and Walk tiers are **replaced** by per-height state in the cache file (D35). Materialization survives only in the paged retry list (D31). | [PRF1], [TE]. |
| D6 | Cursor snapshot semantics | A cursor yields **the subset of today's start snapshot that still qualifies when visited and has not been re-stamped** (`seq ≤ S0`). Skipped rows go to the next pass, and `lower_total` keeps `done == total`. This is stated as a deviation (§7 Q-D). It replaces round 1's claim of exactness. | [PRF6]: `store_failure`'s `current()` does not check `missing` (`lib.rs:1286-1289`). Exactness needs either an O(n) start materialization or a snapshot held for the whole pass; both were rejected (D5, D31). |
| D7 | `analysis_removed` | One body: `next_seq`; `pending=1, borders_due=0, seq, changed`. | Same as round 1. |
| D8 | `pending` stamping | Writers set `pending` only. Trigger `files_pending_stamp` (AFTER UPDATE OF pending, WHEN old=0 AND new≠0) stamps `changed`/`seq`. `pend_batch` and the `pdq_heavy_attr` queue belong to `files_pending_on`; clearing and the requeue belong to `files_pending_off` (D25). Upgrade Pass A suppresses **only** the changed/seq stamp, through an in-transaction `counts.bulk` flag. It never suppresses `pend_batch`, its clearing, the requeue or the attr queue. Every CASE test reads `pending <> 0`. | [PRX14] (stamps come from triggers), [U2] (tri-state), [RR1]: two triggers stamped `pend_batch`, and the scope of the bulk suppression was undefined. |
| D9 | Root scope and unpaired index | **A root is the pair (anchor, key range).** Folder keys merge drives: the anchor is not part of the key (`rules.rs:196-215`; test `the_drive_is_not_part_of_a_folder_key`, `:331`). So the key range of `E:\Photos` also covers `D:\Photos`, and `E:\` covers everything. The indexes are `files_unpaired_fresh(unpaired_anchor, order_key) WHERE unpaired=1 AND volume IS NOT NULL` and `files_unpaired_kept(unpaired_anchor, order_key) WHERE unpaired=1 AND volume IS NULL`. **Upkeep:** `files.unpaired_anchor` is `folders.anchor` (an `anchors.id`, immutable per folder) while unpaired=1, and NULL otherwise. It is written in the same row write by every statement that sets `unpaired` or `folder_id`. A scan reads its root's (anchor, key range) into an id-sorted spill. Rows whose folder path fails the exact `under_root!` byte test are dropped, so the set equals today's `folder_id IN under_root!`. Reader masks (D34) use the same key-range prefilter plus path test. | Today's fresh set is root-scoped by `folders.path` (`pair.rs:200-210`, `under_root!` `lib.rs:80-84`). `folders.path` compares bytewise (`under_params`, `lib.rs:99-115`), so the path test keeps case-variant folders out exactly as today. A folder's anchor never changes, so add_root and remove_root never re-key. The upsert's DO UPDATE moves `folder_id` while keeping `unpaired` (`lib.rs:1223-1227`), so the anchor must follow it. [PRF5], [RR2]. |
| D10 | Missing writes | Per-row writes only, enforced by an authorizer allowlist. Removal uses per-row pages with a done-prefix watermark. | [F1], [F5]. |
| D11 | Dups derivation | The dups area design, with identity counters derived from `hash_runs` ([S12]). All counters live in the `counts` row. | [S12], [PRX4]. |
| D12 | Facets | Grid S4 (chunked, contribution log). | Same as round 1. |
| D13 | Import flush | Store only, plus a pairing worker. `pdq_hashes` is deleted. | Same as round 1. |
| D14 | pg-spill | ExtSort, `Budget` with an **interactive 256 MiB share plus a background 256 MiB share**, SpillQueue/SpillStack (mark/truncate), segment stack, PagedFile, codec, ReorderSpill, ConcurrentReader, ScratchRoot with a **reservation ledger**, and Staged. Out of space is a typed error (Q4 policy). The tier-1 bulk build and the journal pass take only the background share. W is a separate single reservation (§2). | [PRX10], [PRX15], [G5], [S4], [F4], [RR5] (G6's 512 MiB conflicted with the split). |
| D15 | Guard toolkit | One toolkit in `pg_schema` (feature `testing`). Phase order is fixed: the toolkit (1.3) comes before keyset (1.5). | [PRF9]. |
| D16 | Leases | One `work_leases` table. Kinds: pair, flush, build, merge, compact, groups, upgrade, read, adhoc, scan. The SQL helper is shared with the thumbnail cache's lease. Staleness is a **heartbeat counter that does not change over the adopter's own monotonic 60 s**, so no clock agreement is needed. **Adoption allocates a new id and directory.** Only immutable checkpointed state carries over: it is copied, or handed over by rename inside `<catalog>.scratch` (the journal pass's sealed runs, [UR20]). A token is checked inside every write transaction. Owners re-verify after a pause and before each file-write unit. Scratch directories are named by lease. | [PRX5], [PRX14]. The cross-OS pause limit is R9. |
| D17 | Runner schedule | **Requested work first:** `[Crops, Scans (resume a pending journal pass, drain outbox, resume pending pairing; journal pass or walk per 3.9), Removals (≤ 60 s slices; hidden by D34, so never urgent)]`. Then `Upgrade`. **Then idle work:** `[Import (+pair worker), PairBacklog (settle units), Borders, Thumbs, Groups, Names, Tier1Maintenance]`, round-robin in ≤ 60 s or one-checkpoint slices. Merges are debt-driven inside the pair worker. A pass returning `UpgradePending` or `Busy{until}` is skipped, and the runner re-wakes itself on the earliest deadline. CLI `status` and `dups` still drive removals first. | [PRF4], [PRX1]. Pairing no longer depends on its timing (D25). [RR7]: `runner.rs:303` restarts the round from the top on `Step::Preempted`, so an unsliced removal ahead of Scans starved requested scans. 7.16's idle pass was missing. |
| D18 | WAL | `journal_size_limit` 64 MiB. WriteBatch commits at a frame budget F = 16 MiB/page (via cacheflush and CACHE_WRITE) and waits when the live WAL (PASSIVE checkpoint `pnLog`) is above WAL_CEILING = 512 MiB. | [F5], [U7]. |
| D19 | Open API | `Catalog::open(path, rules, Role)` everywhere. New errors: `Rebuilding`, `Migrating`, `UpgradePending{steps}`, `RebuildBlocked`, `KeptChanged`, `NoSpace`, `OutOfSpace`. | |
| D20 | Version recomputes | They become Upgrade steps. `prepare` writes the new mark and step keys in O(1). Pass A rewrites the flags in id order with no index writes (tri-state flags). Every reader goes through cursor-relative `pending_now!`/`taken_now!`, so it is exact. Fixed seeded step keys. | [U1], [U2], [PRF2]. Replaces round 1's 2.4 and the [S17] variant. |
| D21 | Temp dir | `<catalog>.scratch/sqlite` (one ScratchRoot name). The directory is created first. On Windows it is aliased with `alias_of_folder`; on Unix the path bytes go to `sqlite3_temp_directory`. | [PRF16]. |
| D22 | Counters | One seeded `counts` row: status, work totals, write queue, band and heavy-band, identity. Each family has its own recount cursor, `2^62` when idle. A definition can change only through a Migration Upgrade recount or a rebuild. Present pairs = Σ bands. | [S1], [U3], [G9]. |
| D23 | Rebuild protocol | Move aside at Start into rollback-journal mode (refused if the file is in use). Manifest plus owner heartbeat. Chunked copy with FKs off. Indexes from exact text. Ordered FK merge. Derived state from sequential merges and ExtSort. Classified errors with corruption salvage. Swap by file identity. A `.rebuilt` note until it is acknowledged. | [F3], [U5], [S7]. |
| D24 | Uniqueness | Every UNIQUE or PK constraint on a large rowid table becomes a named unique index. Every FK child column is indexed. Both are enforced by lints. | [F3], [S2], [S7]. |
| D25 | PDQ pair definition | Missing files are ignored (Q3). **Pending exclusion uses the import-session clock**: `meta.pdq_batch` is bumped at each `import()` start, only after `import_work`'s gate passes and the list is non-empty. `pdq_indexed.batch` is set at store. `files.pend_batch` is set when `pending` leaves 0 (`files_pending_on`). **formed(x newer, y) = NOT(y.pend_batch IS NOT NULL AND y.pend_batch < x.batch)**; NULL means formed, in SQL and in Rust alike. `files_pending_off` (pending nonzero → 0) always sets `pend_batch = NULL`, and queues `pdq_heavy_attr` for a heavy file. It queues a requeue only when a `pdq_indexed` row still exists, so a store (which deletes the row before its REPLACE) queues none. A pending revert without a store re-indexes the file. | Today's clock is the index load at import start (`lib.rs:881-913`, called from `import/src/lib.rs:40` only after the empty-list return). It holds every present, non-pending analysed file, whatever its pending history. This reproduces it exactly and makes pairing results independent of when the pair worker runs. It refines [G1]. [RR1]: without the clearing rule a re-stored file never again pairs as the older endpoint, and G's NULL guard had been dropped. |
| D26 | PDQ storage | Thin sheets (light files) and fat sheets (heavy files, full dihedral). Heavy core with exactly-once emission. Attribute planes (plane P mirrors `files.pend_batch`). Published generations. Cursor-idempotent writer. HEAVY_ENTER = 512. | [G3], [G4], [G8], [G9]. |
| D27 | Duplicates filter | Pure SQL. `hash_runs.present` with `present_xor`, `pdq_near.n` over rows, and `pdq_heavy.near` plus an exact unsettled correction. Duplicate-status flips stamp `files.changed`, so Duplicates views get deltas. | [S12], [G10], [PRX4]. |
| D28 | Ordered sinks | Non-blocking spilling reorder buffer (`Order::Index`). The pipeline carries the item through to the sink on every path. | [S4], [F2]. |
| D29 | Qt numeric edge | Ranks and counts as exact f64 (n < 2^48 by the SQLite file format). File ids as decimal strings. The scene graph sees only viewport-relative px < 2^14. Every queued rank is an `ItemRef`. | [QT1], [QT3]. |
| D30 | View layer | `WindowView.qml` (a plain Flickable plus a non-moving slot layer) over a Rust `SlotWindow`. The ListView behaviours the views relied on are ported from Qt 6.12 and pinned by a golden table. | [QT4]-[QT7]. |
| D31 | Read snapshots | READ_TX_MAX = 2 s for every catalog and cache read, except the rebuild's building file. Long sets are read by converging paged reads, which give an exact snapshot at their final instant. The harness enforces a census. | [U13]. |
| D32 | Gates | `Catalog::gate(Need)` is checked inside the catalog API (import_work, borders_work, crops, groups), never in callers. | [U1]. |
| D33 | GUI writes | No catalog write runs on the GUI thread. An edit worker plus a client overlay is held until the commit stamp is covered. | [S11]. |
| D34 | Pending removals | Readers (grid build and refresh, tree, facets, status) exclude the rows of pending removals: the order-key range as a prefilter, plus the exact `under_root!` path test on the row's folder (D9). Same-named folders on other drives stay visible. Status corrects present and missing exactly. Analyzed, failed, pending and pdq_pairs are as-of with a note, and the Duplicates filter counts twins under the removed root until they are marked (§7 Q-D). CLI `status` and `dups` drive pending removals first, so the CLI stays exact. | [PRF7], [RR2], [RR6]. |
| D35 | Thumbnail pass state | Per-height state in the cache file: gen, sync cursor, walk cursor, laps, exact and range members, trigger-logged `lost`, lease. The cache layout changes once and **starts over**, then freezes with BASELINE. | [TE1]-[TE5]. |
| D36 | Startup | `prepare` is O(1) except for a rebuild or an exclusive RebuildTable migration. Those run on a worker behind a stoppable progress window. | [U6], [S8]. |
| D37 | USN journal rescan | **State-based.** The journal only names dirty folders: record parents, row folders, exception, stale and link parents, and directory candidates. **Listings through the walk's own `visit_folder` are the truth:** relisted folders replay every entry, and big folders are probed one name per new handle. **Cursor:** per root, holding the journal id, LowestValidUsn, next USN and a last-record fingerprint; FirstUsn is checked before and after the read. Roots with equal cursors share reads. **Report:** the mirror `usn_dirs` plus a census give exact ScanStats. **Order:** non-subtree intents are sorted by walk_key and merged with in-place sub-walks through the same ScanWriter, so ids and winners equal the walk's. **Staleness:** affected refs are opened by id for link counts and live values, and open sessions are tracked. **Access:** a logon-independent token projection, with an O(dirs) re-probe when it changes. **Read path:** unprivileged FSCTL_READ_UNPRIVILEGED_USN_JOURNAL on the volume-root handle, gated by a per-build self-test; the privileged read only when elevated. Every other failure falls back to the walk. | Q-S1 (b); [USN1]-[USN8]; [UR1]-[UR21]. |
| D38 | Seen floor | `scans.floor`: a walk's floor is its own id; a journal pass's floor is its root's last full walk. Unseen ⇔ `file_seen.last_scan < floor`. A pass marks a row unseen by stamping the seeded sentinel `scans.id = 0`. mark_missing flips the unseen set in FolderPages/present_page order. Journal passes take SkipMerge from `usn_dirs_bad`. seen_identity stamps only rows of the scanned root. | Avoids O(n) stamps per pass. The statements are unchanged apart from parameters, and the trigger stamp order equals the walk's ([UR5]). |

## 4. Phases

**Commit policy.** Phases 1-8 are one commit each on the current branch. After all of them, run a single `just build && just lint && just test`, then fix forward. Intermediate commits do not compile. Forward dependencies are marked `→`.

**Oracles.** Every equivalence oracle is the old code or SQL kept under `cfg(test)` or the `fixture` feature.

### Phase 0: Preconditions (the user's)

- **0.1** The user lands the in-flight S-17 Unit E work before Phase 1 starts. Phases 1-2 depend on two parts of it:
  - `crates/platform/src/sqlite.rs`: `sqlite_path`, `attach_bytes`, `alias_of_folder`;
  - the `carry_rows` `attach_bytes` change.

  The unrelated untracked picker and yandex edits are the user's.

### Phase 1: Shared foundations (no behaviour change)

**1.1 Temp storage on disk** [r1 T1]

- **Closes:** scan-S08, import-06, import-08, grid-F10, dedup-11, dedup-18.
- **Files:**
  - `.cargo/config.toml`
  - `crates/catalog/src/schema.rs` (SETTINGS)
  - `crates/thumbcache/src/lib.rs` (remove the MEMORY pragma)
  - `vendor/README.md`
  - `docs/DATABASE.md`
- **Change:**
  - `LIBSQLITE3_FLAGS = "SQLITE_TEMP_STORE=0 SQLITE_DEFAULT_MEMSTATUS=1"` (force).
  - SETTINGS gains `temp_store=FILE` and `journal_size_limit=67108864`.
  - Every production ORDER BY is total.
- **Tests:**
  - `catalog/tests/temp_storage.rs`: the compile options; a 400k-row sort raises the highwater by less than 8 MiB; a 200k-row trigger UPDATE journals on disk.
  - `tests/no_memory_temp.rs` lint.

**1.2 `pg_schema::env`: connect, roles, hooks, URIs** [r1 T2, grid S3, scan S2; F3; F5; U7; D3/D4/D21]

- **Closes:** grid-F14, A5, R2-desktop-10.
- **Files:**
  - `crates/schema/{Cargo.toml (+pg-platform), src/env.rs, src/lease.rs}`
  - `crates/platform/src/sqlite.rs` (`attach_uri(path, Mode::{Ro,Rw})`)
  - every `Catalog::open` caller (D19)
  - `app/src/app.rs` (split pools)
- **Change:**
  - **SqliteEnv.** `SqliteEnv{temp_dir (D21), cache: CacheBudget, hard_limit}`.
  - **connect(path, flags, Role)** sets up:
    - SETTINGS plus the role cache;
    - a 1 ms busy poll up to 5 s;
    - one composed progress handler: the cancel/interrupt flag plus a step counter under `instrumented`;
    - a statement cache of 64;
    - the ledger trace and statement tags;
    - under `instrumented`: the **authorizer** (an SQLITE_UPDATE on `files.missing` is denied unless the statement's tag is allowlisted, [F5]) and **read-transaction span recording** ([U13]);
    - a **wal_hook** that chains the default autocheckpoint and feeds the high-water sampler ([U7]).
  - **attach_uri.** Every ATTACH of the catalog uses a percent-encoded `file:` URI with `mode=ro|rw` and never CREATE. The callers are thumbcache `pending_in` (`lib.rs:768`, today a default-flags connection) and ThumbPass.
  - **Role table (MiB):**
    - Writer 256 (runner, prepare, Upgrade, Scans including journal passes); Pair 64; ListLane 64; BrowseLane 64;
    - Ui reader 16 (`query_only`); Edits 16; Provider 8 × provider threads (8);
    - Keeper 16; WriteBack 16; ThumbPass 128 main + 64 cat; ThumbStore 16;
    - Groups 64; GroupsScratch 16 (via connect, no hardcoded `cache_size`);
    - Names 16 (`names.sqlite`, 7.16); Watch 4 (`query_only`, the USN watch, 3.9);
    - Rebuild: phase caches per D23, scaled to min(128, budget/2.5) under the harness;
    - Steady total 884 MiB, about 0.86 GiB.
  - **Pools.** `ui` is size 1, GUI thread only (debug-assert). `provider` is capped at the provider thread count.
- **Tests:** `schema/tests/env.rs`:
  - per-role `cache_size`;
  - temp files appear in temp_dir and are gone after;
  - a second `apply` with different values errors;
  - cancel still interrupts while steps count;
  - an `attach_uri` to an absent path creates nothing;
  - the authorizer denies a crafted folder-wide `missing` UPDATE;
  - a second process's frames are visible to WriteBatch through `pnLog`.

**1.3 Guard toolkit** [r1 T3, wl assert_bounded, grid VM_STEP] (moved before keyset, [PRF9])

- **Files:**
  - `crates/schema/src/testing.rs`
  - `crates/catalog/src/fixture/plan.rs` (`with_billion_stats`)
  - existing guards converted
- **Change:**
  - `Plan::of` (EQP plus an opcode census, including subprograms).
  - `assert_streaming`, `assert_temps`, `assert_bounded(allow)`, `assert_write_fanout`.
  - `steps_of`.
  - Every guard runs with no stats and again with 1B-shaped stats.
- **Tests:** the toolkit's own tests on crafted statements.

**1.4 Write discipline** [r1 scan S2; F5; U7; D18]

- **Closes:** scan-S09 (lock hold), A3.
- **Files:**
  - `crates/catalog/src/batch.rs` (new)
  - `lib.rs` (`write_transaction`)
  - `crates/jobs/src/control.rs` (`checkpoint_with`)
- **Change:**
  - **Gate:** a FIFO ticket per catalog path. Plus `WriteTx` and `read_transaction`.
  - **WriteBatch** opens lazily and commits at `rows`, at 250 ms, or when the transaction's frames reach **F = 16 MiB / page_size**. Every 64 driving rows it calls `sqlite3_db_cacheflush` and reads the `SQLITE_DBSTATUS_CACHE_WRITE` delta. That count over-counts frames, so commits come early, never late. Bound: ≤ (F + 64·P) pages ≈ 26 MiB, with P (pages per row) pinned in 3.8.
  - **Before each batch:** a `PRAGMA wal_checkpoint(PASSIVE)` reads the live `pnLog`. Above WAL_CEILING it tries `RESTART` with a 50 ms busy budget, then sleeps 100 ms (stoppable). Foreground WriteTx are exempt.
  - The lock yields 2 ms after being held for 1 s.
- **Tests:**
  - gate FIFO under 8 threads; no BEGIN before the first write;
  - commits at rows, age and frames (a statement dirtying 50 random pages per row commits at ≤ F + 64·50 frames);
  - `set_rating` succeeds within 150 ms during a batch loop and while a reader holds a 5 s snapshot;
  - a second process grows the WAL past the ceiling and this process waits.

**1.5 Keyset and subtree primitives** [r1 wl S3, scan S3, grid continuation rule; F1]

- **Files:**
  - `crates/schema/src/keyset.rs`
  - `crates/catalog/src/subtree.rs`
- **Change:**
  - `KeySeek`: NULL-aware multi-key seeks, plus a single-key form. Rule: exactly one bound lower limit per indexed column.
  - `folder_in_root!`, `FolderPages`, `SkipMerge` with `reseed(cursor_path)`: O(depth) exact root probes, since roots are an antichain (`lib.rs:786-800`). SkipMerge takes a `SkipSource` (`ScanSkipped(scan)` or `UsnBad(root)`, 3.3).
  - **`root_range(root)`** (D9): the (anchor id, key lo, key hi) of a root, plus `folder_under!(folder_id, path)`, the exact `under_root!` byte test as a folders PK probe (FolderCache-backed in Rust).
  - The `paged(read_page, apply_write)` driver.
  - The concrete `present_page` text lands in 3.3, because it needs `file_seen` from 2.1.
- **Tests:**
  - drains equal `ORDER BY` at budgets 1/2/7/1024;
  - SkipMerge equals the `under_root` oracle, including sibling, nested, UNC and drive cases, and after a reseed;
  - `root_range` + `folder_under!` equals `folder_id IN under_root!` on two drives with mirrored layouts, case-variant folders and a drive root;
  - no TEMP B-TREE.

**1.6 `pg-spill` and platform primitives** [r1 pdq S5/S9, dups S3, scan S5, grid S6; F2; F4; G5; S4; U4; PRX10; PRX15; USN1]

- **Files:**
  - `crates/spill/src/{lib, extsort, budget, spill, segstack, paged, codec, reorder, concurrent, scratch}.rs`
  - `crates/platform/src/{file.rs (Staged, read_at/write_at, file identity), volume.rs (free_bytes, mount_root, detachable, volume_root_handle), process.rs (peak_private_bytes, run_in_job)}`
  - `crates/platform/src/usn.rs` (new: Journal trait, query, reads, wait_read, parser, resize, open_by_id, links, dir_path, unpriv_selftest, collation, VolumeGuard)
  - `crates/platform/src/token.rs` (new: access_projection, change_notify_enabled)
  - `crates/platform/src/dir.rs` (probe_entry, first_call)
  - `Cargo.toml` (windows-sys features Win32_System_Ioctl, Win32_Security, Win32_Storage_FileSystem, Win32_UI_WindowsAndMessaging, Win32_System_IO, Wdk_Storage_FileSystem; checked against 0.61)
  - `app/src/store.rs`
- **Change:**
  - **ExtSort:** stable, fan-in 64, `merge_reduce`.
  - **Budget:** a process-wide semaphore with an interactive share (256 MiB, grid) and a background share (256 MiB: dups, pdq bulk, review, picker, journal pass). Background holders never take the interactive share.
  - **SpillQueue and SpillStack.** SpillStack has `mark`/`truncate_to` in O(1).
  - **Segment stack** for borders ancestor tails.
  - **PagedFile.**
  - **Codec:** 64 KiB blocks that decode independently. A frame record holds:
    - the name in WTF-8 or UTF-16LE, whichever is shorter;
    - front coding against the previous name;
    - a zigzag varint id delta;
    - the volume, only when it differs.

    Worst case 27 + 2·units B.
  - **ReorderSpill** ([S4]): 64 MiB segments with live counts, plus a PagedFile offset table of 12 B per index.
  - **ConcurrentReader** ([G5]): IOCP with overlapped ReadFile on Windows, N reader threads elsewhere. QD 128 on SSD, 32 on seek-penalty volumes. Elevator sort per submission window.
  - **ScratchRoot** `<catalog>.scratch/`:
    - a reservation ledger (bytes reserved per lease in `work_leases.reserved`) checked against `free_bytes` before any work, with priorities: interactive > pass > maintenance;
    - every owner is lease-named (SQLite temp, scan frames, journal-pass runs, borders, reorder, picker, groups scratch, adhoc, rebuild, tier1 work, runs, tree generations, facet log);
    - `hand_over(old_lease, new_lease)` renames a sealed directory (D16);
    - startup sweeps only stale leases through lease GC.
  - **Staged.**
  - **mount_root:** lexical: drive, `\\server\share\`, or the library root on POSIX.
  - **USN platform ([USN1]).** Windows only; every other OS returns Unsupported. Everything sits behind `trait Journal` (with `Listing`), so FakeVolume and fault wrappers can stand in.
    - **JournalHandle::open(`X:\`):** FILE_READ_ATTRIBUTES|SYNCHRONIZE, share RWD, BACKUP_SEMANTICS|OVERLAPPED. Unprivileged.
    - **`query()`:** returns `{journal_id, first_usn, next_usn, lowest_valid_usn, max_usn, max_size, alloc_delta}`. Codes 1179 → NotActive, 1178 → Deleting, any other → Unavailable.
    - **`read(start, journal_id, end, each)`:**
      - READ_USN_JOURNAL_DATA_V1 with ReasonMask !0, ReturnOnlyOnClose 0, versions 2-3, via FSCTL_READ_UNPRIVILEGED_USN_JOURNAL (0x903AB).
      - The privileged FSCTL_READ_USN_JOURNAL on `\\?\Volume{GUID}` is used only when the token is elevated.
      - Fixed 1 MiB buffer.
      - Safe parser: length bounds, alignment 8, V3 high id bits 0, USN strictly increasing, otherwise Malformed. Names are never read.
      - Codes 1181 → EntryDeleted, 87 → BadStart.
    - **`wait_read`:** overlapped, cancellable by CancelIoEx.
    - **`resize`:** FSCTL_CREATE_USN_JOURNAL. Elevated; keeps the ID.
    - **`open_by_id`:** returns `Live{end_of_file, last_write, creation, attributes, number_of_links}`. `links()` reads FileHardLinkInformation into a 1 MiB buffer; `dir_path()` reads FileNormalizedNameInfo. Codes 87/2/3 → Gone, 5 → Denied.
    - **`probe_entry(dir, name)`:** opens a NEW directory handle per name, because the FileName pattern is fixed by the first query on a handle. It loops with RestartScan = FALSE until an exact UTF-16 match or no more files, so POSIX `<`, `>`, `"`, `*` and `?` names and case-sensitive directories are handled. It yields the same `Entry` as `Lister::each`.
    - **`first_call(dir)`:** Lister::open plus one listing call, the walk's own access check.
    - **`detachable()`:** true when the bus type is one of {Usb, 1394, Sd, Mmc, Virtual, FileBackedVirtual}, or RemovableMedia is set, or IOCTL_STORAGE_GET_HOTPLUG_INFO reports DeviceHotplug or MediaRemovable.
    - **`access_projection()`:** BLAKE3 over a canonical encoding of:
      - TokenUser;
      - enabled and deny-only group SIDs, except SE_GROUP_LOGON_ID groups (logon-type SIDs are kept);
      - restricted SIDs;
      - AppContainer SID and capabilities;
      - the integrity SID;
      - the enabled state of SeBackup, SeRestore and SeChangeNotify.
    - **`unpriv_selftest(dir)`:** denies itself access to a file F and to a folder G, writes F and G\c through held handles, and requires those records in the unprivileged read. Result: Verified, Filtered or Unsupported.
    - **Collation:** a 64K upcase table (RtlUpcaseUnicodeChar), `collate`, a prefix-free `collation_key`, and `is_table_independent` (every unit < 0x80).
    - **VolumeGuard:** a message-only window with DBT_DEVTYP_HANDLE registration. On DBT_DEVICEQUERYREMOVE or GUID_IO_VOLUME_LOCK it cancels and closes the handles and raises a stop for that volume. It re-arms on unlock or a failed removal.
    - **Not used:** per-file FSCTL_READ_FILE_USN_DATA; the device handle for reads; a FILE_READ_DATA volume handle; NtQueryDirectoryFileEx pattern reuse; the volume's $UpCase.
    - **Memory:** fixed 1 MiB journal buffer, 1 MiB link buffer, 64 KiB probe buffer, 128 KiB upcase table, 64 KiB per watched volume.
- **Tests:**
  - ExtSort equals stable sort;
  - SpillStack proptest including mark/truncate;
  - codec proptest (non-UTF-8 names, unpaired surrogates, CJK, random 64-bit ids, block edges); 1M NTFS-like frames average ≤ 20 B;
  - ReorderSpill: segments are deleted when they empty;
  - ConcurrentReader gives the same bytes as ReadAt, with ≤ QD in flight;
  - the reservation ledger refuses an oversize reservation before work starts and never sweeps a live lease; `hand_over` is a rename;
  - `run_in_job(200 MB)` stops a 400 MB hog;
  - **USN1:**
    - parser vectors, truncation at every byte, and fuzzing (never panics);
    - real NTFS unelevated (PG_TEST_DIR): query and read see the test's own create, rename, link and delete with the right refs, parents and reasons;
    - a mid-record start gives 87;
    - probe_entry: ≥ 3 probes in one 20K-entry folder; POSIX `<`, `>`, `"` names; a case-sensitive Foo/FOO pair;
    - links() lists both links;
    - unpriv_selftest is Verified on 19045;
    - access_projection is equal across two logons of the same user and differs between elevated and not;
    - detachable() is true for a VHD;
    - FSCTL_LOCK_VOLUME succeeds while a wait_read is armed.

**1.7 pg-model shared types** [r1 pdq S1, dups S2, wl S1; QT1; G1]

- **Closes:** dedup-12, the WorkList part of import-01.
- **Files:** `crates/model/src/{pdq.rs, ids.rs, num.rs, dups.rs, grid.rs, path.rs}`
- **Change:**
  - `min_quality!()`, `PdqSeq`, FolderCache (which also caches folder path and anchor for `folder_under!`), `Listed`/`Fetched`.
  - **Numeric contract ([QT1]):**
    - `Rank(u64)` (serde-transparent), `GlobalRow`, `ListEpoch`, `ItemRef{rank, epoch, file}`;
    - `LayoutY(u128)`, in 2^-32 px, with `from_local`/`to_local`;
    - `MAX_CATALOG_BYTES = 2^48`, from the file format: `typedef u32 Pgno` (`sqlite3.c:16993`) and page size ≤ 65536 (`:14995`);
    - `rank_to_qml`/`rank_from_qml`, `FileId::to_decimal`/`parse_decimal`.
  - **PDQ model ([G1] as refined by D25):**
    - the pair definition `formed(x, y) = !(y.pend_batch.is_some_and(|b| b < x.batch))`, `HEAVY_ENTER=512`, `PairKey`, `PairRecord`, `Band`;
    - the parity lemma.
  - **dups:** `group_distance!`, `pdq_bands!`. There is no `COUNTS_KEY` (D22).
  - Const assertions: every record id width (entry id, query index) against its cap ([PRX20]); PDQ_BANDS partitions 0..=31.
- **Tests:**
  - round trips at 0, 2^31±1, 2^32+7, 2^46 and 2^48-1;
  - `rank_from_qml` rejects NaN, inf, fractions and 2^53;
  - LayoutY associativity is exact;
  - weight and parity proptests;
  - legacy-free seq ordering;
  - `formed` with a NULL/None `pend_batch` is true, and agrees with the SQL text on random inputs.

**1.8 pg-jobs pipeline and progress** [r1 wl S2, pdq S10/S11, dups S7; F2; S4; S1]

- **Files:**
  - `crates/jobs/{src/pipeline.rs, src/control.rs}`
  - `library/src/crop.rs`, `cli/src/borders.rs`, `cli/src/lib.rs`
  - `desktop/src/library.rs`
  - `client/src/text.rs`, `client/locales/en/text.ftl`
- **Change:**
  - `run(ctx, feed, ...) -> Ended{Exhausted, Stopped, SinkStopped, FeedFailed}`, with an explicit shutdown order.
  - **The item travels with its result to `sink(i, item, result, len)` on every path,** including unread items and failed opens (`pipeline.rs:116`, `:143`, `:179`). There is no index-to-item table.
  - **`Options.order: Order::{Completion, Index{window_rows, window_bytes}}`.** Index mode needs `R: Reorderable{held_bytes, codec}`:
    - results inside the RAM window (rows and bytes) go into a ring;
    - everything else goes to ReorderSpill, and workers never block;
    - the head index is always admitted;
    - a stop or Exhausted end flushes the held and spilled rows in index order;
    - sink returning false is final.
  - Control gains `lower_total`, `raise_total`, and `Phase::{Pair, Index, Groups, Upgrade, Rebuild, Removal, ScanJournal, AccessReprobe}`.
  - Messages: `PassPair`, `PassIndex`, `PassGroups`, `PassUpgrade`, `PassWorkingOpen`, `PreparingCatalog`, `PreparingPhase`, `RemovalInProgress`, and the journal notices (`JournalInactive`, `JournalWrapped`, `JournalBaseWrapped{needed}`).
- **Tests:**
  - paged items are read once, in grant order;
  - one 500 ms-slow head among 10^5 fast items: RAM stays ≤ window_bytes, the spill is deleted afterwards, sink indices strictly increase, workers stay busy;
  - a run of failed opens with window 1 in Index mode;
  - a stop during 10^6 Empty fetches ends within one fetch;
  - sink-false gets no further calls;
  - FeedFailed comes after the in-flight sinks.

**1.9 Lease helper** [D16; PRX5; F3; TE2]

- **Files:** `crates/schema/src/lease.rs`
- **Change:** These are SQL texts over a `work_leases`-shaped table. The catalog's table is created in 2.1; the cache's lease reuses the helper.
  - `claim(kind, target, unique)`, which returns an id and token;
  - a heartbeat thread that increments a counter every 5 s;
  - `adopt` when the counter has not changed over the adopter's own monotonic 60 s, Lock (where it works) is free, and on the same host the pid plus start time is dead. Adoption takes a **new id and directory**, and carries over only checkpointed immutable files, by copy or by `ScratchRoot::hand_over` rename;
  - `fence(tx)`, a token check inside each write transaction;
  - pause detection (a monotonic gap of more than 30 s), after which the owner re-verifies;
  - `release`; GC.
- **Tests:**
  - a paused owner, then adopt, then resume: the old owner's next write unit aborts, and results equal the uninterrupted run;
  - a 120 s statement is not taken over;
  - no wall-clock dependence (clock skew injected);
  - a rename handover leaves exactly one owner of the directory.

### Phase 2: One coordinated schema change, rebuild, upgrade and startup

**2.1 BASELINE objects** (the single edit; [r1 pdq S7, scan S1, wl S4, grid S1/S14, dups S4]; F1; F3; S1; S2; S7; S12; U1-U4; TE5; G7; USN2; RR1; RR2)

- **File:** `crates/catalog/src/schema.rs`. Literals come from macros (`max_attempts!`, `match_distance!`, `group_distance!`, `min_quality!`).
- **files table:**
  - Drop `last_scan`.
  - Add `borders_due INTEGER NOT NULL DEFAULT 0` (0/1/2), `pend_batch INTEGER` (NULL unless pending, D25) and `unpaired_anchor INTEGER` (D9). `pending` is documented as 0/1/2.
  - `UNIQUE(volume, file_ref)` becomes `files_identity`.
  - Indexes:
    - `files_unpaired_fresh(unpaired_anchor, order_key) WHERE unpaired=1 AND volume IS NOT NULL`
    - `files_unpaired_kept(unpaired_anchor, order_key) WHERE unpaired=1 AND volume IS NULL`
    - `files_present_order(volume, file_ref & 0xFFFFFFFFFFFF) WHERE missing=0`
    - `files_borders_due(volume, file_ref & M) WHERE missing=0 AND pending=0 AND borders_due=1`
  - `files_to_import` stays `WHERE missing=0 AND pending=1`.
- **analysis table:**
  - Drop `borders_due` and `analysis_borders_due`.
  - Add `analysis_no_dihedral` and `analysis_taken_utc ON analysis(file_id) WHERE taken_utc IS NOT NULL`.
- **folders table:**
  - Add `present` and `folders_shown`.
  - Add `anchor INTEGER NOT NULL`: the `anchors.id` of the path's drive or UNC share. It is immutable like `path`, and is declared without REFERENCES, so it needs no FK index; a lint checks it against `anchors`.
  - `path` UNIQUE becomes `folders_path`.
- **anchors table** (SMALL): `anchors(id PK, path UNIQUE)`, the normalised drive or `\\server\share`, or `/` on POSIX.
- **file_writes table:**
  - The PK becomes `file_writes_key`.
  - Add `parked_folder`.
  - Drop `file_writes_next`.
  - Add `file_writes_ready((temp IS NULL), attempts, queued) WHERE attempts < 5 AND parked_folder IS NULL`.
  - Add `file_writes_failed(queued, file_id, kind) WHERE error IS NOT NULL`.
  - Add `file_writes_parked(parked_folder) WHERE parked_folder IS NOT NULL`.
  - New tables `write_parked_roots(root_id PK, path UNIQUE, recheck_at, state)` and `write_parked_folders(folder_id PK, root_id, recheck_at)`, with an index on `(root_id, recheck_at)`.
- **Scan tables:**
  - `file_seen(id PK → files CASCADE, last_scan, last_claim)`, `scan_skipped`, `pair_ambiguous`, `scan_cleanup`.
  - `root_removals(id, path, key_lo, key_hi, hidden, cursor_path, cursor_id, report, finished)`.
  - `scans` gains `pair_tier`, `pair_cursor` (the pairing continuation, 3.2), `finished`, `kind` (0 walk, 1 journal, 9 sentinel), `floor` (D38; NULL = its own id) and `usn_state BLOB` (phase, committed walk_key position, seal id, E, fingerprint).
  - Seed the sentinel `scans(id 0, root X'', started 0, finished 0, kind 9)`. Readers listing scans use `id > 0`.
- **USN journal (USN2):**
  - `usn_builds(build PK, result)`.
  - `usn_volumes(serial PK, collation_ok, collation_note)`.
  - `usn_roots(root_id PK → roots CASCADE, volume, root_ref, journal_id, lowest_valid_usn, base_scan → scans (indexed by`usn_roots_base(base_scan)`, as the D24 FK lint requires), next_usn, fp_usn, fp_ref, fp_reason, fp_time, access_proj, rules_hash, verify_due, last_reason, census: folders, files, entries, reparse_files, extra_links, foreign_ids)`.
  - `usn_dirs(id PK, root_id, volume, ref, parent_ref, path, wkey, state 0 listed/1 unreadable/2 reparse, error, entries, media, reparse_files, counted)` with:
    - `usn_dirs_ref` UNIQUE(volume, ref, root_id);
    - `usn_dirs_path` UNIQUE(path);
    - `usn_dirs_parent`(volume, parent_ref);
    - `usn_dirs_bad`(root_id, path) WHERE state <> 0.
  - `usn_entries(root_id, volume, ref, parent_ref, name, kind 1 multi-link/2 reparse media/3 Foreign, PK(all but kind)) WITHOUT ROWID`, with `usn_entries_parent` and `usn_entries_foreign(volume, ref) WHERE kind = 3`.
  - `usn_stale(root_id, volume, ref, open_session, PK(root_id, volume, ref)) WITHOUT ROWID`.
  - `usn_wake(root_id, volume, ref, PK) WITHOUT ROWID`.
  - Scratch: `usn_drop(scan, folder_key, file_id, PK) WITHOUT ROWID`, where folder_key is FolderPages' order, so a PK scan equals present_page order; and `usn_scope(scan, path, kind 0 folder-only/1 subtree, PK(scan, path)) WITHOUT ROWID`.
- **Grid:** `files_log(seq PK, id, what)`.
- **PDQ (G7, with D25):**
  - `pdq_indexed(seq PK, file_id → named unique pdq_indexed_file, batch)` (no `pend_batch`: it lives on files);
  - `index_runs` (kind pdq/pdq_heavy; state staged/live/retired; `retired_gen`);
  - `pdq_heavy(file_id PK, seq UNIQUE, gen, batch, dihedral, counted, near, transfer)`;
  - `pdq_heavy_stage`, `pdq_band_stage`, `pdq_heavy_gone` (with `acct`/`groups` flags and `pend_batch` copied from `files.pend_batch`), `pdq_heavy_flip`, `pdq_heavy_attr`, `pdq_requeue`, view `pdq_heavy_unsettled`;
  - `pdq_pairs` gains `CHECK(distance BETWEEN 0 AND match_distance!())`;
  - `pdq_near(file_id PK → files CASCADE, n)`.
- **Dups:**
  - `dup_member`, `dup_group_member`, `dup_group`, `dup_group_rank`, `dup_log`;
  - `hash_runs(kind, hash, total, present, present_xor, PK(kind, hash)) WITHOUT ROWID`.
- **Shared:**
  - `work_leases(id, kind, target, owner, token, beat, inputs, seq_lo, seq_hi, dead, progress, reserved)` with unique partial indexes for pair, groups, upgrade and scan (per root);
  - the `counts` row (D22): files, present, pending, borders_due, analyzed, failed, failed_retryable, writes, writes_failed, writes_left, band_0..3, heavy_band_0..3, the 4 identity counters, the per-family recount cursors, and `bulk`. `SEEDS += ('counts','one')`.
- **Thumbnails, catalog side (TE5):**
  - `catalog_opens(id PK, seq, nonce)`;
  - views `thumb_clock(seq, epoch, horizon, extent)`, `thumb_walk`, `thumb_removed`, `thumb_opens`.
- **Meta seeds:** every key is fixed and seeded, and none is ever added or deleted at run time ([U1]):
  - `view_epoch`, `log_horizon`, `thumb_epoch`, `opens_ack`;
  - `pdq_seq`, `pdq_paired_hi`, `pdq_batch`, `pdq_write`, `heavy_vis`, `heavy_gen_next`;
  - `lease_next_id`;
  - `groups_state='pending'`, `groups_version`, `groups_gen`, `groups_consumed`, `groups_transition`, `groups_count`, `groups_files`;
  - `upgrade_import`, `upgrade_borders`, `upgrade_taken`, `upgrade_counts_<family>` (×4), `bulk_walk_import`, `bulk_walk_borders`.
- **Triggers:**
  - **grid:** `files_removed`, `files_renamed`, `analysis_cut_changed`, `analysis_shape_changed`, `analysis_removed` (D7);
  - **work-lists:** `files_identity_changed`;
  - **D8:** `files_pending_stamp` (changed/seq only; suppressed by `counts.bulk`);
  - **folders.present:** 3 counter triggers;
  - **pdq:**
    - `pdq_indexed_removed`;
    - `files_pending_on` (old=0 AND new<>0): sets `pend_batch = meta.pdq_batch` and queues `pdq_heavy_attr` for a heavy file;
    - `files_pending_off` (old<>0 AND new=0): sets `pend_batch = NULL`, queues `pdq_heavy_attr` for a heavy file (with the NULL write), and queues a requeue only when a `pdq_indexed` row exists;
    - `files_heavy_presence`;
    - `pdq_heavy_removed`, which copies `files.pend_batch` into `pdq_heavy_gone`;
    - the dead-count triggers for both run kinds.

    None of these is suppressed by `counts.bulk`.
  - **dups:** the coalesced log with a gen refresh and the overflow switch (6.6), band counters, `hash_runs` (with `present_xor`) and `pdq_near` upkeep, and the shared files BEFORE DELETE (presence-off transition first, [S12]);
  - **counters (S1):** about 12 triggers, each gated by its family cursor;
  - **U2 exit triggers:** turn flag 2 into 1 on a key change or a missing flip;
  - **U4 unpark trigger:** `files AFTER UPDATE OF folder_id, missing`;
  - **USN:** `files_usn_wake_u` (AFTER UPDATE OF volume, file_ref, folder_id) and `files_usn_wake_d` (AFTER DELETE): `INSERT OR IGNORE INTO usn_wake SELECT root_id, volume, ref FROM usn_entries INDEXED BY usn_entries_foreign WHERE kind = 3 AND volume = OLD.volume AND ref = OLD.file_ref`. That is one partial-index probe, nearly always empty.
- **Lists:**
  - CARRIED/NOT_CARRIED: every `usn_*` table is NOT_CARRIED (one walk per root after a rebuild); the wake triggers are recreated by BASELINE.
  - SMALL_TABLES = meta, session, views, roots, root_removals, scans, scan_skipped, anchors, usn_builds, usn_volumes, usn_roots.
  - LARGE_TABLES += usn_dirs, usn_entries, usn_stale, usn_wake, usn_drop, usn_scope.
- **Tests:**
  - `CARRIED ∪ NOT_CARRIED == tables`;
  - the seeded key set (scans 0 included) equals expected, and `mismatch()` is None at every Upgrade stop point;
  - lints: autoindexes only on `SMALL_TABLES`; every FK child column indexed; no upsert on analysis; no OR REPLACE on files or file_writes; every `folders.anchor` exists in `anchors`;
  - FK-cascade child triggers fire (pinned);
  - each partial-index `INDEXED BY` text prepares;
  - the dups DDL snapshot forces a `GROUPS_VERSION` bump;
  - the wake trigger's EXPLAIN uses `usn_entries_foreign`.

**2.2 Catalog rebuild: chunked, resumable, crash-safe** [F3 + U5 + S7; D23]

- **Closes:** R2-startup-01 (carry), A2, missed-4, round-1 residual 7, [PRF3], [PRX9].
- **Files:**
  - `crates/catalog/src/rebuild.rs` (new)
  - `schema.rs` (CARRY specs; the old `copy_tables` kept under `cfg(test)` as the oracle)
  - `lib.rs` (`prepare`, `open`, `rebuild`, `make_replacement`, `move_aside`, `acknowledge_rebuilt`)
  - `platform/src/{sqlite.rs, file.rs, volume.rs}`
  - `schema/src/env.rs`
  - `docs/DATABASE.md`
- **Change.** A state machine driven by `<catalog>.rebuild`, the manifest: JSON written by Staged plus fsync. It holds:
  - the build digest, the reason, the kept base name and the building path;
  - the kept file's change counter and size;
  - the built file's OS identity;
  - the dup-carry decision, the phase and the counts.

  The heartbeat goes in `<catalog>.rebuild.owner` (1.9). Phases: Start → Copy → Index → Verify → Derive → Finish → Swap. Every step is one transaction on `.building` that also writes its progress row.
  1. **Detect.** In prepare, under `pg_schema::Lock`:
     - a manifest exists: resume (a different digest restarts at Copy);
     - otherwise run the mismatch check.

     A file at version v < current is rebuilt to v, then migrated ([U5]).
  2. **Start.**
     - Checkpoint the old file (TRUNCATE, checking busy=0).
     - `PRAGMA journal_mode=DELETE` must return 'delete'. It succeeds only when no other connection is open, otherwise the result is Error 'catalog in use' (deviation h).
     - Assert no -wal, -shm or -journal is left.
     - Check free space (step 9).
     - Choose the first free `.replaced[.n]` base, record it, and rename the one main file.
     - An unreadable source renames its parts individually.
     - Resume renames only to the recorded base, idempotently.
     - A foreign file at the path gives `RebuildBlocked{path}`, with nothing moved.
  3. **Connections.**
     - Building: journal_mode=DELETE, synchronous=FULL, `main.locking_mode=EXCLUSIVE`, foreign_keys=OFF, temp_store=FILE. Cache 64 MiB for Copy, Verify and Derive; 128 MiB for Index (the PMA is sized from it).
     - Source: `ATTACH attach_uri(kept, Ro) AS carried`, `carried.locking_mode=EXCLUSIVE`, 16 MiB.
     - Kept integrity is checked at every resume, at every phase boundary, after any CORRUPT/NOTADB/IOERR, and before Swap: no kept-wal, no hot journal, and the change counter and size match. Otherwise the result is `KeptChanged`: building is discarded and the rebuild restarts at Copy.
     - First run on building: `bring_to(BASELINE)`, then save and drop every secondary index and trigger into `rebuild_objects`.
  4. **Copy.**
     - Each CARRY table, in batches of ≤ 100k rows, by rowid or KeySeek PK, with `INSERT ... SELECT ... WHERE key > ?c AND key <= ?hi ORDER BY key`. These are appends and are not journaled.
     - The 2.2 conversions run inline: `borders_due`; `file_seen`; the `temp` cast; `anchors` and `folders.anchor` from `folders.path`; `unpaired_anchor` from the row's folder when unpaired=1; `pend_batch`, which depends on the source:
       - a legacy (pre-BASELINE) source has no `files.pend_batch`, so it is set to 0 for pending rows and NULL otherwise;
       - a BASELINE-shaped source carries `pend_batch`, `pdq_indexed.batch` and `meta.pdq_batch` verbatim.

       Rebuild oracle case: a BASELINE catalog with pend_batch = k and unpaired seqs pairs the same after the rebuild.
     - `pdq_pairs` is filtered to `distance BETWEEN 0 AND 31`, and the rows it drops are counted into `dropped_pairs` ([S7]). Radius 31 means none exist.
     - Legacy catalogs (no `pdq_heavy`) do not copy `pdq_pairs`. Writer seqs are assigned in file_id order, `batch = 1`, and `pdq_paired_hi = 0` ([G7]). A pending row's `pend_batch = 0 < 1` excludes it exactly as today's index load does.
     - `usn_*` is not carried.
  5. **Index.** One saved index per transaction, from its exact text, through SQLite's sorter.
  6. **Verify.** For each FK child, a paged, resumable, ordered merge (`SELECT DISTINCT col ... INDEXED BY ... ORDER BY col` against the parent's id order). An orphan is a data error.
  7. **Derive.** All of it is sequential merges plus ExtSort plus key-order appends ([S7] P3), resumable:
     - `folders.present`: a keyset over `files_present` in 100k-entry pages;
     - the `counts` status columns;
     - `hash_runs` (with `present_xor`) and the identity counters;
     - `pdq_near` and `band_0..3`: two ExtSort passes over pairs;
     - the dup tables carried if `groups_version` matches, otherwise `groups_state='pending'`;
     - heavy state carried with `index_runs` (validated by catalog_id).

     Derived state is always recomputed from rows, never carried verbatim (deviation d).
  8. **Errors.**
     - Environmental (FULL, IOERR, CANTOPEN, interrupt or stop, NoSpace, KeptChanged): Err, and the manifest is kept for resume.
     - Data (UNIQUE, FK, CHECK, NOT NULL): `Carried::Nothing` as today.
     - CORRUPT or NOTADB from carried with an unchanged counter: retry once on a fresh attach. If it reproduces, **salvage**: bisect the table-key ranges, skip and record unreadable ranges, delete only the children of skipped parents, and report "N rows in M unreadable ranges dropped" ([U5]).
  9. **Free space.** Checked at Start, at each phase and at each resume. Needed = Σ remaining outputs (row bound × sampled bytes per row ÷ 0.9) + 2 × the largest remaining index + 1 GiB. Short gives `NoSpace`.
  10. **Finish.**
      - `locking_mode=NORMAL`.
      - One transaction: meta, `view_epoch += 1`, `log_horizon = seq`, the `counts` row, the triggers recreated from their saved text, and the progress tables dropped.
      - Record the OS identity.
  11. **Swap.**
      - Assert no building-journal exists, rename, and verify the identity on resume.
      - Write `<catalog>.rebuilt` (Staged) and delete the manifest.
      - `prepare` reports `Rebuilt` from the note until `acknowledge_rebuilt`.
  - **Absent-path window:** `Catalog::open` returns `Rebuilding` while a manifest exists, and every catalog ATTACH uses `attach_uri`.
  - **Progress** goes through PrepareHooks (2.7). Cancel uses the 1.2 interrupt.
- **Deviations** (§7 Q-D):
  - (a) The catalog path is absent during the build.
  - (b) The kept file is left in rollback mode.
  - (c) Environmental failures are resumable, where today they produce an empty catalog.
  - (d) Derived counters are recomputed from rows.
  - (e) A crash between the move and the rename no longer empties the catalog.
  - (f) A foreign file at the path gives RebuildBlocked.
  - (g) The Rebuilt notice is shown at least once.
  - (h) 'Catalog in use' is refused.
  - (i) A data error's reason text can name a different first violation.
  - (j) Corruption is salvaged by range instead of the whole library being dropped.
- **Tests:**
  - The equivalence oracle at batch sizes 1, 2, 7 and 100k: full dumps, the sqlite_schema set, and the derived oracles (including `folders.anchor`, `unpaired_anchor` and `pend_batch`).
  - `fail_after(n)` at every commit, manifest write and rename.
  - A half-done move resumes.
  - A Windows child-process kill resumes.
  - Orphan or duplicate gives Nothing.
  - `max_page_count` forcing FULL keeps the manifest.
  - Heartbeat: a 120 s statement is not taken over.
  - A write to kept gives KeptChanged.
  - A corrupt page is salvaged; a transient one is retried.
  - After Swap there is no journal.
  - The thumbcache ATTACH creates no file at the path.
  - Derive is resumable mid-folder.
  - A legacy catalog re-pairs to the oracle.
  - After a rebuild, an NTFS root's next scan walks (no `usn_roots`).
  - Scale: flat heaps, SQLite heap ≤ 2 × cache + 16 MiB + index/32768, temp ≤ 2.2 × the largest index, no WAL.

**2.3 Post-freeze migration machinery** [F6 + U12; D1]

- **Files:**
  - `schema.rs` (`Migration`, `bring_to`, `expected_for`, `version_of`)
  - `rebuild.rs` (shared copier, verifier, estimator)
  - `crates/schema/src/{shadow.rs, lib.rs (difference_allowing)}`
  - `catalog/src/upgrade.rs`
- **RebuildTable** (exclusive, runs in prepare behind the startup window):
  - The first transaction sets `user_version = (1<<30) | (V+1)`, so older builds take the refusal branch (`lib.rs:597-603`). It also saves and drops T's indexes, triggers and dependent views, and creates `T__new` plus the progress tables.
  - Each batch copies into `T__new`, then `DELETE FROM T WHERE key <= ?hi`. Only the table B-tree is touched, and freed pages are reused.
  - Then: drop the empty T; `legacy_alter_table=ON` rename; new indexes from exact text; FK merge; recreate views and triggers; clear the flag.
  - `bring_to` and `expected_for` run Steps through the same executor in memory, so the quoted `CREATE TABLE "T"` text matches.
  - This build's other processes get `Error::Migrating`.
- **Shadow** (online, an Upgrade step; also used by 7.17's opt-in indexes):
  - `sh_<name>` (WITHOUT ROWID), plus `sh_<name>_log`, plus seeded state keys.
  - Phase R: gate, short snapshot, in-RAM chunk sort, fsynced run file, manifest.
  - Phase M: K-way merge-append through WriteBatch.
  - Phase L: replay the log with old keys; ready in the same transaction.
  - Readers pick their statement by state.
  - `difference_allowing(names)` lets configured shadows and their triggers exist without making `mismatch()` non-None.
- **Tests:**
  - A RebuildTable(files) with views and triggers equals the 12-step oracle at batch 1/7/100k.
  - `fail_after` at every commit.
  - An older-build fixture refuses, and the file is byte-unchanged.
  - A migrated file's `mismatch()` is None.
  - No secondary index exists during batches.
  - Shadow equals `SELECT key, id ... ORDER BY` under concurrent writes and kills at every phase.
  - `set_rating` stays within 150 ms throughout.

**2.4 Writers adapted to the new columns** [r1 scan S4, wl S4, pdq S8, grid S1; D8; D9; D25]

- **Files:**
  - `crates/catalog/src/lib.rs`, `pair.rs`
  - fixtures (`raw.rs`, `state.rs`)
  - `tools/shots/fixture.py:101`
- **Change:**
  - **AnalysisWriter:**
    - the store transaction runs in this order: `delete_indexed`, then the analysis REPLACE and the pending → 0 update, then the insert of the new `pdq_indexed` row (new seq, batch = meta.pdq_batch). So the store's pending → 0 queues no requeue, and a test asserts that a store queues no `pdq_requeue` row;
    - a pdq seq is assigned when quality ≥ 50 and a dihedral exists, with `pdq_indexed.batch = meta.pdq_batch`;
    - `borders_due` goes to files;
    - `store_failure` clears `borders_due` and the indexed row;
    - `store_pdq_pair` is removed → 5.7.
  - **BordersWriter.cut** and **crop_source** move to files.
  - **ScanWriter:**
    - no held statements; `file_seen` stamps;
    - `folder()` upserts the anchor and inserts `folders.anchor`;
    - the insert, its DO UPDATE (which changes `folder_id` while keeping `unpaired`, `lib.rs:1223-1227`) and displace (`lib.rs:1219`) each set `unpaired_anchor` in the same row write. Each statement has its own form, because SET expressions read the row's values from before the update:
      - the INSERT uses `(SELECT anchor FROM folders WHERE id = ?4)`, since unpaired is the literal 1;
      - displace always uses `(SELECT anchor FROM folders WHERE id = files.folder_id)`, since it sets unpaired = 1;
      - DO UPDATE uses `CASE WHEN files.unpaired = 1 THEN (SELECT anchor FROM folders WHERE id = excluded.folder_id) END`.

      An 8.2 ledger check refuses any statement that derives `unpaired_anchor` from a column the same statement assigns.
  - **pair.rs rebind/replace** stamp `file_seen` and keep `unpaired_anchor` in step. The clear sets `unpaired = 0, unpaired_anchor = NULL`.
  - **mark_pending** sets `pending=1` only (D8 trigger stamps).
  - Fixture shims.
- **Tests:**
  - the existing suites;
  - `each_user_visible_change_names_its_file`, extended and schema-derived: every view-visible column has a stamping trigger;
  - the dups trigger proptests;
  - the rebind seq bump;
  - an onset sets `pend_batch` to the session clock; a store and a revert both clear it; a revert queues one requeue and a store none; a file re-imported in session k pairs as the older endpoint with a file imported in session k+5 (the oracle is today's `pdq_hashes` load);
  - invariant proptest: after any writer sequence, `unpaired_anchor = CASE unpaired WHEN 1 THEN folders.anchor END` for every row, including moves across drive letters.

**2.5 Upgrade pass: version recomputes off prepare, exact reads** [U1 + U2; D20; D32]

- **Closes:** A1, A3, missed-8, [PRF2], [PRX18].
- **Files:**
  - `catalog/src/{upgrade.rs, lib.rs (prepare, set_*_version), work.rs}`
  - `query/src/lib.rs` (`status_sql`, the taken conditions and sort key)
  - `library/src/runner.rs`
  - `cli/src/lib.rs`
  - `jobs/src/control.rs`
- **Change:**
  - **prepare.** In one IMMEDIATE transaction, compare `import_version`, `borders_version`, the TAKEN_ZONE fingerprint and the counter family keys. For each that differs:
    - write the new mark first;
    - set the fixed step key to its start phase;
    - import: bump `thumb_epoch`, set `groups_state='pending'`, reset `bulk_walk_import`;
    - borders: reset `bulk_walk_borders`.

    Cost O(1).
  - **`run_upgrade`** runs under the 'upgrade' lease, through WriteBatch.
  - **Import Pass A.** Id-keyset batches of `UPDATE files SET pending = CASE WHEN <stale> THEN (CASE WHEN missing=0 AND pending<>1 THEN 2 ELSE 1 END) ELSE 0 END ...`, with `counts.bulk=1` inside the transaction.
    - There are no index writes, and no seq or changed stamps.
    - **`pend_batch` is left to `files_pending_on`/`files_pending_off`,** which `counts.bulk` does not suppress (D8). Their write lands on the row's already-dirty page and no index holds `pend_batch`, so it costs no extra page.
      - The 3.8 fan-out pin and the §8 version-bump fan-out assertion both count the nested pend_batch UPDATE.
      - A lint requires every AFTER UPDATE trigger on files to name its columns (OF ...), so none fires on pend_batch.
      - Pass A CPU is stated with the nested statement included.
    - **There is no tier-1 gate and no backlog drain.** By D25, a backlog file x (session < bump) pairs with every y whatever y's stamp, exactly as today's inline pairing did. Imports after the bump are gated.
  - **Borders Pass A.**
    - Missing or pending≠0 rows get `borders_due = coalesce(has_borders, 0)` directly.
    - Present rows with pending=0 go to 2/0.
    - Files without an analysis are untouched.
  - **Taken step:** batches over `analysis_taken_utc`. The zone fingerprint is re-read each batch. It also marks `sh_taken` (7.17) not ready.
  - **Count steps:** per-family recounts (2.6).
  - **Shadow steps** (2.3, 7.17).
  - **Gates (D32):** `Catalog::gate` returns `UpgradePending{steps}` from import_work (import), borders_work (import, borders), crop list creation and crop_source (import, borders), and the groups maintainer and builder (import). Scans, removals, thumbs, writes and tier-1 are not gated.
  - **Exact reads.** `pending_now!(f)` = `CASE WHEN f.id < ?c THEN f.pending <> 0 ELSE <stale_of!(f)> END`, and `taken_now!(a)` likewise. Every reader of these values uses them; a grep guard enforces it. Two statement variants are plan-guarded.
  - **CLI:** `with_upgrade(|| cmd)` drives exactly the steps a command needs (import, borders find, status). Tier-1 is no longer gated (D25). `gallery write` labels its trailing count provisional.
- **Deviations:**
  - Scans, removals, thumbs and writes may run before the steps finish (ordering only).
  - The status line shows upgrade progress instead of counts.
  - A process time-zone change during a pending taken step restarts it (impossible on Windows: the CRT fixes the zone at first use).
  - D6's "import-version bump bumps seq" is replaced by `thumb_epoch` plus the gate.
- **Tests:**
  - The oracle (today's three statements) at batch 1/7/100k, with stops and re-bumps.
  - Keys stay mismatch-free at every stop.
  - Two processes take the lease in turn.
  - prepare is O(1) on 1M rows.
  - Gates fire inside the API.
  - Exact-read proptest: Status filters, taken filter and sort, and `ids(build_run)` equal an instant full recompute at every cursor position under concurrent writes.
  - Pairing during Pass A equals the oracle under D25; `pend_batch` after Pass A equals its definition.

**2.6 Counters and status** [S1 + U3; D22]

- **Closes:** A4, W1, R2-cli-01, R2-cli-08, the R2-writeback-01 counts, [PRF10].
- **Files:**
  - `schema.rs` (triggers)
  - `lib.rs` (`status`)
  - `writes.rs` (`remaining`, `left_to_write`)
  - `work.rs` (totals)
  - `writeback/src/lib.rs:144`
  - `cli/src/lib.rs`
  - `import/src/lib.rs`, `library/src/borders.rs`
- **Change:**
  - Triggers keep the `counts` row exact under plain INSERT, upsert (files, file_writes), REPLACE (analysis, via BEFORE INSERT) and cascades (files BEFORE DELETE subtracts first). Each trigger adjusts only when its row key is below its family's cursor.
  - `status()` reads one row. Present pairs = Σ(band + heavy_band, folded plus staged ≤ vis). While a removal is pending, the 3.4 correction applies.
  - Totals: borders total = `counts.borders_due`; import total = `counts.pending + counts.failed_retryable`. Both are read with S0 in the cursor's first transaction ([PRF13]).
  - A post-freeze definition change is an Upgrade recount step: zero the family, set its cursor to 0, add each batch's true contribution and advance the cursor in the same transaction, then set the sentinel.
  - There is no prepare-time recount and no `COUNTS_KEY`.
- **Tests:**
  - A proptest against `old_status` (`cfg(test)`) after every step: upserts, REPLACE, cascades, toggles, file_writes producers and consumers.
  - Pin tests for upsert and REPLACE trigger semantics.
  - Recount one family while others write: all families equal their oracles.
  - VM_STEP for `status()` is O(1).
  - Golden `gallery status` output.

**2.7 Startup off the GUI thread; opens log** [U6 + S8; TE5; D36]

- **Closes:** R2-startup-01 (UI), missed-5, the S8 review issues.
- **Files:**
  - `desktop/src/{lib.rs, app.rs, preparing.rs}`
  - `desktop/qml/Preparing.qml`
  - `cpp/gpu_device.cpp`
  - `app/src/app.rs` (`open_with(config, PrepareHooks{progress, stop})`)
  - `cli/src/lib.rs`
  - `catalog/src/lib.rs` (`prepare` appends `catalog_opens(meta.seq, 63-bit nonce)` before any migration, when the table exists; `ack_opens` prunes, keeping ≤ 65,536)
- **Change:**
  - `run()` starts the GPU device and `gui_app`, then spawns the open worker: prepare, ThumbCache open, Ui pool, Reader.
  - Within 300 ms it proceeds as today.
  - Otherwise it joins the GPU thread, loads `Preparing.qml` into a separate engine, and shows it with `show_windows(prep, gpu)`. Progress is queued through `qt_thread()`.
  - Close or Quit sets the 1.2 interrupt, the worker returns `Cancelled`, and the next start resumes.
  - On success it drops `prep` and loads Main as today.
  - The CLI prints stderr progress lines, and Ctrl+C interrupts.
- **Tests:**
  - A stub prepare that is slow shows the window after 300 ms, rendered with the app's device.
  - Cancel exits within 100 ms.
  - Exactly one `show_windows` over Main's roots.
  - A fatal error shows today's message.
  - `tst_preparing.qml`.
  - `prepare_records_its_open_before_any_migration`.

### Phase 3: Scanner and pairing

**Closes:** scan-01..08, scan-11, scan-S03, scan-S08, scan-S09 (writes), import-10, residual (d), [PRF5], [PRF8], [PRF20], [PRX13], [RR2], [RR3], Q-S1, R-SCAN (on local fixed NTFS).

**Files:**

- `crates/scan/src/{lib.rs, sink.rs (new), order.rs (new), journal.rs (new)}`
- `platform/src/{dir.rs, usn.rs, token.rs}` (from 1.6)
- `catalog/src/{lib.rs, pair.rs, subtree.rs, usn.rs (new)}`
- `crates/testkit/src/ntfs.rs` (new)
- `library/src/runner.rs`
- `app/src/library.rs:166-174`
- `desktop/src/library.rs` (Full rescan action)
- `jobs/src/control.rs`
- `cli/src/lib.rs`

- **3.1 Walk** ([r1 scan S6] + [F4] + [USN3]):
  - **Inline pass.** Up to 16,384 kept entries in the reused Vec, exactly as today.
  - **Validate pass**, when a folder overflows: O(1) state (no-id flag, I/O result, count). A failure leaves the folder unreadable as today, with nothing recorded.
  - **Relist pass.**
    - It streams each entry through today's per-entry function, in listing order, through WriteBatch.
    - `frames.mark()` is taken before the first entry.
    - On failure: `truncate_to(mark)`, then `scan_skipped`, `clean=false`, no descent, and the rows already recorded stay.
  - **Frames** go to a codec SpillStack beyond 16K. Paths are rebuilt from the in-RAM ancestor chain.
  - **Out of space** (scratch ENOSPC or cap, or SQLITE_FULL):
    1. drop the SpillStack, deleting its files;
    2. roll back the open batch;
    3. return `Error::OutOfSpace{Scratch|Catalog}`.

    The result is a crash's state.
  - **Stop and pause** inside a listing: `Listing::each` returns ControlFlow, and `checkpoint_with(batch.commit)` runs every 1,024 entries.
  - **Also:**
    - the `claim().seen_this_scan()` rule; volumes collected into `Walked`;
    - **`visit_folder(lister, frame, mode Walk|Dirty, sink) -> FolderTally`**, extracted with today's per-entry decisions unchanged. It emits `SinkOp::{Record, SeenIdentity, Skip, Descend, Listed}`. FolderTally is exactly the walk's ScanStats increments for that folder.
    - **Sinks:** `DirectSink` (ScanWriter through WriteBatch) and `SortedSink` (ExtSort by walk_key, used only for journal intents outside sub-walks).
    - **walk_key (order.rs):** a folder is `wk(parent) ++ 0x01 ++ invert(collation_key(name))`; a file is `wk(D) ++ 0x00 ++ collation_key(name)`. Lexicographic order equals the LIFO walk's order, and `subtree_end(wk)` bounds a subtree.
    - **Collation check:** every adjacent pair of a listing must collate Less; the first violation sets `usn_volumes.collation_ok = 0`.
    - **Mirror upkeep** on capable roots (Windows, local NTFS): subfolder diffs against `usn_dirs` (paged, differences only), tallies, subtree deletes for skipped or vanished folders, `usn_entries` kinds 1/2/3, and `usn_stale(R)` upkeep (one `open_by_id` per kind-1 identity or stale member). `usn_wake(R)` is cleared at finish.
    - **Base cursor, crash-safe:**
      - **begin_scan:** a walk deletes `usn_roots(R)` in its own transaction.
      - **Before listing:** query the journal; fingerprint the last record < E0; take the access projection and `rules_hash`.
      - **At finish:** re-query the journal. Store `usn_roots` in finish_scan's transaction only if the journal id and LowestValidUsn are unchanged and FirstUsn ≤ fp_usn. Otherwise store nothing, set `last_reason`, and show `JournalBaseWrapped{needed ≈ (NextUsn_end − E0) ÷ walk hours × rescan interval × 1.5, rounded up to 1 GiB}`.
    - seen_identity stamps only rows of the scanned root.
  - **Deviations:**
    - (a) A relist failure keeps the rows recorded before it.
    - (b) A stop during a relist leaves part of the folder recorded.
    - (c) Out of space is typed.
- **3.2 Pairing** ([r1 scan S7] + D9 + continuation + [USN4]):
  - Fence `hw`.
  - **Read the root's scope once.** The root's (anchor, key range) of `files_unpaired_fresh` is read into an id-sorted ExtSort spill (16 B per row). Rows whose folder fails `folder_under!` are dropped (FolderCache), so the set equals today's `folder_id IN under_root!`.
  - **Tiers.** The 4 tiers iterate the spill in visited-bounded pages, read optimistically and re-done in a WriteTx only when a candidate exists.
  - **`pair(root, scan, walked, floor, pace)`:** every tier's `< ?scan` on `file_seen` becomes `< ?floor` (D38). A walk passes floor = scan, so its statements are unchanged.
  - **`Walked.volumes`** for a journal pass:
    - {v0} if census files − duplicates > 0;
    - otherwise, only when some fresh row has volume ≠ v0, {v0} iff an identity probe finds a row for some kind-2 entry;
    - else ∅.
  - Exact tier skips.
  - `pair_ambiguous`.
  - The outbox is written inside replace.
  - The clear (`unpaired = 0, unpaired_anchor = NULL`) and the `unpaired_with_data` count (fresh plus kept (anchor, range)) are paged.
  - **A stop is honoured between pages.**
    - `scans.(pair_tier, pair_cursor)` persists the position, and `finished` stays NULL.
    - The next Scans round resumes that scan's pairing first: it rebuilds the spill from the scope and skips ids ≤ cursor in the current tier, then runs mark_missing, then finishes.
    - The result equals uninterrupted pairing for the same interleavings Q6 already accepts.
- **3.3 mark_missing** ([r1 scan S8] + [F5] + [USN4]):
  - Signature: `mark_missing(root, scan, floor, scope, skip)`. Per row: `UPDATE files SET missing=1 WHERE id=?1 AND folder_id=?2 AND missing=0 AND (SELECT last_scan FROM file_seen WHERE id=?1) < ?3`, over `present_page` (`INDEXED BY files_present`, LIMIT counts visited rows) plus SkipMerge, through WriteBatch.
  - **Walks:** WholeRoot + `ScanSkipped(scan)`, with ?3 = scan.
  - **Journal passes:** one ordered stream merging, by (FolderPages folder key, file id):
    - `usn_drop` (its PK order);
    - `present_page` over each `usn_scope` entry in path order (folder-only or subtree).

    Duplicates are collapsed. SkipMerge reads `UsnBad(root)` from `usn_dirs_bad`. ?3 = floor. So the flipped set and its order equal the walk's whole-root pass, and the seq/changed/files_log stamps match ([UR5]).
  - **Drop and scope stamps** (per row, D10):
    - `stamp_dropped`: `UPDATE file_seen SET last_scan = 0 WHERE id = ?1` plus an insert into `usn_drop`;
    - `stamp_scope`: `UPDATE file_seen SET last_scan = 0 WHERE id = ?1 AND last_scan < ?scan AND last_scan >= ?floor`.
- **3.4 Root removal** ([F1] + D34 + [S10] + [RR2] + [RR6]):
  - `remove_root` inserts `root_removals(path, key_lo, key_hi)` from `scope_keys` and returns.
  - **Membership:** a row belongs to the removal iff its order_key is in [lo, hi) (prefilter) and `folder_under!(folder_id, path)` holds (the exact `under_root!` byte test). The probe is paid only for rows inside the range. Same-named folders on other drives stay visible.
  - **Readers mask at once (D34):**
    - grid build and refresh add `NOT (order_key >= lo AND (hi IS NULL OR order_key < hi) AND folder_under!(folder_id, r.path))`.
      - A drive or UNC root has no upper key bound, so `root_removals.key_hi` and the `root_range` hi are NULL. Every range text (1.5, D9, 3.4) therefore uses `(hi IS NULL OR order_key < hi)`.
      - Tests: a drive-root range test in 1.5; a 3.4 test that removes `E:\` while `D:\` stays visible;
    - an open run receives a filtered delete over the key range (O(run rows in range)), not a range delete;
    - the tree subtracts the removed folders' counts from merged nodes, and hides only nodes left empty;
    - facets use the same condition;
    - **status:** present = `counts.present` − Σ `folders.present` over folders under a pending removal's path (a `folders_path` range), and missing = (`counts.files` − `counts.present`) + that sum. Both are exact, cost O(folders under pending removals), and apply only while a removal exists.
    - analyzed, failed, pending and pdq_pairs are shown as of the removal start, with a `RemovalInProgress` note (Q-D).
    - The Duplicates filter counts twins under the removed root until they are marked (Q-D).
  - **run_removals** pages per row with a forward-only done-prefix watermark (`cursor_path`, `cursor_id`), in ≤ 60 s runner slices after Scans (D17).
    - Each batch adds `hidden + ?n`, where n is summed in Rust, in the same transaction as its marks.
    - Resume is O(depth) through `SkipMerge::reseed`.
    - There is no lease: the per-row recheck makes concurrent pagers exact.
    - It also pages out the root's `usn_dirs`, `usn_entries`, `usn_stale`, `usn_wake` and `usn_roots` rows.
  - Completion sets `finished`. The app's row is deleted; a `report=1` row is left for the CLI, which reads `hidden`, deletes it and prints today's text. A runner sweep deletes report rows older than 24 h.
  - CLI `status` and `dups` drive pending removals first.
  - `add_root` cancels a pending removal. An `add_root` that replaces nested roots deletes their `usn_roots`.
  - Orphan cleanup: `usn_dirs` has no FK, so the cascade misses it. Before mirror upkeep, a walk's begin_scan pages out every `usn_dirs`, `usn_entries`, `usn_stale` and `usn_wake` row under the root's path whose root_id ≠ R, one row at a time through WriteBatch. These are the orphans of replaced or removed roots. The same rule goes in 3.7 begin_scan and USN4 housekeeping.
  - Tests: add a parent root over two journaled roots, then walk; re-add a root while its removal is pending.
  - Stated internal deviation: per-row seq and changed stamp order follows folder path, not folder_id ([PRF19]). No user-visible result depends on it.
- **3.5 Outbox** ([r1 scan S10] + [S10]):
  - Pages of ≤ 1024 rows.
  - One `forget_many` cache op per page (logged by the cache triggers, 4.4).
  - Temps deleted.
  - Exact-id acknowledgement.
- **3.6 `import --again`** ([r1 scan S11] + [S3] + [RR2]):
  - Target normalization.
  - `mark_pending_at` per row through the pager. Folder targets page through FolderPages (path-exact).
  - **File targets** resolve through `file_by_path`, which becomes a `files_order` probe: `order_key = folder_key ++ name_key AND missing = 0 ORDER BY order_key, id`, iterating every row with that order_key (the same key on other drives) until the `rules.same` folder check matches.
  - Deviation: ties among matching rows return the lowest id.
- **3.7 Housekeeping** ([PRF8]):
  - `prune_files_log()` (LOG_KEEP 1M down to 512k, in paged deletes, advancing `log_horizon`) runs at the end of each Scans pass and each CLI scan.
  - `forget_scan_scratch(scan)` always runs. It also deletes `usn_drop` and `usn_scope` pages.
  - `begin_scan(root, kind, floor)` deletes `pair_ambiguous` and `scan_skipped` residue of earlier scans of the same root, paged, and deletes `usn_roots(R)` when kind = walk.
- **3.8 Missing-writer allowlist and fan-out** ([F5]):
  - Allowlisted statement tags:
    - per-row mark_missing;
    - per-row removal;
    - ScanWriter displace (`lib.rs:1219`) and upsert DO UPDATE (`:1223`);
    - pair.rs rebind (`:237`) and replace (`:289`);
    - fixture-only `raw.rs:68` and `state.rs:79`.
  - Also listed (as file_seen and usn_wake writers, not missing writers): the two stamp-0 statements and the `files_usn_wake_u/d` triggers.
  - A source test matches only SET clauses: `UPDATE\s+files\s+SET[^;]*?\bmissing\s*=`.
  - c = Δ`sqlite3_total_changes` per toggle at degree 0/1/10/1000 is pinned to the derivation from the trigger text (≤ 14). P (pages per row) is pinned.
- **3.9 Journal rescan** (D37, D38; [USN4]-[USN8]; Q-S1 b). The journal only says where to look; listings are the truth; results replay in walk order through the same writer.
  - **R0 ladder.** Any failed rung returns `NeedWalk(reason)` before anything is written. The rungs:
    - Windows, local NTFS (GetVolumeInformationByHandleW); the scan path equals a root exactly; check_folder passes; a `usn_roots` row exists.
    - The root passes `first_call` (the walk's own access check), and its FileIdInfo ref and serial equal the stored ones.
    - Not `detachable()`, unless `journal.trust_detachable` lists the serial (Q-S3).
    - `usn_volumes.collation_ok`.
    - The read is verified: `usn_builds(current build) = Verified` (the self-test runs in R0 if absent), or the privileged read is in use.
    - SeChangeNotifyPrivilege is enabled.
    - `rules_hash` is unchanged.
    - **Journal:** journal_id and LowestValidUsn equal the stored values; FirstUsn ≤ fp_usn (or ≤ cursor with no fingerprint); cursor ≤ NextUsn ≤ MaxUsn; the read from fp_usn returns the fingerprinted record first. Any error or mismatch (87, 1181, other) → NeedWalk.
    - If `access_proj` differs, R-A runs first instead of a walk.
  - **R-A access re-probe** (only on a projection change):
    - `usn_dirs(R)` in pre-order, paged 1024, with an O(depth) skip stack.
    - `first_call` on each directory. Descendants of a failing or state-2 directory are skipped, as the walk skips them.
    - Transitions become R3 intents: listed → unreadable is a StateChange: W1 drops the mirror below the folder, and its rows are kept and excluded from missing through UsnBad, as the walk's scan_skipped does. Unreadable → listable is a SubWalk.
    - The projection is stored at W7. A stop discards. Cost O(dirs) (R-USN3).
  - **R1 read** [cursor, E = NextUsn at R1 start]:
    - one ExtSort by ref, holding OR'd reasons, a dir flag, the last record's reasons, `closed_after_last_change` and the last USN;
    - a second uncapped ExtSort of distinct (ref, parent) pairs;
    - roots with equal cursors share one read;
    - afterwards a re-query: the same journal_id and FirstUsn ≤ fp_usn, else NeedWalk. FirstUsn only increases, so nothing unread was purged.
  - **R2 classify** (streamed in ref order):
    - A merge-join with `files_identity`, plus `usn_dirs_ref`, `usn_entries(R)` and `usn_stale(R)` lookups.
    - **Relevance:** a ref is relevant if it has a row in R, an exception or stale entry in R, is a mirror dir of R, or has a parent that is a mirror dir. Only relevant records count toward the crossover.
    - **Links:** a ref with HARD_LINK_CHANGE, ≥ 2 distinct parents, or kind-1 gets `open_by_id.links()`, and every link parent in mirror(R) becomes dirty. If Denied, the union of kind-1 parents, record parents and the row folder is dirty, flagged Relist.
    - **Staleness open:** skipped only for FILE_CREATE without HARD_LINK_CHANGE and with a CLOSE after the last change. This skip is conditional [UR4]: it stays only if the USN1 real-NTFS test (added to the 1.6 USN1 tests) shows that such an entry equals the live values from open_by_id after close. Otherwise every affected media ref is opened. Otherwise `open_by_id` gives NumberOfLinks and live values.
    - **Output:**
      - ExtSort D of (folder wkey, ref, flags). Directory candidates contribute their old parent, their new parent, and themselves when their reasons include SECURITY_CHANGE, REPARSE_POINT_CHANGE or BASIC_INFO_CHANGE.
      - Spill A of affected rows, keyed (FolderPages key, file id, ref).
      - A sorted membership file of affected refs with a fence index.
  - **R3 dirty folders**, from D in wkey pre-order, with an O(depth) skip stack over SubWalk, DropSubtree and newly unreadable roots.
    - **Directory candidates** (resolved by `dir_path` or the parent's relist):
      - moved within R: DropSubtree(old) + SubWalk(new);
      - moved out: DropSubtree;
      - created or moved in: SubWalk;
      - into a skip name, a reparse point or unreadable: a StateChange;
      - a directory in D itself gets `first_call`.
    - **Relist mode** applies when:
      - entries ≤ max(4096, 64·k);
      - or an affected ref is Denied;
      - or a probe contradicts `open_by_id`;
      - or opening a probe_entry handle fails. This is treated as the walk's open error: P is relisted, and a list failure follows the 3.1 relist-failure rules [UR2];
      - or the order rule fails: more than one intent-carrying name (files, or separately child folders) and some name has a unit ≥ 0x80.

      In Relist mode, `visit_folder(P, Dirty)` emits an intent for EVERY media and reparse media entry, exactly as the walk does. Subfolders are compared with the mirror, the tally is recomputed, and P becomes a folder-only scope.
    - **Probe mode** otherwise: `probe_entry` for each affected link in P. Found → intent. Absent → drop candidate. Absent while `open_by_id` listed that link in P → switch to Relist, never absent. The tally changes by the difference from the before-state.
    - **Staleness** (both modes):
      - A ref enters `usn_stale(R)` when it has more links than R lists, or live ≠ listed values, or no CLOSE after the last change.
      - It leaves when CLOSE has been seen and all listed entries equal live.
      - Denied → J-a(ii).
    - **Multi-link:** for every intent whose identity has ≥ 2 listed links in R, every listed link gets an intent, and the kind-1 rows are rewritten.
  - **R4 SubWalk plan:**
    - SubWalk roots with their wkeys;
    - `subtree_census(old path)` for in-R moves;
    - a moves map (old → new prefix) for remapping kind-1 link parents.
  - **R5 recurring sets:**
    - every state-1 directory gets `first_call`, as the walk retries it;
    - `usn_stale(R)` members are re-probed;
    - `usn_wake(R)` members get a claim probe (and, if no longer Foreign, a `probe_entry`); then the wake rows are deleted.
  - **Cost rung (Q-S4):**
    - est_pass = c_e·(Σ relist entries + Σ subtree_census(moved) + state-1 entries) + c_p·probes + c_o·opens;
    - est_walk = c_e·census.entries + c_d·census.folders;
    - walk when est_pass > 0.8·est_walk. Constants are calibrated in Phase 8.
  - **R6 seal**, under the scan lease: the sorted non-subtree intents, the SubWalk roots, DropSubtree and StateChange lists, the moves map, A, the membership file and the scopes. Then `begin_scan(R, kind 1, floor = base_scan)` = J, with `usn_state = {W1, ∅, seal, E, fp}`.
  - **W replay:**
    - **W1:** mirror and `usn_entries` deletions under DropSubtree old paths and new state-1/2 directories, per row, with the census.
    - **W2 merged replay** in walk_key order. The sorted intents go to DirectSink, merged with each SubWalk run **in place** at its position: the 3.1 frame loop, `visit_folder(Walk)`, DirectSink and MirrorWriter. Sub-walk entries need no scratch.
      - **Inline rule** after the walk's claim probe: when the hit row is in R, not claimed by J, and (outside every DropSubtree of this pass, or kind-1, or in the membership file, or stale), call `links()`. The winner is the listed link with minimum walk_key (remapped through the moves map). When two links of the identity sit in one folder and have a non-ASCII name, that folder is listed for ordinals, and its listing order decides [UR12]. It upserts; the others count a duplicate and write kind-1 rows. Denied → abort to walk.
      - Otherwise a plain upsert: a move by construction, with no open.
      - **Unexplained results** (an unaffected relisted entry that is Changed, a relisted folder's unseen row not in A, or a claim hit outside every DropSubtree with no record) are traced and set `verify_due = now` (Q-S5).
      - WriteBatch commits advance `usn_state.position`.
    - **W3:** in (folder key, id) order, `stamp_dropped` each A row not claimed or stamped by J and not under UsnBad, then `stamp_scope` each scope. The sealed files are deleted.
    - **W4:** SkipSource = UsnBad(R).
    - **W5:** pair(root, J, Walked{clean = complete ∧ no state-1/2 directory, volumes exact}, floor), with the 3.2 continuation.
    - **W6:** mark_missing (the 3.3 ordered merge).
    - **W7:** finish_scan, which calls `store_cursor(E, last record < E, lowest_valid, access_proj, census)`, then forget_scan_scratch. The cursor moves only here.
    - ScanStats = replay counts + mark_missing + pairing + census. Report counts come from the census (errors = state-1 count; skipped_reparse = reparse_files + state-2; duplicates = extra_links + foreign_ids; failures = the first FAILURES_KEPT state-1 rows by wkey; loops = Σ tally.loops).
  - **Stop, kill and resume:**
    - **R0-R6 or R-A:** a stop discards scratch and writes nothing, and the root goes back to the front.
    - **W1-W3:** the next Scans round adopts the sealed files by rename (D16) and resumes from `usn_state`. A SubWalk resumes from its root. It prunes children with subtree_end ≤ position, and skips entries ≤ position in the folder that holds the position. Affected refs met inside a SubWalk get the R2 staleness open. The 24 h lost-or-old-seal abandonment applies only before W3 completes, since nothing sealed is needed after W3.
    - **Lost or old seal:** if the sealed files are gone or older than 24 h, J is abandoned (no pairing has started) and a fresh pass runs from the unchanged cursor. The result equals a stop plus a rescan (Q6).
    - **W5-W6:** resume through the 3.2 continuation and `usn_drop`/`usn_scope`.
    - **Abort to walk** (a Denied winner check or an impossible inconsistency): J ends like a stop (complete = false, nothing marked missing, cursor unchanged), and the same pass runs a full walk. This equals stop plus walk under Q6.
  - **Equivalence invariant I:** after every pass of root R, for every row r of R not under a state-1/2 directory, last_scan(r) ≥ floor_R ⇔ the walk of that tree saw r. Rows, missing flags, mirror, `usn_entries` and census equal the walk's. The proof covers:
    - untouched entries yield Same;
    - merged replay is the walk's non-Same subsequence in order;
    - drops plus scopes are the walk's unseen set, flipped in its order;
    - pairing gets the same inputs;
    - the report is Σ tallies.
  - **Runner, CLI and app ([USN6]):**
    - **Scans pass:** `scan()` tries the journal pass. On NeedWalk it walks in the same pass and records the reason in `usn_roots.last_reason`, a trace event and `gallery scan --explain`.
    - **Verification walks:** when `verify_due ≤ now`, the root walks (Q-S5, `journal.verify_days` 30).
    - A pending J resumes first, and queued roots with equal cursors share R1.
    - **CLI:**
      - `gallery scan --walk`;
      - `gallery journal status`: cursor, size, FirstUsn age, days of history, last reason, verify due, self-test;
      - `gallery journal size <volume> <bytes>`: elevated, keeps the ID; 4 GiB recommended at 1B (Q-S6);
      - the notices print the fsutil equivalent and the estimate.
    - **Journal sizing with consent (Q-S6, decided):**
      - **Sizing estimate:** each journal pass records the volume's record rate. That gives target = rate × max(`journal.verify_days`, `journal.retention_days`, default 30) × 1.5, clamped to [32 MiB, 8 GiB], plus an AllocationDelta of target/8.
      - **When to offer:** when the current MaximumSize holds less than the retention days, or when R-USN2 has just forced a walk. The app then offers, once per volume per size bucket, "Enlarge the change journal on E: to N GiB (keeps its history)".
      - **On accept:** the per-session helper (`pg-volume.exe`, silent elevation here; a UAC prompt elsewhere) runs a new closed request, `ResizeJournal { volume, max_size, delta }`, which calls `FSCTL_CREATE_USN_JOURNAL` and keeps the journal ID.
      - **On decline:** the choice is stored in `gallery.json` and is not asked again until the estimate doubles.
      - **Non-admins:** never resized; the notice explains why.
      - The CLI `gallery journal size` stays as the manual path. Nothing ever resizes silently.
      - **Tests:**
        - the FakeVolume estimate;
        - the offer, decline and re-offer rules;
        - the helper request validation (local NTFS volume only; bounds checked);
        - the journal ID kept after a resize on the test VHD.
      - `docs/NTFS_MFT_SCAN_PLAN.md` gets the `ResizeJournal` request.
    - **App:** Rescan uses the journal; a 'Full rescan' action forces the walk.
    - **Watch** (app only, Q-S7):
      - **Handles:** one thread holds an overlapped `wait_read` per capable volume, registered with VolumeGuard.
      - **Relevance:** it checks relevance through a `Role::Watch` read-only connection and a 16 MiB LRU (the parent or ref is a mirror dir of a root on that volume, or a `files_identity` hit there).
      - **Own writes ignored:** records for the catalog, -wal, -shm and journal refs, and under `<catalog>.scratch` or the thumbnail cache, are ignored. Those files are recreated, so their refs are read again at each open and checkpoint.
      - **No waiting read:** if waiting reads are not supported, there is no watch, and Rescan still works.
      - **Queueing:** a relevant record queues a Scans request, debounced 5 s. The watch never writes.
      - **Shutdown and locks:** at shutdown CancelIoEx, then join. A volume lock or removal closes the handles. A pass's handles are registered too: a lock raises a stop (R discards; W keeps its prefix).
  - **Testkit ([USN7]):** `crates/testkit/src/ntfs.rs` FakeVolume implements Listing, Journal and the token and bus probes on every OS. It models:
    - refs with sequence and reuse;
    - multi-link stale directory entries, refreshed silently by another-link opens;
    - open sessions with lazy index updates;
    - POSIX delete; reparse points; ACL denial (including OpenFileById denied while listing works);
    - token changes; case-only and POSIX-wildcard names; NTFS collation with an optional divergent non-ASCII $UpCase;
    - a bounded journal with wrap, purge during a read, ID change, rollback and LowestValidUsn jumps;
    - an unprivileged-read filter toggle; a bypass mode; transient open and list errors; volume locks.

    A **parent-ambiguity mode** randomises the record parent for HARD_LINK_CHANGE, FILE_DELETE and RENAME_OLD_NAME. The `walk_with` and `journal_with` fixtures live in `scan/src/lib.rs`.
  - **Memory:** ≤ about 140 MiB from the background share, constant in n:
    - read buffer 1 MiB;
    - R1 groups ≤ 40 MiB, shared with the pairs sort;
    - D ≤ 24 MiB;
    - intents ≤ 24 MiB;
    - merges 8 MiB;
    - mirror LRU 16 MiB;
    - in-place SubWalks: the 3.1 bounds;
    - skip stack ≤ 256 KiB; fence index ≤ 4 MiB; link buffer 1 MiB; probe buffer 64 KiB.
  - **Disk:** scratch about 60 B per record, 150 B per D pair or non-subtree intent, 40 B per A row or scope, and 0 per sub-walk entry. Catalog: `usn_dirs` about 280 B per directory.
  - **Time** (1B, NVMe warm; measured in Phase 8): see §2. A walk is 1-4 h.
  - **Deviations** J-a … J-f are in §7 Q-D.
- **Tests:**
  - All walk tests at the default budget and at `Budget{1,1}` with `Limits{rows:1, age:0}`.
  - Validate and relist failures.
  - 40,000 subfolders pop in reverse listing order.
  - Out of space leaves scratch empty and makes zero commits after the failure.
  - A stop in the validate pass writes nothing; a stop in the relist gives `3 × BATCH` rows.
  - A pause lets a second connection's BEGIN IMMEDIATE succeed within 100 ms.
  - Two-root pairing visits only its root's (anchor, range). Two drives with mirrored layouts pair and remove independently; a drive root reads only its anchor.
  - A stop between pairing pages, then resume, equals the uninterrupted result.
  - Removal:
    - a 25,000-row single folder pages at ≤ PASS_BATCH visited rows;
    - kill and resume;
    - concurrent pagers give `hidden` = 50;
    - the reseed with a nested root;
    - the mask hides immediately in grid, tree and status, and leaves `D:\Photos` visible while `E:\Photos` is removed;
    - status present and missing are exact during the removal.
  - Outbox id reuse.
  - The authorizer test; the fan-out pin.
  - The S12 differential proptest against `reference_pair`, `reference_mark_missing` and the reference walk, at page sizes 1/2/1000, with floor = scan.
  - An interrupted scan leaves no residue after the next scan of that root.
  - `file_by_path` with equal order keys on two drives.
  - **USN3/USN4:**
    - the DirectSink sequence equals itself sorted by walk_key;
    - FolderTally sums equal ScanStats;
    - the mirror equals an oracle rebuild;
    - a kill or stop mid-walk leaves no `usn_roots`;
    - a wrapped base gives no cursor and the notice;
    - drops in /b (low ids) and /a (high ids) plus a scope give stamps equal to WholeRoot;
    - census = Σ tallies;
    - UsnBad equals `scan_skipped`;
    - the wake trigger inserts exactly the kind-3 roots;
    - remove_root leaves no usn residue.
  - **USN6:**
    - runner: NeedWalk → walk; resume; verify_due → walk; a shared read;
    - CLI golden output: `scan` and `scan --walk` give identical lines, except elapsed time;
    - watch: one queued scan per relevant create; nothing for WAL, scratch, thumbnail or outside-root churn; FSCTL_LOCK_VOLUME and safe removal succeed on a VHD.
  - **USN8 oracle** (`scan/tests/usn_oracle.rs`): A and B are byte copies after a base walk. A script of operations runs, and at each quiescent checkpoint A runs its schedule while B runs `--walk`.
    - **Compared:** canonical dumps of files (every column including id, pending, seq, changed, unpaired, unpaired_anchor, missing, order_key), folders (id, path, key, anchor, present), the file_seen seen bit, scan_cleanup, counts, files_log order, the usn tables, ScanStats and scans.
    - **Schedules:** all-J; J → walk → J on the new floor; J stopped in W5 with the continuation resumed; J abandoned and redone; R-A after a token change.
    - **Seeds:** ≥ 10K FakeVolume seeds (parent ambiguity in half, Budget{1,1} and defaults), 200 real-NTFS seeds, and `just usn-vhd` (second-user and elevated mutators).
    - **Operation set:**
      - moves, renames and case-only renames; delete and recreate; save-over;
      - hard links within, across and outside roots, including a new link earlier in walk order;
      - junction swaps; deny and allow; a rewrite with mtime restored; extension changes;
      - create and delete in one pass; a 20K-entry folder with k = 1/3/50 probes;
      - `<`, `>` and `"` names and non-ASCII probe-mode names;
      - a renumbered volume;
      - files held open with lazy updates; POSIX deletes of open files;
      - ACL changes on mirror directories and the root.
    - **Deviation tests:** each of J-a … J-f differs in exactly the stated rows.
  - **USN8 faults** (`scan/tests/usn_faults.rs`):
    - (i) failing journal calls (k-th call of each kind; FirstUsn past the cursor while the read succeeds; a purge during R1);
    - (ii) a child abort at each W commit, including inside SubWalks;
    - (iii) random stops in R, R-A and W;
    - (iv) ENOSPC and SQLITE_FULL give typed OutOfSpace;
    - (v) VHD journal delete, create, wrap (including during the base walk), rollback and clone;
    - (vi) a projection change runs R-A; SeChangeNotify off walks;
    - (vii) a concurrent mutator, then one more pass equals the walk;
    - (viii) a collation violation walks; a divergent $UpCase relists;
    - (ix) a Filtered self-test walks;
    - (x) a volume lock during a pass stops it and resumes later.

    Every case gives an identical catalog or a fallback walk with an identical catalog, never a mixed one.

### Phase 4: Work lists, write-back, thumbnails, borders CLI

**4.1 `import_work` / `borders_work` cursors** ([r1 wl S7] + [U2] + D6 + [U13])

- **Closes:** import-04..08 (import-01, scan-10 and dedup-04 close with 5.1).
- **Files:**
  - `catalog/src/work.rs`
  - `lib.rs` (`pending_import`, `pending_borders` and `work_list` removed → 5.1)
  - `library/src/borders.rs`
- **Change:**
  - Two keyset streams are merged in (volume, record, id) order from frontier F:
    - the partial index (`INDEXED BY`);
    - while `bulk_walk_<kind> ≠ 'done'`, a walk of `files_present_order` from max(F, P) that returns flag-2 rows, examining ≤ 4096 entries per fetch.
  - Rows with `seq > S0` are skipped and lower the total.
  - P is persisted in each writer's store transaction and advances only past consumed keys.
  - **Retry list:** materialized into TEMP on the pass connection in 65,536-row keyset chunks over the failures index, with the `seq ≤ S0` filter per chunk (D31). S0 and the `counts` totals are read in the first chunk's transaction through ATTACH.
  - `import_work` reports whether its gate passed and its list is non-empty, so 5.1 can bump the session clock (D25).
- **Tests:**
  - Oracle drains equal the old SQL constants at PAGE 1/2/7/1024, including bulk flags and preemption at every fetch.
  - Total examined ≤ n + preemptions × 4096.
  - `done == total` at Exhausted.
  - Mutation yields each id at most once.

**4.2 Borders pass** on the cursor. The final flush runs on every `Ended`; then `end_work`.

**4.3 Write-back queue** ([U4] + [S2])

- **Closes:** W2, W3, W4, R2-writeback-01.
- **Files:**
  - `catalog/src/writes.rs`
  - `writeback/src/lib.rs`
  - `platform/src/volume.rs`
- **Change:**
  - **next_ready:** ≤ 10 probes, for t in (0, 1) then a in 0..=4: `... INDEXED BY file_writes_ready WHERE attempts < 5 AND parked_folder IS NULL AND (temp IS NULL)=?t AND attempts=?a AND queued <= ?now - ?settle*(?a+1) ORDER BY queued, rowid LIMIT 1`. Take the minimum (queued, rowid) over the first t that has a hit.
  - **attemptable** is O(1).
  - **recent_errors:** latest-first on `file_writes_failed` (a bug fix: the doc says "latest").
  - **Hierarchical parking:**
    - an offline failure parks the job on its (mount root, folder);
    - a Prober thread checks at most 16 roots and 16 folders per tick, with PROBE_WAIT 5 s and at most one probe outstanding per root;
    - a present folder unparks its oldest job as a probe;
    - producers' upserts and the files trigger unpark.
  - **W4:**
    - sniff from a 4 KiB prefix;
    - hash with `file_hash_of` (streaming);
    - `copy_into` through `io::copy` with a 1 MiB buffer;
    - verify through a read-only mapping, dropped before `remove_file`.
- **Deviations:** online jobs no longer wait behind offline ones; a share slower than PROBE_WAIT is parked, not counted as an attempt; recent_errors shows the latest errors.
- **Tests:**
  - The order oracle against today's NEXT_READY.
  - VM_STEP with 10^6 rows all settling.
  - 10^5 parked folders on an absent root cost one probe per 30 s.
  - A hung probe does not stall the drain.
  - W4 gives identical bytes and digest, and peak RAM stays below 64 MiB on a 1 GB file.

**4.4 Thumbnail cache layout: the one edit, then start over** ([TE1] + [U8] + [U9] + [U10]; D35)

- **Files:**
  - `thumbcache/src/{lib.rs, layout.rs, key.rs}`
- **Change:**
  - **Removed:** `checked`, `entries_lru`.
  - **Added:**
    - `clock`, `heights` (gen, sync cursor, tomb, reval, walk cursor w as an order-preserving 14 B BLOB, lap, counters, opens id/nonce, compact, owner, beat, used, healed, epoch);
    - `todo` (exact and range members, laps) with `todo_file`;
    - `lost` with `lost_order`;
    - `retired`;
    - **`height_totals(height PK, bytes, count)`**;
    - **`entries_height_lru(height, last_used, bytes)`**;
    - heal cursor keys.
  - **Triggers:**
    - LOG(H, F) as DELETE plus a plain INSERT (it cannot conflict);
    - entries added, removed and changed (OF file_id, height, mtime, rotation) log;
    - `todo` counter triggers keyed on (height, gen);
    - `height_totals` maintained by the totals triggers;
    - every totals trigger also adjusts `heal_acc` when id < `heal_cursor`.
  - CACHE_VERSION stays 1. `judge` starts existing caches over once (keeping the cap) and publishes `StartedOver`. Post-freeze cache changes use 2.3 Shadow.
- **Tests:**
  - Each entry write logs exactly its rows, including outer OR IGNORE/REPLACE.
  - Counters equal count(*) per (height, gen).
  - Retired-gen deletes leave live counters unchanged.
  - Key order matches SQLite.
  - A cache with `checked` starts over.

**4.5 Cache writer ops** ([TE2] + [U8] + [S9] + [U9] + [U10] + T4)

- **Closes:** T2, T3, T4, missed-1, missed-3, missed-6, R2-startup-02, residual (thumbs 2).
- **Change:**
  - **TE2 ops:**
    - Claim (IfFree or Steal; owner from `clock`; Drop beyond HEIGHTS_KEPT);
    - heartbeat through the 1.9 helper;
    - Release;
    - Reset and Drop by gen (O(1));
    - Sync, Put plus Settle (atomic), Compact, Opens;
    - Gc of retired gens in 4 MiB WAL slices;
    - StateRecount.
  - **evict** ([U8], [S9]):
    - K-way merge over per-height keyset cursors on `entries_height_lru`, heights ≠ keep, in the total order (last_used, height, bytes, id);
    - collect ≤ 1024 victims, reset the statements, delete, re-seek;
    - **short-circuit** on the main path only: if Σ `height_totals.bytes` over heights ≠ keep < excess, return false without walking. The slice loop is unchanged.
  - **Heal** ([U9]):
    - `open` only queues `HealIfDue`;
    - the writer runs 65,536-id chunks (5-10 ms each) between ops, accumulating in `heal_acc` under the gated triggers;
    - the final step fixes totals and `height_totals`, then publishes `TotalsFixed`;
    - a second process sees `healed` advanced and skips.
  - **Touches** beyond 65,536 ids spill to `temp.touch` ([U10]).
  - **forget_many**: one op per ≤ 1024 ids.
  - **Inbox:** a byte cap of 32 MiB on `PutOne` payloads drops the oldest PutOne (best-effort, regenerated on demand).
- **Deviations:**
  - One pass per height at a time: the CLI steals.
  - `TotalsFixed` may follow open.
  - A PutOne dropped under a sustained writer stall is regenerated later.
- **Tests:**
  - The TE2 lease tests.
  - Reset and Drop are constant-time, and GC never touches live rows.
  - The evict oracle (today's ORDER BY) with keep_height dominating; available = excess - 1 / excess / excess + slack.
  - Heal is exact under puts from another process, and open returns in under 10 ms on 1M entries.
  - Touch spill while another process holds the lock.
  - The inbox cap.

**4.6 ThumbPass** ([TE3] + ordered batches + memo)

- **Closes:** import-and-thumbnails-04/05/06, residual-2, residual-5, [PRF1], [PRX8].
- **Files:**
  - `thumbcache/src/pass.rs`, `pass/compact.rs`, `key.rs`
  - Cargo.toml (pg-catalog dependency dropped)
- **Change:**
  - **`pass(catalog, height, Claim) -> PassStart{Disabled, Busy{until}, Pass}`.**
  - **Open check** (one read transaction over both files). Reset when any of these holds:
    - S < c, or the catalog opens log shows a rollback or fork;
    - **`thumb_epoch` changed** (the import-version bump, [U1]).
  - **Chunked live-resolution sync**, each chunk ≤ 4096 rows and stoppable:
    - tombstones from `thumb_removed` (revalidate if `tomb < horizon`);
    - the delta over `files_seq`;
    - the `lost` log.

    Resolution R(id): delete any exact member, then insert into lap L if ok, key ≤ w and no lap-L range covers it. Abort rule: Reset when `sync_rows` > `walked_rows`.
  - **Budget:** B = max(2^20, N/16) members. Compaction merges the smallest gaps into ranges (two-level histogram), with hysteresis B → B/2. w is never lowered.
  - **Stream:** lap L (exact and range members), then the walk from w over `files_present_order`.
  - **SettleTracker** advances only over the settled prefix.
  - **Batches use `Order::Index` (1.8),** so a batch is the first BATCH successes in feed order, which is deterministic.
  - **Cache-full memo (Q-T4 a, decided):** before decoding, if `rejected[h] = (cap, total, own_bytes(h), digest of the next BATCH candidates' (id, seq))` matches and the stored rejection's candidates all succeeded, stop as cache-full without decoding. Q-D lists the exactness condition.
- **Theorem** ([TE3]): in a quiescent system, at a lap start, a round feeds exactly today's FULL `pending_sql(false)` rows in (volume, record, id) order. Any other round feeds those rows minus the files that already failed or changed in this lap.
- **Tests:**
  - The [TE3] differential proptest: identity rebind across w in both directions, id reuse, pruning, restore and advance, B tiny, steal.
  - A wake every k items still advances w.
  - An offline volume compacts to one range.
  - A cache-full round decodes 0 after the memo.
  - Batch membership is stable across worker schedules.
  - Plan guards.

**4.7 thumbs::run, runner and CLI** ([TE4])

- **Files:**
  - `thumbs/src/lib.rs`
  - `library/src/runner.rs`
  - `cli/src/lib.rs`
  - `library/tests/{video,writeback}.rs`
  - `bench/benches/ir/catalog.rs`
  - `docs/perf/baseline.json`
- **Change:**
  - `Disabled` → complete with nothing done; `Busy` → `elsewhere`.
  - Sync, then `ack_opens`.
  - The pipeline keeps the file_state check.
  - `first_failed` is removed.
  - Every `Ended` sends the final Settle and Release.
  - `needed: Option`.
  - The runner keeps a `retry_at` and waits on it; the CLI steals.
- **Deviations:**
  - The app yields to `gallery thumbs` for ≤ 30 s after a release, or ≤ 150 s after a CLI crash.
  - ≤ E + BATCH duplicate decodes per steal.
  - A failure is retried once per lap.
- **Tests:**
  - The CLI takes the height (duplicate decodes ≤ E + BATCH).
  - The app resumes after a release or a crash.
  - Disabled completes.

**4.8 `gallery borders --csv` and `--import-review`** ([F2] + [S4] + [S3]/[S6] + [RR9])

- **Closes:** R2-cli-02, R2-cli-03, residual (b).
- **Files:**
  - `cli/src/{borders.rs, sorted_walk.rs}`
  - `jobs/src/pipeline.rs`
  - pg-spill
- **CSV:**
  - **Counting pre-walk:** today's `walk` and `is_image` with `n += 1`, returning every error at today's point.
  - **SortedWalk per input:** each directory's entries are sorted by OsStr through ExtSort. Ancestor tails go to the one segment stack, written once per directory.
  - **K-way merge of inputs** by (path, input index), which equals today's stable `out.sort()`.
  - **The pipeline in `Order::Index`.**
  - **Output:**
    - rows go to `<csv>.partial`, exclusively locked;
    - on Ok: rename if no CSV exists, otherwise copy in place (truncate, write, flush) to keep the file's identity, ACL and hard links; then delete the partial;
    - on Err before the in-place copy starts: delete the partial, and the CSV is untouched;
    - on Err during the copy: keep `<csv>.partial` and report its path. The CSV may be truncated, as with today's `fs::write` (`borders.rs:134`).
- **Review import:**
  - A serde map visitor over a BufReader.
  - ExtSort by (key, ordinal) with keep-last.
  - Nothing is marked before `de.end()`.
  - Marks are applied in key order through the `file_by_path` probe (3.6) inside WriteBatch.
- **Deviations:**
  - (a) A tree that changes between the walks.
  - (b) The `.partial` file while the run is in progress.
  - (c) serde's error text for non-UTF-8 input.
  - (d) Marks are committed in batches.
- **Tests:**
  - Byte-identical CSV against the `cfg(test)` oracle: nested and overlapping inputs, non-UTF-8, junctions, ExtSort budgets of 1-7 records.
  - A walk error leaves the CSV untouched.
  - A copy failure (injected) keeps the partial and reports it.
  - A stop gives the sunk rows in order.
  - Hard links keep their identity after success.
  - The review oracle: last-wins, invalid JSON marks nothing.
  - A scale CLI variant.

### Phase 5: PDQ tier 1 and import ([G1]-[G12], D13, D25, D26)

**Closes:** dedup-01, 02, 03, 15, import-01, 02, 03, scan-09, scan-10, dedup-04, residual (PDQ 4c), [PRX1], [PRX2], [PRX3], [PRX7], [RR1].

**Files:**

- `crates/dedup/src/{layout, canonical, sheet/*, join/*, peel, stop, components}.rs`
- `crates/tier1/src/{store, gc, tail, pair, classify, publish, transfer, account, partners, schedule, maintain, budget, reclassify, planes}.rs`
- `crates/catalog/src/tier1.rs` (the HeavyPartners trait, bounded readers and writers)
- `crates/query/src/lib.rs` (`pdq_near_pair`)
- `import/src/lib.rs`, `import/Cargo.toml`
- runner
- CLI `gallery tier1 maintain|check|rebuild|status`
- crate graph: pg-catalog does not depend on pg-tier1; pg-tier1 depends on pg-catalog, pg-dedup and pg-spill.

- **5.1 Import.**
  - `import()` bumps `meta.pdq_batch` (the session clock, D25) only after `import_work`'s gate passes and the list is non-empty. This matches today's index load, which happens after the empty-list return (`import/src/lib.rs:35-40`).
  - Paged cursor (4.1), store only; the pairing worker runs on `Role::Pair`.
  - Drain is stoppable.
  - Flush on every `Ended`.
- **5.2 Layouts and canonical rules** ([G2]):
  - EVEN31 for equal parity (including odd-odd), FULL31 for different parity, R22 and R22o.
  - `query_canonical` gives ≤ 8 thin emissions per pair.
  - `pair_canonical` gives exactly-once emission with the exact (d, v).
- **5.3 Sheets** ([G3]):
  - Thin sheets: 33 B records, cap 2^29.
  - Fat sheets: about 261 B records, cap 2^26.
  - CRC32C per 4 KiB block on every read; on-disk prefix tables.
  - Fat sidecars: position table plus planes P, G and B, with journaled updates. Plane P mirrors `files.pend_batch`, fed through `pdq_heavy_attr` by `files_pending_on`/`off`. For a heavy file, `off` queues `pdq_heavy_attr` together with its NULL write.
    - `pdq_heavy_removed` copies the plane-P value that the published pairs were formed under. Once attr updates are applied in order, that equals files.pend_batch.
    - Test: revert a heavy file, then run a newer import before the requeue runs; the gone accounting equals the oracle.
- **5.4 Join engine** ([G4]):
  - sparse or sweep mode by measured q*;
  - co-iteration that skips promoted thin entries;
  - thin reducer capped at 4,104 per older file with exact recovery;
  - fat and in-batch exactly-once emission;
  - sinks; unit mode; peel.

  Budget W split: B_batch W/2, probe W/8, match W/8, readers W/8, sketch 16 MiB. W is the one process-wide reservation (§2).
- **5.5 Calibration** ([G5]): IOPS and sequential bandwidth are measured at store open into tier1 meta; q* per store.
- **5.6 Merges and debt** ([G6] + [PRX1]):
  - 4-way tiered ratio-4 merges, with a liveness pass and checkpoints.
  - Bulk build: one extraction, then one sheet at a time. Sorts use only the background share (D14), not G6's 512 MiB.
  - **Run-count cap** = the log4 ladder + 8 capped runs. When live runs exceed it, the pair-lease holder (the CLI included) runs bounded, resumable merge units before its next batch.
- **5.7 Pair worker protocol** ([G8] with D25):
  - P0: select seq > `pdq_paired_hi`. **No pending skip on queries:** today pairs a file at store whatever a later onset does. P0 does not write `meta.pdq_batch` (it is the import-session clock, D25).
  - P1: sweep.
  - P2: classify, with light degree = row count (two range scans with LIMIT 513) plus new partners.
  - P3: rows for pairs with ≥ 1 light endpoint, skipping non-current seqs and those failing `formed` (older.pend_batch IS NOT NULL AND older.pend_batch < newer.batch).
  - P4: heavy run 'staged', `pdq_heavy` rows.
  - P5: stage.
  - P6: publish (CAS `pdq_paired_hi`, `heavy_vis`).
  - P7: transfers in counted terms.
  - A fold applies the stages.
  - Every phase is cursor-idempotent (`meta.pdq_write`).
- **5.8 account_heavy** ([G9]):
  - Gone units run first, then flip units.
  - All accounting is in counted terms.
  - Publish re-reads presence inside IMMEDIATE.
  - Gone rows are acknowledged twice (`acct`, `groups`).
  - Reports show "as of gen G, N pending" while anything is unsettled. `gallery dups` waits, stoppably; `gallery status` prints the note unless `--wait`.
- **5.9 Readers and retention** ([G10]):
  - The pure `pdq_near_pair`.
  - HeavyPartners in pg-tier1, injected by runner and CLI.
  - A `read` lease records `heavy_vis` before the reader's snapshot. Runs are opened at step start (Windows: no FILE_SHARE_DELETE).
  - GC deletes a retired run only when no lease is older than its `retired_gen`.
- **5.10 Scheduling and lag** ([G12]):
  - A batch starts at backlog ≥ B_batch, at import end, or at backlog age ≥ L_lag = clamp(5 × t_sweep, 30 s, 60 min). Sweep duty ≤ 20%.
  - `tier1.max_pair_lag` is set in gallery.json.
  - `reclassify` job for a lowered HEAVY_ENTER.
  - `tier1 status` reports runs, heavy files, unsettled, odd fraction, q* and IOPS.
- **Deviations:**
  - Missing files are ignored when pairs form (Q3). This adds raw `pdq_pairs` rows that today never exist; they become visible through `present_pdq_pairs` when a missing file returns ([PRX3]).
  - A pending revert re-indexes the file (D25).
  - Two imports in two processes no longer miss each other's pairs.
  - Legacy catalogs are re-paired.
  - Reports carry an as-of note while unsettled.
- **Tests:**
  - The brute-force oracle (sample plus planted copies at 0-31 flips, odd weights) at every layer, under budgets 4 KiB/64 KiB/unlimited, with HEAVY_ENTER forced to 2 and 8.
  - Crash points between every transaction.
  - A 100k planted clique: temp O(k).
  - The flip race between two connections.
  - Promotion while a partner's flip is pending.
  - Two gone partners.
  - **Session-clock oracle:** an onset during and before the session; a re-store mid-session; a file re-stored after an old onset pairs as the older endpoint in a later session; a never-pending file (NULL) pairs in SQL and in Rust; an empty or gated import does not bump `pdq_batch`.
  - Retention: a reader paused across merge plus GC.
  - The run-count cap holds after a long import.
  - Crate-graph test: no cycle.
  - dhat ≤ W + 2 MiB; W is never held twice (pair worker, groups peel and merge contend for it).
  - Oracle comparison: (a) raw `pdq_pairs` filtered to the old predicate, (b) `present_pdq_pairs`, with missing-file fixtures.

### Phase 6: Duplicate groups and the Duplicates filter ([r1 dups S1-S8]; [G11]; [S12]; D11, D27)

**Closes:** dedup-05..11, 18, 19, import-09, R2-query-02, R2-cli-07, [PRX4], [PRX11], [PRX17].

**Files:**

- `crates/dedup/src/{components, build}.rs`
- `crates/catalog/{Cargo.toml (+pg-dedup), src/groups/{mod, graph, store, scratch, build, counts, maintain, report, oracle}.rs, src/dups.rs}`
- `crates/query/src/lib.rs` (`duplicates!`)
- `library/src/dups.rs`
- `cli/src/dups.rs`

- **6.1** Edge-granular balanced checks plus a residual per group, capped. GraphReader adds HeavyPartners per frontier batch.
- **6.2 Build.** A disk-DSU build from bitmaps with sorted appends. Heavy adjacency is handled by a seq-ordered RAM-batch peel plus one sweep per batch with plane B ([G11]). The peel takes the process-wide W reservation (§2).
- **6.3 Maintainer.** A single maintainer under the `groups` lease (with lease-named scratch, opened through `connect` as `Role::GroupsScratch`), watermark `groups_gen`.
  - Op-5 heavy adds are batched against plane G.
  - Heavy removals take seeds from the gone dihedral.
- **6.4 Report.** Counters (in `counts`, D22) make the report O(show). The report takes the `groups` lease while paging (D31).
- **6.5 Duplicates filter** ([S12] + [G10]):
  - The hash part: `hash_runs.present − self ≥ 1`.
  - The PDQ light part: `pdq_near.n ≥ 1`.
  - The heavy part: `heavy_near!()`, which is n = near + stage, plus the unsettled correction.
  - Builders wait while more than 4,096 changes are unsettled.
  - **Stamps:** a run crossing present 1↔2 stamps `files.changed` on both affected members, found in O(1) through `present_xor`. `pdq_near` crossing 0↔1 stamps its file. A heavy publish crossing `near` 0↔≥1 stamps that file. So Duplicates views take deltas, and the grid's "always rebuild" exception is removed.
  - **Deviations:**
    - A Duplicates-filtered run reflects a heavy file's presence change when the settle unit publishes, not at the instant of the flip (the read-time correction applies at build time).
    - While a removal is pending, twins under the removed root still count until marked (D34).
- **6.6 dup_log overflow switch.** When `groups_gen − groups_consumed > CAP` (2^22 keys), the next logging statement sets `groups_state='pending'` and logging stops. A rebuild follows, and the maintainer re-checks the state in every write transaction.
- **6.7 Ad-hoc `--distance`** under an `adhoc` lease (non-unique), with lease-named scratch.
- **6.8** The Groups pass is an idle round-robin pass (D17).
- **Tests:**
  - Partition proptests against `oracle_groups` (the parameter grid, HEAVY_ENTER = 2, t ∈ {10, 22, 31}).
  - Step-counter tests: ladder, chain, 1M clique.
  - The two-connection interleaving.
  - Resume inside a 50k run.
  - A cut-vertex re-import.
  - The Duplicates filter equals today's EXISTS forms, including cascade deletes of present and missing files with twins.
  - Stamp completeness: every status flip stamps.
  - A storm drives the overflow switch.
  - Golden stdout for `--show 0/1/100` and `--distance 10/22/31`.

### Phase 7: Grid, browse, tree, Qt edge, edits

**Closes:** grid-F01..F10, F12, F13, F17, R2-desktop-01..06, A6, A7, A9, S2-held, the review's residual 10, [PRX22], [RR4], plus all [QT] items.

**Files:**

- `crates/runs/`
- `catalog/src/browse/read/{changes, facets, grid, tree}.rs`
- `catalog/src/names.rs`
- `catalog/src/range_index.rs` (new, 7.17)
- `app/src/{reader.rs, edits.rs, app.rs, library.rs, views.rs, images.rs}`
- `client/src/{layout, layout/density, layout/provisional, grid, grid/refs, window, window/listview_rules, tree, controller/*, synthetic}.rs`
- `desktop/src/{gallery, shell, filters, facet_choices, settings, animation, video_source, yandex, folders, picker, picker/listing, test_hooks}.rs`
- QML: `WindowView.qml` (`WheelListView.qml` deleted), `GalleryGrid.qml`, `GridRow.qml`, `GridCell.qml`, `Main.qml`, `Viewer.qml`, `Hud.qml`, `MediaView.qml`, `TopBar.qml`, `FilterBar.qml`, `TestHooks.qml`, `YandexPanel.qml`, `FolderTree.qml`, `FolderPickerDialog.qml`, stubs
- `library/src/crop.rs`

- **7.1** Paged ChangeFeed over (changed, id) plus `files_log`.
- **7.2** Facets as chunked steps plus a contribution log. The removal mask is applied path-exact (D34).
- **7.3** Lanes with List coalescing. Every catalog-sourced result carries its snapshot stamp s = max(changed) ([S11]).
- **7.4 build_run.**
  - Single-lower-bound steps of 65,536 examined rows; the sidecar sort removes duplicates.
  - **The post-build delta is a converging paged read** (D31).
  - The removal mask (D34: key-range prefilter plus `folder_under!`).
  - The name driver (7.16) and the range-index driver (7.17).
  - **Sorted views:** a live top-K preview (K = 4096, a bounded heap over rows examined so far, marked provisional) while the build runs. It re-anchors at the same top item when the build lands ([PRX22]).
- **7.5 refresh_run:** uncapped two-phase delta (converging), RankMap, persistence across restarts. A pending removal arrives as a filtered delete over its key range.
- **7.6 Directory** ([QT2]):
  - u128 fixed-point checkpoints, bit-identical across parallel, splice and sequential builds.
  - `density.rs`: k_max, h_min, min_pitch and rho, proven from greedy's closure (rho = 22.91 items/px at 7680 px, T 60, s 4).
  - `Directory::periodic` is compiled unconditionally.
  - **ProvisionalGeometry** and a re-anchor that keeps contentY.
- **7.7 Client grid** ([QT3] + [U11] + [S11]):
  - Rank everywhere; u64 versions.
  - **RefBook** (≤ 2 retained RankMaps, proven bound).
  - The overlay folds stamped Pendings until s ≥ c.
  - `held` becomes a pinned run handle.
  - `position_of`, `entries` and full relayout are removed.
- **7.8 SlotWindow** ([QT4]):
  - Synchronous slot assignment for viewport ± 1500 px.
  - Slot capacity from the density bound.
  - Slab rebase only at idle and only beyond 2^40.
  - ListView rules ported from the Qt 6.12 sources (current remap, keys, follow, Contain/Beginning, reset), pinned by `golden/listview_rules.json`.
  - The bar's seek is coalesced per frame.
- **7.9 Desktop Qt edge** ([QT5]):
  - f64 properties; `viewedFile` string.
  - `loadedAnchor`; the `lastFileIndex()` invokable; `save_last_file(Option)`.
  - ItemRef stamping on every queue hop.
  - `to_i32` and `qt_int` deleted; i32 conversions carry a `qt-edge-bound` reason.
- **7.10 QML WindowView** ([QT6]):
  - A plain Flickable plus a non-moving slots layer plus a Repeater.
  - Typed scalar signals.
  - doubles throughout; `'f',0` formatting; `file` strings.
- **7.11 Folder tree** ([r1 grid S15] + [QT7]):
  - The streaming per-generation tree files.
  - FolderModel as a `SlotWindow<Uniform>`.
  - Model-owned `currentRow` with ported rules; native attached bar.
  - Row LRU ≥ 2·S.
  - The tree is merged across drives (`browse/read.rs:1`). A pending removal subtracts its folders' counts from merged nodes and hides only nodes left empty.
- **7.12 Folder picker** ([QT8]):
  - In RAM up to 4 MiB of names, synchronous.
  - Beyond that, spill-sorted on a worker into a PagedFile with offsets, with generation cancel.
  - Slot model.
- **7.13 Crops:** a streamed list with a prepass refusal and unordered resume.
- **7.14 Live pages.**
- **7.15 Edit worker and pools** ([S11]):
  - The `Edits` worker (FIFO 1024) runs Rate, Turn, BorderMark, SaveListing, SaveView, DeleteView, AddRoot and RemoveRoot through the Gate. Its ack carries the commit stamp c.
  - The `ui` pool is query_only.
  - `App::stop` drains the queue.
  - Deviation: an edit shows at once and reverts if the write fails.
- **7.16 Name index** ([S13]):
  - The `names.sqlite` FTS5 trigram file (contentless, detail=none), names padded with two U+0001, opened as `Role::Names`.
  - Fed by the ChangeFeed in the `Names` idle pass (D17).
  - Needles of 1-2 characters go through `fts5vocab` term-range prefix expansion.
  - It is used when Σ doc < scope/8; otherwise the build scans.
- **7.17 Opt-in range indexes** (Q-G1 a, decided; [RR4]):
  - `gallery.json grid.range_indexes` lists any of size, mtime, taken, megapixels, duration.
  - **Build.** Each listed column is a 2.3 Shadow access path `sh_<col>(value, id) WITHOUT ROWID`, built online as an Upgrade step (Phases R, M, L) in the background share. A plain CREATE INDEX is ruled out: the freeze guard refuses it on LARGE_TABLES, and it would make `mismatch()` non-None and force a rebuild.
  - **Upkeep.** After ready, the shadow is maintained by triggers whose names are covered by `difference_allowing`, so `mismatch()` stays None.
  - **Readers.** build_run and facets use the shadow when the range's candidate estimate is below scope/8 (as 7.16 does). Candidate ids are ExtSorted by the view key, then filtered by scope, the removal mask and the other predicates. Results are identical to the scan.
  - **taken.** The taken step (2.5) marks `sh_taken` not ready and rebuilds it afterwards; readers scan meanwhile.
  - **Opt-out.** The triggers and the table are dropped in one DROP TABLE: reads O(size), WAL about pages/1000.
  - **Cost:** 20-30 GB per column at 1B; the build takes 1-2 h online.
- **Tests:**
  - `ids(build_run) == ids(list_grid_reference)` across SortKey × direction × query (Duplicates, name needles of 1-5 characters, the removal mask on mirrored drives, each range index) × scope × rules.
  - VM_STEP per step is O(step).
  - The delta proptest with 50 chained refreshes at page size 1, with writes between pages.
  - Duplicates deltas.
  - The window, grid, tree and density laws at 2^31 + 2^20 + 7 and 2^40.
  - The RefBook proptest.
  - The golden ListView table, reproduced in Rust and by a real ListView in qml_host.
  - `tst_windowview`, `tst_gridrow`, `tst_viewer`, `tst_media_view`, `tst_foldertree_rules`, `tst_folder_picker_dialog`.
  - Picker order equals `PathBuf::sort`.
  - Overlay folding: a turn plus `refresh_turned`, a rating plus a delta, an out-of-order arrival.
  - Crop refusal writes nothing.
  - Name index: lag, tombstones, reset.
  - Range indexes: results equal the scan for every column, direction and scope, both during a build and after; opt-out and opt-in round trip; `mismatch()` stays None; `set_rating` ≤ 150 ms during the build.

### Phase 8: Verification harness, ledger, lints, docs

- **Files:**
  - `crates/scale/{src/bin/{synth, sql_ledger}.rs, tests/scale.rs, expectations.toml}`
  - `catalog/src/fixture/synth.rs`
  - `testkit/src/{alloc.rs, ntfs.rs}`
  - `scan/tests/{usn_oracle, usn_faults, usn_vhd}.rs`
  - `desktop/tests/qt_edge_lint.rs`
  - `tools/compare-shots.ps1`
  - `tools/shots/asserts/*.json`
  - `docs/perf/sql-plans.toml`
  - `Justfile` (`usn-vhd`)
  - `.github/workflows/{ci, scale}.yml`
  - `docs/{SCALE (new), DATABASE, PLAN, STORAGE, HEAP_PROFILE, REVIEW, REFACTOR_PLAN}.md`
  - `crates/README.md`
  - `.env.example`
- **8.1 Generator.** It produces every trigger-maintained value, each checked against its recount oracle ([PRX21]):
  - `folders.present`, `folders.anchor`, `anchors`, `borders_due`, `pend_batch`, `unpaired_anchor`, `file_seen`;
  - `pdq_indexed` with batch;
  - the `counts` row, `hash_runs`, `pdq_near`;
  - `view_epoch`, `catalog_opens`;
  - for the journal rows: a `usn_dirs` mirror, census and `usn_roots` consistent with a FakeVolume tree.

  `Spec` gains a planted near-duplicate dihedral distribution and a multi-drive mirrored layout. Generation runs in a child process.
- **8.2 Ledger** (`just plans`):
  - statement tags;
  - class, temps, fan-out and total order;
  - "folders.path, folders.key and folders.anchor are immutable" (no UPDATE OF those, no DELETE; trigger bodies checked too, [PRF15]);
  - every statement that writes `unpaired` or `folder_id` also writes `unpaired_anchor` (D9);
  - the **read-transaction census** (max ≤ READ_TX_MAX, offenders listed);
  - the missing-writer allowlist (3.8);
  - open findings.
- **8.3 Lints:**
  - `qt_edge_lint` ([QT9]);
  - autoindex, FK-index, no-upsert-on-analysis and no-REPLACE (2.1);
  - partial-index literal terms;
  - `pending_now`/`taken_now` usage;
  - `no_memory_temp`;
  - scans readers use `id > 0`.
- **8.4 Assert scenarios** ([QT9]):
  - `--synthetic-list`, `--dump-state`, `--bar-at`;
  - `$Asserts` in compare-shots, run on the working tree only, failing on mismatch: `huge-list` at 2,148,532,231 and `huge-keys` at 4,294,967,303.
- **8.5** The scale harness and matrix are in §8. Docs updated, including the test-layer split and the journal operations guide (`gallery journal`, fsutil, Q-S5).
- **8.6 Calibration** of the 3.9 cost-rung constants c_e, c_d, c_p and c_o, from the scale runs, into `expectations.toml`.

## 5. Review outcomes

### Round 1 (unchanged summary)

| Area | Verdict | Changed | Rejected by the designer |
|---|---|---|---|
| pdq-index | needs-changes (1 blocker, 11 major) | Batched external join, leases, legacy seqs, dead counts | Liveness snapshot per analysis, AUTOINCREMENT ids. **Round 2 superseded both the design and these rejections (G).** |
| scan-pairing | needs-changes (6 major) | `file_seen`, Gate, durable removal, outbox | Per-root partition. **Overturned in round 2 (D9), and corrected in round 3 to (anchor, key range).** Stop between pairing pages. **Now decided (3.2).** |
| work-lists | needs-changes (6 major) | Cursors, FolderCache, `assert_bounded` | Borders report stays O(inputs). **Overturned (4.8).** |
| grid-browse | needs-changes (3 blockers, 8 major) | `files_log`, single-lower-bound rule, rank sidecar | "`changed > ?last` suffices" (rejected, still holds) |
| dups-report | needs-changes (2 blockers, 8 major) | Coalesced log, single consumer, O(show) | Seeding `done`; writer-side draining (still holds) |
| sqlite-harness | needs-changes (9 major) | Materialize-then-page, sub-journals, per-role caches | VM_STEP via trace; `remove_root` per folder. **Overturned by F1 per-row pages.** |

### Round 2: designs

| Design | Verdict | Issues that changed the design | Rejected by the designer (judge these) |
|---|---|---|---|
| gap-folder-residuals (F1-F6) | needs-changes, all 17 accepted | Validate-then-relist (no entry spill); typed OutOfSpace per Q4; heartbeat thread plus token fence; rollback-journal source with change counter; `attach_uri`; WAL byte budget; authorizer allowlist; migrating flag; indexes dropped before batched delete; removal with no lease | A removal lease ("work_leases has no target; the CLI would block for hours"); `immutable=1` source; keeping the old file at its path; F6 copy-without-delete (+1x disk) |
| gap-thumb-eviction (TE1-TE6) | needs-changes, 16 accepted | Gen-keyed state; interval members plus compaction; laps; live resolution; heartbeat lease; timed runner retry; opens log; tombstones and revalidation | Sweep cursor t (laps keep catalog changes prompt); per-member gap counter (it goes stale); catalog witness (checks one row); durability barrier / synchronous=FULL; skipping decode at cache-full. **Overridden by ordered batches plus the memo; the user took Q-T4 (a).** |
| gap-pdq-structure (G1-G13) | unsound, then revised (25 accepted) | Fat heavy sheets with exactly-once emission; counted terms; IMMEDIATE re-read publish; idempotent phases; pend_batch; legacy re-pair; deg dropped; pure-SQL filter; lease-before-snapshot retention; ConcurrentReader; 4-way merges; block CRC | Per-run heavy bitmap (n/8 RAM); query-major resolution (about 2 PB); witness rows plus a function reading runs; signed correction stores; demotion; WAL-checkpoint retention; history-free pairs |
| gap-qt-cap (QT1-QT9) | needs-changes, 18 accepted | Density bound; synchronous slots (no lead L); WindowView with numeric rebase; typed scalar protocol; ported ListView rules plus a golden table; ItemRef/RefBook; picker windowed; text-dump asserts | Owning flick inertia (private Qt logic, `qquickflickable.cpp:1150-1170`, `:1498-1531`); a ScrollWindow QObject property; model-driven bars for all views; capping L |
| audit-write-start (U1-U13) | needs-changes, all accepted | Fixed seeded keys; tri-state flags plus a files_present_order walk; per-family cursors; classified rebuild errors and salvage; hierarchical parking; ten-probe `next_ready`; shadows; converging reads | A durable sort for bulk_due (the walk is cheaper); id-order shadow backfill (1B random inserts) |
| audit-shell-cli (S1-S17) | needs-changes, all 16 accepted | Fenced carry; FK merge; ExtSort derived state; spilling reorder; overlay with stamps; dedup eviction log; padded trigram; Versions off prepare | A stepped in-place recount (S1: "never needed"). **Partly overturned: D22 keeps cursors for post-freeze migrations.** `carried.locking_mode` (the alias-path WAL index differs) |

### Round 2: the synthesizer's cross-design resolutions (judged in round 3)

| Conflict | Chosen | Rejected | Evidence |
|---|---|---|---|
| Rebuild: F3 (move aside) vs S7 (write fence, keep path) vs U5 (two-hop S) | F3, plus U5 error classes and salvage, plus S7's P3 ExtSort derivations and pairs filter | S7's fence: the moved file is kept by F3, and F3's `journal_mode=DELETE` refusal proves no other connection exists at that instant. So no write can be lost before Swap: any commit to kept bumps its change counter, which gives KeptChanged and a restart at Copy. After Swap, a descriptor opened without prepare in the close-to-rename window is today's residual (`move_aside`, `lib.rs:347`) ([RR11]). U5's two-hop S: D24 turns every out-of-order UNIQUE into a droppable index. | `schema.rs:175-221`; `lib.rs:219-279` |
| Version recomputes: U1/U2 vs S17 | U1/U2 (exact reads, no index rewrites, gates in the API) | S17 (stale desktop values while it runs; a pending batch rewrite through live indexes) | `lib.rs:363-433` |
| Tier-1 gate and backlog drain during the import step (U1) | Removed (and removed from `with_upgrade`, [RR10]) | D25's session clock makes pairing results independent of the step | Today's index load happens at import start (`lib.rs:881-913`) |
| Pairing clock: G1 batch start | Import-session start, stamped at store; `pend_batch` on files, **cleared to NULL whenever pending returns to 0**, with G's NULL guard restored and owned by `files_pending_on`/`off` ([RR1]) | Batch start (a deviation whenever an onset happens mid-import) | `lib.rs:881-913` loads every present, non-pending analysed file |
| G8 P0 "skip queries with pend_batch" | Dropped | Today pairs a file at store whatever a later onset does | |
| Borders report: F2 (two listings) vs S5 (one walk plus a 100-150 GB ExtSort) | F2 plus S4's non-blocking reorder | S5 (scratch at 1B); F2's blocking window (head-of-line stall, the shell-cli review) | |
| CSV output: F2 (partial plus copy in place) vs S5 (Staged rename) | F2, with the partial kept on a copy failure ([RR9]) | A rename drops ACL and hard links | `borders.rs:134` |
| Thumbnail heal: TE2 (snapshot in open) vs U9/S9 (chunked on the writer) | U9 | TE2 still costs O(entries) at startup (T3) and pins the WAL past READ_TX_MAX | `thumbcache lib.rs:386`, `:1113` |
| Thumbnail eviction log: TE `lost` vs U8 recheck vs S9 `evicted` | TE `lost` (trigger-logged, live resolution) | U8 and S9 logs (TE's resolution makes logged replacements harmless and fixes stale on-demand overwrites) | |
| Evict walk cost | U8/S9 merge plus short-circuit added to TE (TE kept victim selection unchanged) | | `thumbcache lib.rs:1070-1080` |
| Cache migration: U8/U12 online in place vs TE start-over | TE start-over now (Q-T5); U12 for later changes | | |
| Picker: QT8 vs S14 | QT8 (with S14's generation cancel) | | |
| Status pairs counter: U3 total minus touching-missing | Σ bands (Phase 6 already maintains present pair counts by band) | U3's extra counter and `files_missing` index | |

### Round 2: plan-review issues

Every issue is listed. "Applied" points at the step.

| Issue | Disposition |
|---|---|
| PRF1 thumbnail tiers | Applied (4.4-4.7) |
| PRF2 stale!() / version written last | Applied (2.5) |
| PRF3 carry_rows | Applied (2.2) |
| PRF4 PairBacklog starvation | Applied differently: D25 makes pairing timing-free; D17 fairness. Round 3 also slices Removals after Scans ([RR7]). |
| PRF5 global unpaired | **Applied with an (anchor, order_key) key.** A root is the pair (anchor, key range), because folder keys omit the drive (`rules.rs:196-215`). `unpaired_anchor` never needs re-keying, since a folder's anchor is immutable, whereas the reviewer's `unpaired_root` would need re-keying on add_root absorption. So the reviewer's column is replaced, not rejected. The exact `under_root!` path test keeps the scope equal to today's ([RR2]). |
| PRF6 D6 not exact | **Reject "make exact":** exactness needs an O(n) start snapshot (D5) or a pass-long read snapshot (D31). Applied as a stated deviation (D6, Q-D). |
| PRF7 removal visibility | Applied as a path-exact mask (D34), with status corrections and as-of notes ([RR6]). **Reject the 'removal' lease:** F1's per-row recheck, the hidden counter committed with its marks, and the forward watermark make concurrent pagers exact; the CLI reads the finished row. |
| PRF8 pruning and scan residue | Applied (3.7) |
| PRF9 phase order | Applied (1.3/1.5, → marks) |
| PRF10 set_counts | Applied (removed; D22) |
| PRF11 temp peaks | Applied (§2) |
| PRF12 roles | Applied (1.2) |
| PRF13 retry snapshot | Applied (4.1) |
| PRF14 rescan reads, pairing CPU | Applied (§2, R-SCAN). The USN journal is now decided: 3.9 (Q-S1 b). |
| PRF15 folders rule | Applied (8.2) |
| PRF16 alias / temp dir | Applied (D21) |
| PRF17 borders report | Applied (4.8) |
| PRF18 D1 rationale | Applied |
| PRF19 removal stamp order | **Reject the re-order:** the stamp order is internal (change-feed cursors only). Stated as an internal deviation (3.4). |
| PRF20 stop during pairing | Applied (3.2) |
| PRF21 Phase 0 wording | Applied |
| PRX1 run count / Groups starvation | Applied (5.6, D17) |
| PRX2 9 PB | Applied (G: hash-inline sheets, about 370 TB; R=22 text removed) |
| PRX3 Q3 deviation | Applied (Phase 5 deviations, oracles) |
| PRX4 Duplicates rebuilds | Applied (6.5 stamps), with the heavy-settle lag stated |
| PRX5 adoption | Applied (D16) |
| PRX6 verification matrix | Applied (§8) |
| PRX7 PDQ scenario | Applied (§8 split) |
| PRX8 eviction | Applied (TE) |
| PRX9 carry | Applied |
| PRX10 scratch ledger | Applied (1.6) |
| PRX11 dup_log | Applied (6.6) |
| PRX12 stale residuals | Applied (§6 rewritten) |
| PRX13 pairing stop | Applied |
| PRX14 residuals 9, 10, 3 | Applied: counter heartbeat (R9 remains only across OS boundaries); converging delta; trigger stamps |
| PRX15 Budget priorities | Applied (D14) |
| PRX16 Groups scratch role | Applied |
| PRX17 adhoc Lock | Applied (6.7) |
| PRX18 set_counts at prepare | Applied (removed) |
| PRX19 folders rule | Applied |
| PRX20 lag and id widths | Applied (G12; 1.7 const asserts) |
| PRX21 synth values | Applied (8.1) |
| PRX22 sorted interim | Applied (7.4 top-K preview) |

### Round 3: USN-journal design (USN1-USN8)

Verdict: needs-changes. 21 issues, all accepted by the designer and applied in 1.6, 2.1, 3.1-3.9 and §6-§8.

| Issue | Severity | Disposition |
|---|---|---|
| UR1 access hash changes at every logon, so every reboot walks | blocker | A logon-independent `access_projection` (drops only SE_GROUP_LOGON_ID; logon-type SIDs kept on purpose). A change runs the O(dirs) R-A re-probe, not a walk (R-USN3). New rung: SeChangeNotify enabled. New deviation J-e. |
| UR2 probe_entry reuses a handle whose FileName filter is fixed | major | A new handle per probe; loop to an exact UTF-16 match. A miss contradicting `open_by_id` relists and is never absent. |
| UR3 wrap rung checked LowestValidUsn instead of FirstUsn | major | LowestValidUsn must be equal; FirstUsn ≤ fp_usn; the fingerprint record must be read first; a re-query after R1. |
| UR4 staleness tracked only for kind-1 identities | major | One mechanism: every affected media ref is opened by id (except provably single-link closed creates); per-root `usn_stale` until values agree and CLOSE is seen. J-a narrowed. |
| UR5 drops marked in (scan, file_id) order | major | `usn_drop` keyed by (FolderPages key, id); one merged stream with scopes; stamps equal the walk's. |
| UR6 folder rename moving 1B descendants: 150 GB scratch, 8-17 h of opens | major | Moves under a DropSubtree skip opens; SubWalks run in place in W2 (zero scratch); cost rung includes the moved-subtree census; scale row 0.5·N. |
| UR7 walk-visible failures with no record (sharing, corruption, device) | major | Deviation J-d; Q-S5 periodic verification walk; J-b lists chkdsk and self-healing. |
| UR8 HARD_LINK_CHANGE parent assumption; 4-parent cap | major | `links()` for HARD_LINK_CHANGE, multi-parent and kind-1 refs; uncapped (ref, parent) ExtSort; the multi-link rule applies to every intent; FakeVolume parent ambiguity. |
| UR9 dirty set D outside the RAM bound | major | D is an ExtSort in pre-order with an O(depth) skip stack; buffers counted (≈ 140 MiB) and asserted. |
| UR10 the unprivileged FSCTL may filter records | major | Per-build `unpriv_selftest` rung; elevated `just usn-vhd` release gate with second-user mutators. |
| UR11 the watch's feedback loop on its own writes; volume lock blocked | major | Relevance filter; own-file exclusions; overlapped waits; VolumeGuard device notifications. |
| UR12 $UpCase may differ from RtlUpcaseUnicodeChar | minor | Probe mode only when the name order is table-independent (ASCII or a single name); else relist. |
| UR13 relist discarded unaffected entries' values; count formula | minor | A relist replays every entry plus a folder-only scope; unexplained differences queue a verification walk. |
| UR14 usn_stale not keyed by root; table classes | minor | Keyed (root, volume, ref); LARGE; `usn_scope` LARGE. |
| UR15 directory security changes not re-checked | minor | Such directories enter D and get `first_call`; the root is checked with `first_call` in R0. |
| UR16 a killed walk leaves a stale cursor | minor | begin_scan of a walk deletes `usn_roots`; finish writes it. |
| UR17 recurring per-pass costs and a volume-wide crossover | minor | Cost stated (R-USN4); kind-3 is event-driven through `usn_wake` triggers; `scan_skipped` is not rewritten; the crossover counts relevant records only. |
| UR18 detachable disks reporting fixed buses; dual-boot | minor | Hotplug and removable flags; J-b reworded. |
| UR19 journal wrap during the base walk | minor | No cursor stored; a notice with the needed size. |
| UR20 resume replays old listings; adoption copies tens of GB | minor | Rename handover; 24 h bound with redo from the same cursor; J-c extended. |
| UR21 oracle coverage | minor | Mixed schedules, pairing continuation inside J, other-user and denied mutators, open sessions, POSIX deletes, parent-ambiguity mode. |

**Rejected by the USN designer (judge these):**

- OpenFileById live values as the truth (the walk records directory-index values);
- a links table (changes walk semantics);
- stamping every seen row (O(n) WAL per pass);
- ids in record order;
- sealing sub-walk entries;
- opens for every moved entry;
- hashing the whole token;
- dropping logon-type SIDs;
- handle reuse or NtQueryDirectoryFileEx for probes;
- the per-folder count check;
- polling every Foreign identity;
- rewriting `scan_skipped`;
- reading $UpCase (not openable unprivileged);
- base-time link counts (admin-only or O(n) opens);
- an elevated helper process;
- one cursor per volume;
- ReturnOnlyOnClose or reason filtering;
- trusting detachable buses by default;
- auto-resizing the journal;
- carrying `usn_*` through a rebuild;
- an unfiltered watch.

### Round 3: review of the round-2 resolutions

Verdict: needs-changes (2 blockers, 5 major, 4 minor). Every issue was resolved:

| Issue | Severity | Resolution |
|---|---|---|
| RR1 `pend_batch` moved to files without its clearing rule; NULL guard dropped; two stamping triggers | blocker | D25, D8, 2.1 triggers, 2.4, 2.5 Pass A, 5.1, 5.3 plane P, 5.7, 1.7, tests |
| RR2 an order-key range is not a root (drives merged) | blocker | D9 (anchor, key range) + path test; D34; 1.5; 2.1 (anchors, folders.anchor, unpaired_anchor); 2.2; 2.4; 3.2; 3.4; 3.6; 7.2/7.4/7.5/7.11; Q-S2; §2; §8 |
| RR3 Q-S1 (b) taken but no step | major | 3.9 (full revised USN design) and its edits |
| RR4 Q-G1 (a) taken but no step | major | 7.17 |
| RR5 the whole-process sum did not add up | major | §2 derivation; D14; 5.6; §8 PDQ RAM |
| RR6 the removal mask corrected only the present count | major | 3.4 status rule; D34; Q-D |
| RR7 Removals unsliced ahead of Scans; Names missing | major | D17; 3.4 |
| RR8 Q-D omitted the memo and borders deviations | minor | §7 Q-D |
| RR9 a CSV copy failure loses both files | minor | 4.8 |
| RR10 `with_upgrade` still drove tier1; Names had no role | minor | 2.5; 1.2; D3 |
| RR11 the rebuild row overstated "cannot arise" | minor | §5 rebuild row |

**Plan edits RE1-RE26:**

| Edit | Disposition |
|---|---|
| RE1 D25 formula and clearing | Applied |
| RE2 D8 trigger ownership and bulk scope | Applied |
| RE3 2.1 pdq triggers | Applied. Integrator change: 2.5 Pass A no longer writes `pend_batch` inline, so only the triggers own it (no double write). |
| RE4 5.7 guard; P0 does not write pdq_batch | Applied |
| RE5 2.4 tests | Applied |
| RE6 D9 (anchor, key range) | **Applied with two additions.** (1) `folders.anchor` is a column, so `unpaired_anchor` can be written in SQL by every writer: displace touches rows in any folder (`lib.rs:1219`), and the upsert's DO UPDATE moves `folder_id` while keeping `unpaired` (`lib.rs:1223-1227`). (2) Rows read from the range are filtered by the exact `under_root!` path test, because `folders.path` compares bytewise (`under_params`, `lib.rs:99-115`): (anchor, key range) alone would include case-variant folders that today's scope excludes. |
| RE7 2.1 indexes, anchors | Applied (plus `folders.anchor`) |
| RE8 3.4 root_removals anchor and FolderCache anchor mask | **Modified:** the mask is the key-range prefilter plus the exact `folder_under!` path test, for the bytewise reason in RE6. `root_removals` needs no anchor column. |
| RE9 3.4 grid/run/tree mask text | Applied with the path test of RE8 |
| RE10 3.4 status | Applied, with the field names corrected: `Status.files` is the present count (`lib.rs:1067`), so present = `counts.present` − Σ and missing = (`counts.files` − `counts.present`) + Σ. |
| RE11 PRF5 row | Applied |
| RE12 Q-S2 | Applied |
| RE13 §2 Pairing row | Applied |
| RE14 derivation (defaults) | Applied, without its stray "Superseded wording follows:" line, and with the journal pass term added |
| RE15 derivation (minimum) | Applied |
| RE16 D17 | Applied |
| RE17 Q-D additions | Applied |
| RE18 4.8 CSV | Applied |
| RE19 2.5 CLI | Applied |
| RE20 1.2 Names role | Applied, plus a Watch role for the USN watch's reader; re-summed to 884 MiB |
| RE21 rebuild row | Applied (`move_aside` is at `lib.rs:347` today; cited by symbol) |
| RE22 §1 decisions | Applied |
| RE23 Q-S1 | Applied (recorded as decided) |
| RE24 R-SCAN | Applied, merged with the USN design's narrower list (local fixed NTFS; ReFS walks) |
| RE25 3.9 text | **Rejected in favour of the reviewed and revised USN design (USN1-USN8).** Evidence: its `folders.volume/file_ref` identity cannot map a USN parent ref for directories without media, because `folders` rows are created lazily at a folder's first media file (`crates/scan/src/lib.rs:300-301`). The revised design keeps a separate `usn_dirs` mirror of every directory. Its `folder_notes` is superseded by the census; its LIFO hard-link precedence by walk_key plus the inline winner rule; its `scan.full_every` by Q-S5 `journal.verify_days`; and its "else a volume handle" by the elevated-only privileged rung (the self-test gates the unprivileged read). |
| RE26 7.17 | Applied |

## 6. Residuals: only what is proven impossible to do better

1. **R1. Rebuild disk ≥ size(old) + size(new), about 2 TB at 1B.**
   - The old file is kept whole (`lib.rs:143-145`) and the new one holds every carried row, so two copies coexist at the swap.
   - Only an in-place migration avoids this. After the freeze that is RebuildTable (2.3). The BASELINE route is the user's decision.
2. **R2. Rebuild time Θ(n log n), 6-12 h on NVMe; resume loses at most one bulk statement (one CREATE INDEX, ≤ about 30 min).**
   - The row layout changes, so every row is rewritten: Ω(n).
   - Building a B-tree index over unsorted keys needs Ω(n log n) comparisons.
   - SQLite fills an index only through one CREATE INDEX statement; an online build through the WAL (U12) doubles I/O in a pass that is already exclusive.
3. **R3. Huge-folder frame scratch Θ(pending subfolders), only above 16K subfolders, about 8-20 B per frame.**
   - The walk pops frames LIFO (`scan lib.rs:236`, `:292`), so it visits subfolders in reverse listing order.
   - Reversing an m-item one-pass stream with M words needs p·M ≥ m passes. Re-listing 1B entries 61K times is infeasible.
   - An in-order DFS would change row ids and hard-link winners.
4. **R4. Borders report: scratch O(entries) for a single directory above the sort budget.**
   - Munro-Paterson: sorting n keys from read-only input in p passes needs p·M = Ω(n).
5. **R5. A missing toggle costs Θ(degree) reads, and `pdq_near` and band upkeep cost Θ(degree).**
   - A toggle changes the presence of every incident pair.
   - Exact O(1)-report counters must move by that many. Any per-file pre-aggregate moves the same work onto the partners. This is also why a pending removal shows pdq_pairs as-of (Q-D).
6. **R6. CREATE INDEX holds one page buffer per PMA: index/32768, which is 1.6 MiB at 1B.**
   - vdbesort opens every leaf PMA reader at merge time, and maps temp files only below 2 GiB.
   - The heap P + P + index·pgsz/P is minimized at Θ(√(index·pgsz)); no fixed P makes it constant.
7. **R7. A borders CSV run in progress holds up to the new CSV's size beside an existing CSV.**
   - Today's contract is "untouched on error" plus "replaced in place on success". With no RAM buffer the new bytes must be stored outside the old file until success.
   - Truncating up front breaks the first half of the contract (Q-B1).
8. **R8. Cold-cache removal and mark_missing cost Ω(uncorrelated indexes − 1) random reads per row: 3-80 h on NVMe, possibly months on cold HDD.**
   - SQLite B-trees have no batched update. Any order is sequential in at most one index.
   - Dropping and re-sorting the indexes would make the catalog unusable for hours, which contradicts the online removal of D34/Q7.
9. **R9. Cross-OS (Windows/WSL) lease exclusivity under an unbounded pause holds for at most one in-flight transaction.**
   - No shared atomic primitive exists across the boundary (`schema/src/lib.rs:136-139`).
   - Gray-Cheriton: a lease is safe only under bounded pause and drift.
   - The token fence, pause detection, counter heartbeat and kept change counter narrow it to one transaction.
10. **R-SCAN. A full walk reads every listed entry plus one catalog identity probe per entry (200-400 GB, 1-4 h at 1B).**
    - **When it still runs:** step 3.9 uses the USN journal on local fixed NTFS volumes, so a full walk runs only in the fallback cases:
      - a root's first scan;
      - a 3.9 ladder fallback: journal inactive, wrapped, recreated or rolled back; a fingerprint, rules or collation change; an unverified unprivileged read; SeChangeNotify disabled; a cost-rung crossover;
      - a Q-S5 verification walk;
      - a requested `scan --walk` or Full rescan;
      - Linux/WSL roots, network shares, ReFS and other filesystems, and detachable volumes;
      - the first scan after a rebuild (`usn_*` is not carried).
    - **Why:** without a filesystem change log, a change can be any entry. An adversary argument shows every entry must be listed and compared with its stored state, and the probe is the minimal comparison.
11. **R-PAIR. First-scan pairing costs about 20-25B B-tree probes.**
    - Each fresh row needs its tier lookups, and each lookup is an index probe. The tiers' candidate sets have no common order (identity, folder/name, file_ref), so the probes cannot be merged into fewer sequential scans without O(n) sorts per tier, which costs more.
12. **R-PDQ1. Pairing CPU of about 26 core-days at 1B.**
    - 8 × 32-bit chunks minimize probes plus accidental candidates.
    - A subquadratic exact Hamming join at d ≈ 8.5 log₂ n would refute SETH (Williams 2005; Alman-Williams 2015).
13. **R-PDQ2. k² CPU (not storage) for dense clusters, about 360 ns per near-copy pair (a 1M cluster is about 50 core-hours).**
    - Band counts as a function of the present subset determine every pair's band: Θ(k²) bits.
    - o(k²) storage must recompute, and one flip needs Ω(deg).
14. **R-PDQ3. About 18 KB of disk per heavy file.**
    - Stateless exactly-once emission needs the full dihedral at all 64 sort positions.
    - The alternatives are Θ(pairs) state or Θ(pairs) random reads.
    - HEAVY_ENTER = 512 equalizes the worst light and heavy cost.
15. **R-PDQ4. Pair lag ≤ L_lag + t_sweep; lone-image latency is bounded by distinct probe blocks (about 1.9M at 1B).**
    - Above q*, one sequential read of the index is the cheapest exact read.
    - Fewer runs would need merge transients above 146 GB.
16. **R-PDQ5. Exact PDQ counts while heavy presence changes are unsettled need Ω(Σ deg) work.**
    - Hence the as-of note or a wait (5.8). Today's exact `stats()` is itself O(rows).
17. **R-TH1. A height with no usable state walks N rows once (14-28 h at 1B), resumable.** This covers a new height, a start-over, a rollback, a fork or the abort rule.
    - Nothing records which files lack a current entry at that height, and the walk is proportional to its own output (the decodes).
    - The abort rule is ≤ 2× the cheaper of its two options.
18. **R-TH2. The progress total is unknown while a walk, range or revalidation is open (Q-T6).**
    - A range's needed count is a join across two databases with no subtree counts.
19. **R-TH3. Compaction re-walks ≤ N/2 rows per compacted height per lap.**
    - Pigeonhole: more than B arbitrary keys in B members must lose information. Smallest-gap merging minimizes the loss.
20. **R-TH4. Fork detection fails with probability 2^-63 per fork.** Any bounded detector is a fingerprint.
21. **R-UP1. A version bump writes Ω(n) flag pages, about 1-2 h sequentially.**
    - The flag lives in every row. A version-relative flag would need a permanent 30-40 GB index and could not be expressed in the `files_borders_due` partial index.
22. **R-RD. A converging read's final iteration is bounded by the rows changed during the previous iteration.**
    - An exact snapshot must read every changed row in one snapshot. U7's ceiling slows the batch writers that would prevent convergence.
23. **R-QT1. Flickable contentY drifts between idle frames; the double keeps 2^-11 px up to 2^40 px.**
    - Reaching that takes about 4.6 years of continuous maximum-velocity flicking.
    - A running Flickable timeline cannot be translated (`qquickflickable.cpp:406-412`).
24. **R-QT2. Scrollbar resolution is about n/h items per bar pixel.** This is inherent to any scrollbar. Exact reach is kept through wheel, keys and reveal.
25. **R-QT3. Delegates per row are set by geometry, as today: up to about 171K cells at 7680 px, T 60, 1:20 strips.** Only a product change to `clamp_aspect` or the minimum row height would lower it.
26. **R-Q1. Unindexed predicates cost O(scope) per query edit; every filter costs Θ(matches).**
    - An adversary can make any unexamined row match.
    - Per-column range indexes are opt-in (Q-G1 a, 7.17); columns not opted in keep this bound.
27. **R-RO. The ordered-sink spill holds up to the full output size in the extreme.**
    - A row cannot be emitted before every earlier row exists, so the later rows must be stored somewhere.
28. **R-USN1. A journal pass lists a dirty folder of m entries in full (O(m)) in four cases:**
    - one of its changed files denies FILE_READ_ATTRIBUTES to OpenFileById;
    - a probe contradicts `open_by_id`;
    - more than one of its intent-carrying names (files, or separately subfolders) is non-ASCII, since NTFS $UpCase tables can differ outside ASCII and the volume's table cannot be read unprivileged;
    - m ≤ max(4096, 64k), where listing is the cheaper choice.
29. **R-USN2. Changes beyond the journal's MaximumSize between two passes, or during a base walk, force a full walk.**
    - The journal is the only record of them.
    - Resizing it needs an administrator. The app offers a consented resize through the helper, sized from the measured record rate (3.9 USN6). Declined offers and non-admin users fall back to the walk.
30. **R-USN3. A change of the token's access projection costs an O(dirs) access re-probe of the root's directories (about 5-25 min at 10-50M directories) instead of a walk.** A changed projection means a different user, group set, elevation, integrity level, or SeBackup/SeRestore state.
    - Access is evaluated per directory against the token. No record reports a token change.
    - Alternating an elevated and an unelevated scanner pays it at each alternation.
31. **R-USN4. Each pass costs O(volume records since the cursor) to classify, even for records outside the library, plus O(|state-1 directories| + |usn_stale| + |woken Foreign identities|).**
    - The journal is per volume, so a record's relevance is known only after it is read.
    - The walk retries every unreadable folder, so state-1 directories are retried too.
    - The crossover counts relevant records only.

## 7. Open questions (genuine user decisions; recommendation first)

**Decisions already taken (recorded):**

- PDQ radius stays exactly 31.
- One BASELINE edit, then freeze (D1).
- Batched commits are accepted (Q6), including the batched folder-wide missing updates, pairing pages and removal.
- Exact global grid layout, provisional briefly while a run builds (7.4/7.6).
- Process rules: implement everything, build once at the end, commit on the checked-out branch, and Phase 0 is the user's.
- Recommended options taken in round 1: Q3 ignore missing; Q4 scratch on the catalog volume with typed out-of-space errors; Q5 budgets as stated; Q10 show indeterminate progress.
- **Q-D:** the stated deviation set below is accepted as a whole, including the round-3 additions.
- **Q-T4 (a), cache-full memo:** skip the decode when the cache state and the next batch's (id, seq) match a stored all-success rejection (4.6).
- **Q-G1 (a), range-filter indexes:** none by default, opt-in per column in gallery.json, built as online Shadow access paths (7.17).
- **Q-S1 (b), NTFS change journal:** the USN-journal incremental rescan is included as step 3.9, with D37/D38, the full-walk fallback, and deviations J-a … J-f.

**Stated deviations to accept as a set (Q-D). Accepted by the user.**

- D6 cursor subset: no failure row for a file deleted mid-import; a file modified mid-pass waits for the next pass.
- **Pairing:**
  - Q3: extra raw pairs with missing files;
  - a pending revert re-indexes the file;
  - concurrent imports in two processes no longer miss each other's pairs;
  - legacy catalogs are re-paired;
  - as-of PDQ counts while heavy changes are unsettled (`gallery status --wait` for exact; `gallery dups` waits).
- **Scan:**
  - partial recording on a relist failure or a stop inside a huge folder;
  - out of space is typed;
  - **(J-a)** A hard-linked or held-open media file keeps the values last recorded when its directory entry in the library either:
    - (i) was already stale at its root's last full walk: changed through a link no walk of the root lists, or written while open, before that walk; or
    - (ii) belongs to a file that denies FILE_READ_ATTRIBUTES to OpenFileById.

    This lasts until the entry is refreshed without a journal record and its folder is next relisted (which then also queues a verification walk), or until the next full walk. Nothing else differs.
  - **(J-b)** Writes that bypass the NTFS change journal are seen only when their folder is next relisted (a full comparison; an unexplained difference queues a verification walk) or at the next full walk (Q-S5, 30 days by default). Examples:
    - another OS's NTFS driver (Linux ntfs3 or ntfs-3g, macOS drivers) on any disk, including internal fixed dual-boot disks;
    - offline edits; chkdsk /f repairs and NTFS self-healing;
    - an image restore that keeps the journal ID and position.

    Detachable buses always walk unless the volume is opted in. These are USB, 1394, SD, MMC, virtual, and hot-pluggable or removable media, including Thunderbolt/USB4 NVMe and eSATA.
  - **(J-c)** An entry modified while a journal pass runs, or between the read side and a resumed replay (up to the 24 h resume bound), is recorded with the values read. The next pass corrects it, because its journal range contains the change. The walk has the same window per folder.
  - **(J-d)** A folder with no journal record since the cursor, which would now fail to open or list for a non-access reason, is treated as readable as last listed. The reasons are: sharing violation, ERROR_FILE_CORRUPT or disk corruption, device not ready, resource exhaustion, or a relist failure. Its failure, error count, clean flag and missing-exclusion appear when the folder is next dirty, at a re-probe, or at the next full walk. Folders already unreadable are retried every pass, as the walk does.
  - **(J-e)** An ACE that names a logon-session SID (S-1-5-5-X-Y) on a library folder is re-evaluated only when the folder or its parent has a journal record, or at the next full walk. The per-session SID is left out of the access projection so that a new logon does not force a re-probe.
  - **(J-f, kept only if the USN8 real-NTFS test shows NTFS writes no record at a POSIX unlink before the last close)** A file POSIX-deleted while another process holds it open stays present until that handle closes.
- **Rebuild:** (a)-(j) (2.2).
- **Upgrade:** ordering of ungated passes; the status line shows progress; a zone restart (non-Windows only).
- **Removal:**
  - internal stamp order;
  - **mask (3.4):** while a removal is pending, the app's analyzed, failed, pending and pdq_pairs counts are shown as of its start with a note. Present and missing are exact. The Duplicates filter still counts twins under the removed root until they are marked. The CLI is exact (it drives removals first).
- **Thumbnails:**
  - the app yields to `gallery thumbs`;
  - retry per lap;
  - ≤ E + BATCH duplicate decodes per steal;
  - `TotalsFixed` after open;
  - a dropped PutOne under a stall;
  - cache-full memo (Q-T4 a): a source that changed on disk without a catalog or cache change is not re-tried until the next such change.
- **Borders CLI (4.8 a-d):** a tree that changes between the two walks; the `.partial` file while the run is in progress; serde's error text for non-UTF-8 input; review marks committed in batches.
- **Duplicates views** reflect heavy flips at settle-publish.
- **GUI edits** are optimistic and revert on failure.
- **Write-back:** offline parking cadence; PROBE_WAIT; recent_errors shows the latest errors.
- **`file_by_path`** returns the lowest matching id on ties.

**Questions** (the recommended option applies unless the user says otherwise):

1. **Q-B1. Borders CSV.**
   - (a) Write `<csv>.partial` beside the CSV, then copy it in place. The old CSV is untouched on error before the copy and keeps its identity; disk peaks at old + new while the run is in progress (recommended).
   - (b) Truncate in place from the first row. 1x disk, but an error midway loses the old CSV.
2. **Q-B2.** Print indeterminate stderr progress during the borders pre-walk (recommended: yes).
3. **Q-R1. Rebuild indexes.**
   - (a) Regenerate every index from the rows, as today; this heals a corrupt index (recommended).
   - (b) Copy unchanged tables' indexes through xfer: about 2x faster, but carries corruption over.
4. **Q-W1.** WAL_CEILING 512 MiB (recommended), or larger.
5. **Q-W2.** PROBE_WAIT 5 s (recommended), or longer.
6. **Q-T1. Thumbnail lease.**
   - (a) The CLI steals and the app retries (recommended).
   - (b) The CLI waits.
   - (c) The CLI refuses.
7. **Q-T2. Failure retry cadence.**
   - (a) Once per lap (recommended).
   - (b) Every round, which brings back starvation at 1B.
8. **Q-T3. State budget.**
   - (a) max(2^20, N/16): ≤ 5.6 GB worst at 1B (recommended).
   - (b) N/32: 2.8 GB, at about twice the compaction re-walk cost.
9. **Q-T5. Existing thumbnail caches.**
   - (a) Start over once at this layout edit, as the cache's documented policy says (minutes at 189K files) (recommended).
   - (b) A one-time in-place upgrade.

   In both cases the cache layout freezes with BASELINE.
10. **Q-T6.** Indeterminate thumbnail progress while a walk, range or revalidation is open (recommended: show it).
11. **Q-P1.** HEAVY_ENTER = 512, the break-even point (recommended).
12. **Q-P2.** Expose `tier1.max_pair_lag` (default L_lag = clamp(5 × t_sweep, 30 s, 60 min)) in gallery.json (recommended: yes).
13. **Q-P3.** Legacy catalogs re-pair under the new definition (recommended) instead of carrying history-dependent rows. Keeping them would break the 512-row bound.
14. **Q-S2. `unpaired_with_data` count.**
    - (a) Keep it O(that root's unpaired rows), read through the two (unpaired_anchor, order-key) ranges (recommended).
    - (b) Maintain a per-root counter with triggers on every child table plus re-attribution on root changes.
15. **Q-S3. Detachable volumes.**
    - (a) Volumes whose bus type, hotplug or removable flags mark them detachable always use the full walk, unless listed in gallery.json `journal.trust_detachable` (recommended).
    - (b) Trust the journal on them by default (J-b becomes common: such disks are often written by drivers that keep no journal).
16. **Q-S4. Cost crossover.**
    - (a) When the estimated pass cost (relist, moved-subtree and state-1 entries, probes, opens) exceeds 0.8 × the walk's estimate, use the walk. Results are identical; only cost differs (recommended).
    - (b) Always use the journal when the ladder passes.
17. **Q-S5. Verification walks.**
    - (a) A full walk per root every `journal.verify_days` (default 30, 0 = never), also queued at once by an unexplained difference in a relisted folder. This bounds J-a, J-b and J-d (recommended).
    - (b) Never; journal passes only.
18. **Q-S6. Journal sizing: decided.** The app offers a resize with consent through the per-session helper, sized from the measured record rate (3.9 USN6). It never resizes silently. `gallery journal size` stays as the manual path. No volume service.
19. **Q-S7. Journal watch in the app.**
    - (a) The app watches each capable volume and queues a Scans request 5 s after a relevant journal record (relevance-filtered, never writes, releases the volume on lock or removal) (recommended).
    - (b) No watch; Rescan stays manual, and journal passes run only when a scan is requested.
20. **Q-G2.** Front-coded order keys for HDDs: not adopted (recommended). SQLite index pages cannot front-code, so it would need a custom index.
21. **Q-G3.** Live mode ("N+", scrollbar off): not needed, since ProvisionalGeometry covers it (recommended: no).
22. **Q-G4.** Persist the current grid run across restarts (recommended: yes, as 7.5 has it).
23. **Q-DP1.** `--distance N≠22` stays exact through a one-shot build (hours at 1B, under the adhoc lease) (recommended).
24. **Q-M1.** Expose the memory budgets (role caches, W, Budget shares) in gallery.json (recommended: yes).
25. **Q-SC1.** Where the weekly 10M and occasional 100M scale runs go: a self-hosted runner (recommended), or D:. D: has 802 GB free; a 1B library does not fit there.

## 8. Verification

**Harness:** `crates/scale/tests/scale.rs`, `#[ignore]`, `--test-threads=1`.

- Catalogs at N and 10N come from a child `pg-scale synth`, cached by schema hash: smoke 100k/1M, full 1M/10M, manual 10M/100M.
- Two configurations:
  - **Uniform:** `CacheBudget::uniform(4 MiB)`, hard_limit = Σ open connections × 4 MiB + 16 MiB margin;
  - **Defaults:** real role caches, W, Budget.
- A counting global allocator; `pg-catalog/instrumented` (steps, authorizer, read-transaction spans).
- Samplers every 50 ms: temp dir, `<catalog>.scratch/*` per owner (journal-pass runs included), `<catalog>.tier1/`, runs, WAL, plus the `wal_hook` frame high-water and WAL bytes per row.
- Journal rows run on FakeVolume over synthesized catalogs; real-NTFS and VHD suites are in `just test` and `just usn-vhd`.

**Assertion classes** (between N and 10N):

| Class | Meaning |
|---|---|
| RAM | Rust peak Δ ≤ max(4 MiB, 5%); SQLite highwater flat and under the limit; PeakPagefileUsage flat |
| STEPS-N | VM steps ratio ≤ 15 (O(n) passes) |
| STEPS-Δ | O(changes) |
| STEPS-1 | flat (O(1), O(show), O(window), O(page)) |
| DISK | the named sampler ≤ the stated bound |
| WAL | per-commit frames ≤ F + 64·P; live WAL ≤ ceiling + batch |
| RTX | max read transaction ≤ 2 s |
| ABS | Defaults configuration: PeakPagefileUsage ≤ the §2 derived sum |

**Matrix: every §2 row has a scenario and an assertion.**

| §2 row | Scenario(s) | Assertions |
|---|---|---|
| Scan walk | First scan of: one N-file folder, N subfolders, a balanced tree. No-change rescan walk. A 10N-subfolder single folder. Validate and relist injection. A stop inside a listing. A first walk with mirror upkeep. | RAM; STEPS-N; WAL; DISK (entry scratch 0; frames ≤ 20 B × N + one block; `usn_dirs` ≤ 280 B/dir); rescan WAL ≤ about 14 B per row |
| Journal rescan | No-change pass at N/10N; 10^4 and 10^6 records (FakeVolume) over N/10N catalogs; a renamed folder of N/100 and of 0.5·N files; an R-A re-probe; a multi-root shared read; stop in R and W, then resume; Budget{1,1} | RAM (counting allocator ≤ the 3.9 bound); STEPS-1 for a no-change pass, as long as \|state-1\|, \|usn_stale\|, \|usn_wake\| and the root's unpaired rows are held fixed between N and 10N (8.1 generator); otherwise STEPS ∝ those sets. A pass with zero relevant records and empty recurring sets stops before R6: no scans row, no W5/W6; STEPS-Δ (∝ records + dirty entries + sub-walk entries + affected rows); WAL; RTX; DISK (scratch ≤ 60 B/record + 150 B per non-subtree intent, 0 per sub-walk entry); half-library rename ≤ 0.6 × the walk's time; oracle-equal to `--walk` |
| Pairing | Two roots; two drives with mirrored layouts; a drive root; save-overs; rebinds; ambiguous; moved; deleted; a stop between pages, then resume; a journal-pass floor | RAM; STEPS (rows visited ≤ that root's (anchor, range) unpaired rows); 0 write transactions when nothing pairs; resume equals uninterrupted |
| mark_missing / removal / `import --again` | Remove an N-file single-folder root (with a mirrored same-named root on a second drive); delete N in one folder, then rescan; `--again` over N; warm and cold timing; a scan request during a removal | RAM; WAL; STEPS-1 per page; the mask hides at once and leaves the other drive visible; status present/missing exact; a requested scan starts within one 60 s slice; time per row recorded (R8) |
| Import work list (+retry) | Import with a stub analyzer; `--retry`; preemption at every fetch | RAM; examined ≤ n + preemptions × 4096; done == total |
| Version bump | Bump import, borders and zone at N and 10N, preempted at every batch; a reader during Pass A | prepare STEPS-1; Pass A no index writes (fan-out); RTX; exact reads equal the oracle; `pend_batch` equals its definition |
| Status / writer counts | `gallery status`, `gallery write` poll, writer drain of 10^5 | STEPS-1 |
| Write-back queue | Q failed rows; 10^5 parked on an absent root | `next_ready` STEPS-1; ≤ 1 probe per 30 s |
| PDQ | (1) Incremental: a fixed 10k new images against N/10N indexes. (2) Maintenance and the debt cap after a long import. (3) Bootstrap and legacy re-pair at smoke size. (4) Lone image (Q=1). (5) Planted clusters of 10k and 100k. (6) Re-import peak. (7) Flips of a whole cluster. (8) Re-store after an old onset, then a later session. Explicit B_batch and T keep R=31 feasible ([PRX7]). | RAM ≤ W + 8.4 MB (dhat Rust heap ≤ W + 2 MiB); DISK (temp O(k) in k; tier1 dir; re-import ≤ 2 × steady + 146 GB); run count ≤ cap; sweep reads each sheet byte once; odd fraction reported; (8) pairs equal today's index-load oracle |
| Borders list | Borders pass after a detector bump | RAM; STEPS-N; no index writes in Pass A |
| `borders --csv` | CLI under `run_in_job` over a SynthTree; one directory of 10N; a 2,000-level chain; a slow head; an injected copy failure | RSS flat; DISK (directory spill only above budget; partial ≤ CSV size; reorder spill freed); the partial kept on a copy failure |
| `--import-review` | 10N entries | RSS flat; DISK runs about 100 B per entry; probes STEPS-1 each |
| Thumbnail pass | Evict 1K at a walked height; a preempted first pass; 1% persistent failures with a stop every PAGE; an offline volume holding 20%; cache-full × 20 (memo); mass seq bump; mass deletes past the prune; two-height alternation; restore and advance | STEPS-Δ / STEPS-1 per round; w monotone; DISK (state ≤ budget + lost bound); RAM |
| Thumbnail cache upkeep | Heal due at E and 10E; evict at the cap with keep dominating | open STEPS-1; RTX; evict steps per victim flat |
| Dups | Build and replay; report; storm (overflow switch); Duplicates filter over a k-member mostly-missing run | RAM ≤ 300 MiB + W; DISK (dup_log ≤ CAP); report STEPS-1; filter flat in k |
| Grid | Name and sorted builds; deltas of 1k and 3M; window scroll at 2^31+2^20+7 (periodic); Duplicates-filtered refresh; top-K preview; removal mask on mirrored drives | RAM (client ≤ 11 MiB + window bound; build ≤ 330 MiB); STEPS-Δ for refresh; DISK runs ≤ 96 GB/1B scaled; RTX (converging delta) |
| Name filter | Needles of 1, 2 and 5 characters, rare and common | STEPS ∝ candidates; scan fallback at scope/8 |
| Range-filter indexes | Opt in size and taken at N/10N; build under concurrent writes; a taken step; a selective and an unselective range; opt-out | Results equal the scan; STEPS ∝ candidates below scope/8; DISK ≤ 30 B per row per column; `mismatch()` None; `set_rating` ≤ 150 ms during the build; RTX |
| Folder tree | Build plus slot scroll at 3e9 rows (Uniform); a pending removal on one of two merged drives | RAM ≤ 4 MiB + slots; merged counts exact |
| Facets | With concurrent writes; a pending removal of `E:\Photos` with a mirrored `D:\Photos` (facet counts equal the oracle with only E: masked) | RAM ≤ 70 MiB; DISK log |
| Crops / changes_since | Crop list of N | RAM ≤ 2 MiB |
| Folder picker | 3×10^6 synthetic names | RAM ≤ 20 MiB; order equals the oracle |
| Qt/QML edge | `huge-list`, `huge-keys` assert dumps; window laws | Exact strings; slots ≤ S |
| Catalog rebuild | Forced rebuild at N/10N; kill at random points | RAM ≤ 2 × cache + 16 MiB + index/32768; DISK temp ≤ 2.2 × the largest index; no WAL; resume identical |
| Post-freeze migration | Synthetic RebuildTable(files) and Shadow | DISK ≤ start + batch + delta; set_rating ≤ 150 ms during Shadow |
| Startup | Normal open; open needing a rebuild | open STEPS-1; window after 300 ms; cancel ≤ 100 ms |
| SQLite connections / temp / WAL | Every scenario | WAL; RTX; temp per owner; role caches sum to 884 MiB |
| Whole process | All passes in one process, Defaults configuration, including a journal pass and the watch | ABS; RAM |

- **CLI variant:** a child under `run_in_job(limit)` for every subcommand, including `tier1 *`, `dups --distance`, `borders --csv`, `--import-review`, `thumbs`, `remove`, `status --wait`, `scan`, `scan --walk` and `journal status`.
- **Self-checks** (feature `self-check`):
  - an injected O(n) Vec fails Rust flatness;
  - `group_concat` over 10N trips the hard limit;
  - a single-transaction 10N update violates WAL;
  - a 5 s reader violates RTX;
  - an injected per-sub-walk-entry intent fails the journal DISK bound.

**Commands** (once, at the end):

```text
just build && just lint && just test   # unit, oracle, proptest, plan guards, qt-edge lint, golden ListView, journal oracle and faults
just lint-qml                          # qmlcachegen: no fallbacks
just plans                             # ledger: tags, plans, temps, fan-out, total order, read-tx census, allowlist, unpaired_anchor writers
just shots                             # includes $Asserts (huge-list, huge-keys), failing on mismatch
just scale-smoke                       # 100k/1M, Uniform + Defaults (also in Windows CI)
just scale                             # 1M/10M weekly
just usn-vhd                           # elevated: journal delete/recreate/wrap/rollback/clone, second-user and elevated mutators, volume lock
just perf --update
just scale-dhat <pass> <size>
```

**Oracles kept in-tree:**

| Area | Oracle |
|---|---|
| Pairing | `reference_pair` / `reference_mark_missing` / reference walk (floor = scan); `folder_id IN under_root!` for the root scope |
| Journal rescan | `gallery scan --walk` at the same checkpoints (A/B catalogs; all-J, mixed, resumed and abandoned schedules), on FakeVolume (with parent ambiguity) and real NTFS |
| Work lists | old pending SQL constants |
| Upgrade | `set_import_version` / `set_borders_version` / `set_taken_zone` bodies |
| Status | `old_status` |
| Write-back | old `NEXT_READY` |
| Rebuild | `cfg(test)` `copy_tables` |
| Thumbnails | `OLD_FULL_SQL` / old evict ORDER BY |
| PDQ | brute force + old `Pairer` (two comparisons) + the old `pdq_hashes` session load |
| Dups | `oracle_groups` / `old_*_counts` / old EXISTS forms |
| Grid | `list_grid_reference` |
| Range-filter indexes | the unindexed scan of the same query |
| Tree | `reference_tree` / FOLDER_COUNTS |
| Facets | per-facet statements |
| Borders CSV and review | old `image_files` / `report` / `import_review` |
| ListView | golden table reproduced by a real ListView |

## Round-2 changelog

- **Residuals and deferrals closed:**
  - thumbnail eviction resets (TE replaces Listed/Walk);
  - single-transaction carry (2.2);
  - per-folder removal and missing UPDATEs (F1, F5);
  - borders report O(inputs) (4.8);
  - huge-folder 60-80 GB spill (now 0 for entries);
  - lease wall clocks (counter heartbeat);
  - grid delta WAL pin (converging reads);
  - writer stamp discipline (trigger stamps);
  - Qt 2^31 cap (QT1-QT9);
  - set_counts in prepare (removed).
- **Rejected or changed from round 1:**
  - The PDQ design was replaced wholesale (G1-G12): about 370 TB instead of 9 PB, exact R=31, heavy core, O(k) temp.
  - "R=22 gate" text removed.
  - D6 "exact" restated as a deviation.
  - D9 index re-keyed by order_key (root range). Superseded in round 3 by (unpaired_anchor, order_key) plus the path test (RR2).
  - D17 reordered with fairness.
  - D20 replaced by the Upgrade pass.
- **New decisions D22-D36:** counters row; rebuild protocol; named uniqueness; session-clock pair definition; PDQ storage; pure-SQL Duplicates filter with stamps; ordered sinks; Qt numeric edge; WindowView; read-snapshot rule; gates in the API; GUI edit worker; removal mask; thumbnail state; startup.
- **Audits added** (write/start, shell/CLI):
  - status and writer counters;
  - write-back probes, parking and streaming I/O;
  - startup off the GUI thread;
  - thumbcache heal, evict, touches and inbox;
  - `file_by_path` probe;
  - name trigram index;
  - folder picker windowing;
  - edit worker plus overlay;
  - pools by role;
  - held list as a pinned run.
- **Synthesizer refinements:**
  - The pairing clock is the import session, stamped at store, with `pend_batch` on files. This removed G's batch-start deviation, U1's tier-1 gate and backlog drain, and G8's pending skip.
  - Thumbnail batches use ordered sinks, so a cache-full memo is possible (Q-T4).
  - The status pairs counter is Σ bands.
  - Per-family recount cursors are kept for post-freeze migrations.
- **Migration machinery:** RebuildTable with a migrating flag (F6), online Shadow access paths (U12), and a guard that refuses CREATE INDEX on large tables.
- **Phase order fixed:** toolkit before keyset; concrete scan statements moved to Phase 3; forward dependencies marked.
- **Housekeeping restored:** `files_log` pruning, `forget_scan_scratch`, begin_scan residue cleanup.
- **Scratch:** a reservation ledger, lease-named owners, stale-only sweep, and a Budget split into interactive and background shares.
- **§2 recomputed:**
  - new rows: rebuild, migrations, startup, status, write-back, review import, picker, name index, Qt, thumbnail upkeep;
  - whole-process peak re-derived: about 3.6 GiB defaults, about 2.1 GiB minimum (corrected from 1.5);
  - temp peaks restated.
- **§6 rewritten** to contain only proven residuals, each with its proof. Round 1's "follow-up" and "must be chunked" items are gone.
- **§7 rebuilt:** taken decisions recorded; deviations collected as Q-D; new user decisions (USN journal, cache-full memo, CSV mode, rebuild index policy, WAL ceiling, probe wait, thumbnail lease, budget, start-over, HEAVY_ENTER, lag, legacy re-pair, range-filter indexes).
- **§8 rebuilt as a matrix** mapping every §2 row to a scenario and an assertion class. Added: the Defaults-budget absolute scenario, per-owner disk samplers, the flat-steps class, the read-transaction census, the feasible PDQ split at R=31, and text-dump Qt asserts.

## Round-3 changelog

- **User decisions recorded (§1, §7):** Q-D accepted as a set; Q-T4 (a) memo; Q-G1 (a) opt-in range indexes (new 7.17); Q-S1 (b) USN journal (new 3.9).
- **Blocker RR1, `pend_batch` clearing:**
  - `files_pending_off` always sets NULL, and requeues only when a `pdq_indexed` row exists;
  - `formed` regains G's NULL guard (SQL and Rust);
  - one owner per trigger duty (D8 stamp vs `files_pending_on`/`off`);
  - `counts.bulk` suppresses only changed/seq;
  - Pass A no longer writes `pend_batch` inline;
  - `pdq_heavy_gone` and plane P read `files.pend_batch`;
  - `meta.pdq_batch` is bumped only after a passed gate and a non-empty list;
  - P0 does not write it;
  - legacy carry sets 0 for pending rows and NULL otherwise;
  - new tests: re-store, revert, NULL, and an empty import.
- **Blocker RR2, root scope:**
  - a root is (anchor, key range) plus the exact `under_root!` path test (D9);
  - new `anchors` table, `folders.anchor` and `files.unpaired_anchor`;
  - re-keyed `files_unpaired_fresh/kept`;
  - every unpaired/folder_id writer keeps the anchor (ledger check and proptest);
  - the D34 mask is path-exact in grid, run deltas (filtered delete), tree (merged-node subtraction), facets and status;
  - `file_by_path` iterates equal keys;
  - Q-S2, the PRF5 row, §2 and §8 rows corrected;
  - multi-drive scenarios added.
- **USN-journal rescan (3.9, D37, D38):**
  - **Platform calls (1.6):** journal, identity, probe, token, bus, self-test and VolumeGuard.
  - **BASELINE objects (2.1):** `usn_*` tables, wake triggers, `scans.kind/floor/usn_state` and the sentinel scan 0.
  - **Walk (3.1):** `visit_folder`/FolderTally, sinks, walk_key, the collation check, mirror upkeep and a crash-safe base cursor.
  - **Catalog side (3.2-3.8):** floor-relative pairing and mark_missing, the ordered drop/scope merge, the UsnBad skip source, usn removal, scratch cleanup and the allowlist.
  - **The pass itself:** ladder, R-A, R1-R6, W1-W7, resume and abort-to-walk.
  - **Around it:** runner, CLI, app and watch; FakeVolume; the oracle, faults and VHD suite.
  - **Bounds and decisions:** a §2 row (≤ 140 MiB); R-SCAN narrowed to the fallback cases; R-USN1..R-USN4; Q-D J-a..J-f; new questions Q-S3..Q-S7; review UR1-UR21 recorded (§5).
- **Major and minor RR fixes:**
  - §2 whole-process sum re-derived with explicit concurrent terms, and W stated as one process-wide reservation;
  - bulk build in the background share;
  - §8 PDQ RAM aligned;
  - D17 puts Removals after Scans in ≤ 60 s slices and adds the Names idle pass;
  - removal status exact for present and missing, as-of for the rest (Q-D);
  - Q-D gains the memo, borders CLI and removal-mask deviations;
  - the CSV partial is kept on a copy failure;
  - tier1 removed from `with_upgrade`;
  - Names and Watch roles (884 MiB total);
  - the rebuild row's claim narrowed.
- **Rejected:** RE25's 3.9 text, in favour of the reviewed USN design. Lazily created `folders` rows cannot carry directory identity (`scan/src/lib.rs:300-301`).
- **Modified:**
  - RE6/RE8: `folders.anchor` plus the path test, because `folders.path` compares bytewise;
  - RE10: field names;
  - RE14: stray line dropped;
  - RE20: plus the Watch role.

### Critical Files for Implementation

- D:/GitHub/photo-gallery/crates/catalog/src/schema.rs
- D:/GitHub/photo-gallery/crates/catalog/src/lib.rs
- D:/GitHub/photo-gallery/crates/catalog/src/pair.rs
- D:/GitHub/photo-gallery/crates/scan/src/journal.rs (new)
- D:/GitHub/photo-gallery/crates/thumbcache/src/lib.rs
- D:/GitHub/photo-gallery/crates/platform/src/usn.rs (new)
- D:/GitHub/photo-gallery/crates/catalog/src/usn.rs (new)
- D:/GitHub/photo-gallery/crates/library/src/runner.rs
