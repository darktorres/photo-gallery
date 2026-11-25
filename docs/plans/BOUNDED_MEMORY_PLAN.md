# Bounded-memory catalog at 1B files: implementation plan

> **Note (plan phase A2, `docs/plans/HEADLESS_SESSION_PLAN.md`).** The `gallery` command line is removed. Its `borders --csv`,
> `borders --import-review` and `dups` commands are gone with it, so the work on them below (table rows 73 and 74, section 4.8,
> the `dups` waits, and the test rows for them) is moot, along with the 450-850 GB worst case recorded for `borders --csv`. The
> `gallery <pass>` names elsewhere here are `pg-dev run --passes <name>` now.

This plan makes every library-wide pass in the catalog run in a fixed RAM budget that does not depend on library size. Results stay identical (pairs, groups, grid order, filters, counts, CLI output) except where a deviation is stated. It covers the schema, scanning and pairing, work lists, write-back, thumbnails, borders, PDQ near-duplicate detection, duplicate groups, the grid and the Qt edge, and the verification harness.

Phase 0 is the user's. The in-flight S-17 Unit E work lands before Phase 1 starts.

## 1. Context, rules and decisions

### 1.1 Problem

A code-only audit found that almost every library-wide pass holds its whole working set in RAM. At 1B files that set is about 0.9-1.2 TB. The PDQ index alone is 864-928 GB, and the work lists, grid lists, scan sets and duplicate maps each add roughly 100 GB. SQLite also runs with `temp_store=MEMORY` and 14 caches of 256 MiB.

### 1.2 Rules

- RAM is a fixed configured budget, independent of library size.
- Results are identical: pairs, groups, grid order, filters, counts and CLI output. Where they are not, the deviation is stated in the step and collected in §7 (Q-D).
- Disk and time are stated at 1B.
- Schema changes go through the migration system: one BASELINE edit now, then a freeze (D1).
- Implementation is completed in full, then built once at the end with `just build && just lint && just test`, and fixed forward. Commits go on the checked-out branch. No new branches.
- Paths are relative to `D:/GitHub/photo-gallery/`. The user is editing the tree, so cited line numbers may drift. Find code by symbol.

### 1.3 Decisions taken by the user (2026-11-07, unless noted)

- PDQ radius stays exactly 31.
- One BASELINE schema edit, then freeze.
- Batched commits are accepted.
- Exact global grid layout, provisional briefly while a run builds.
- The Q-D deviation set (§7) is accepted as a whole.
- Thumbnail cache-full memo: Q-T4 (a).
- Range-filter indexes are opt-in per column: Q-G1 (a), step 7.17.
- The NTFS USN-journal incremental rescan is included: Q-S1 (b), step 3.9.
- **Value truth across scan sources.** Every rung treats directory-index values (size, mtime and creation time, as a directory listing returns them) as the truth. That covers the walk, step 3.9, the per-session MFT helper, and any service.
  - MFT rungs read `$INDEX_ROOT` and `$INDEX_ALLOCATION` raw (an LCN-sorted sweep with fixups), not `$STANDARD_INFORMATION` and `$DATA`.
  - MFT rungs filter folders by whether the client's token has FILE_LIST_DIRECTORY access, so their output equals the walk's.
  - Step 3.9's `usn_stale` and deviation J-a stay.
  - `docs/plans/NTFS_MFT_SCAN_PLAN.md` must be amended to match. Its helper currently takes MFT-record values and sends every folder. It also gains the `ResizeJournal` request (3.9).
- **No volume service.** The ladder is: the per-session helper (MFT read where elevation is silent), then 3.9, then the walk. The service's main gain, following the journal while the app is closed, is mostly covered by journal sizing with consent (Q-S6, step 3.9, runner and CLI). The rest of its gain is for non-admin and multi-user installs, which do not exist yet. Its costs are an installer (none exists), a SYSTEM attack surface, pipe-server authentication, emulated per-folder AccessCheck, and a durable dirty-reference store. Revisit this if the app ever ships with an installer to non-admin users.

Every other open question takes the recommended option unless the user says otherwise (§7).

### 1.4 Phase 0 and history

The in-flight S-17 Unit E work in the working tree is not part of this plan, and it is not committed on the user's behalf. The unrelated untracked picker and yandex edits are the user's.

This version drops the review rounds that produced it: their outcome tables, changelogs and issue ledgers. The previous version, with those rounds, is in git history (`git log -- docs/temp/BOUNDED_MEMORY_PLAN.md`). Where a rejected alternative still explains a current decision, the reason is kept in §5.

### 1.5 How to read the plan

- **D1-D38** are cross-area decisions (§3).
- **§5** lists alternatives that were considered and rejected, with the reason.
- **R1-R-USN4** are residuals: costs proven unavoidable (§6).
- **Q-…** are open questions, with the recommended option first (§7). **Q-D** is the set of accepted deviations.
- **J-a to J-f** are journal-pass deviations, and (a)-(j) are rebuild deviations (§7 Q-D).
- **Numbered steps** (for example 2.4 or 3.9) are the implementation units (§4).
- **Source paths** are paths in this repository, not the review transcripts.

## 2. Target bounds at 1B

All values are for 1B files unless noted. "NVMe" means a warm or cold NVMe catalog volume.

| Stage                                      | RAM today                                                                     | RAM after                                                                                                                                                                                                                            | Disk after                                                                                                                                                                                                                                                                                                                    | Time after                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
|--------------------------------------------|-------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Scan walk                                  | 25-38 GB seen set, plus 115-145 GB per giant folder                           | About 3 MB Rust plus the Writer cache                                                                                                                                                                                                | `file_seen` +14 GB (files shrinks about 10 GB). Entries need no scratch. Frames spill only above 16K pending subfolders, at 8-20 B per frame. On NTFS roots the mirror `usn_dirs` takes about 280 B per directory (2.8-14 GB at 10-50M directories) and is written on change only. The first walk adds about 3% more inserts. | A no-change rescan walk writes about 14 GB of WAL (was 200-400 GB). Reads: one identity probe per listed entry, 200-400 GB, about 1-4 h on NVMe (R-SCAN). A folder with more than 16K entries is listed twice, about 5-15 min per pass of 1B entries. **Full walks now run only for:** a first scan (including the first after a rebuild), a 3.9 fallback, a Q-S5 verification walk, a requested `scan --walk` or Full rescan, Linux/WSL roots, network shares, non-NTFS volumes and detachable volumes (§6 R-SCAN). |
| Journal rescan (NTFS, 3.9)                 | A full walk (R-SCAN)                                                          | About 140 MiB from the Budget background share, constant in n: 1 MiB read buffer, at most 40 MiB record groups, at most 24 MiB dirty set, at most 24 MiB intent runs, 16 MiB mirror cache, and the 3.1 bounds for in-place sub-walks | `usn_dirs` as above. `usn_entries` about 60 B per exception. Scratch: about 60 B per journal record, plus 150 B per dirty pair or non-subtree intent, plus 40 B per affected row or scope. Nothing per sub-walk entry. Deleted at W3 and at finish.                                                                           | O(volume records + dirty-folder entries + sub-walk entries + affected rows + state-1 + stale + woken). No change: at most 0.2 s. 10K changes: 1-3 s. A 1M-record day: about 45-100 s. A 10M-file import: about 20-35 min. A renamed folder costs the walk's per-entry cost for its subtree. An access re-probe takes about 5-25 min at 10-50M directories. A 4 GiB journal holds about 30 days at 1M records/day.                                                                                                    |
| Pairing                                    | 370-410 GB + 8 + 10-20 GB                                                     | About 0.5 MB, plus the root's fresh-id spill (16 B per row)                                                                                                                                                                          | `files_unpaired_*` is transient                                                                                                                                                                                                                                                                                               | First scan: 4 sequential passes, 0.8-1.6 TB of reads, plus about 20-25B B-tree probes (about 6-12 CPU-hours). A rescan with nothing new reads O(that root's unpaired rows) through the (anchor, key range) (D9), and does 0 write transactions. A stop takes effect between pages.                                                                                                                                                                                                                                   |
| mark_missing / removal / `import --again`  | 8-8.6 GB + O(root) FIFO + journal                                             | About 0.4 MB                                                                                                                                                                                                                         | WAL at most about 26 MiB per commit                                                                                                                                                                                                                                                                                           | Removal takes 3-80 h on NVMe, in the background in slices of at most 60 s, and is resumable in O(depth). It is hidden from readers immediately, path-exact (D34). Cold HDD is far slower (R8).                                                                                                                                                                                                                                                                                                                       |
| Import work list (+retry)                  | 94-124 GB (+90 GB retry sort)                                                 | At most 256 KiB page + 8 MiB FolderCache                                                                                                                                                                                             | Retry list: 80 B per failure, TEMP, paged                                                                                                                                                                                                                                                                                     | About 1M fetches at about 1 ms each.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Version bump (import, borders, taken zone) | One transaction: 200-300 GB WAL, -shm 0.5 GB, no window for hours             | O(1)                                                                                                                                                                                                                                 | No sidecar, no sort                                                                                                                                                                                                                                                                                                           | `prepare` takes under 5 ms. Pass A runs 1-2 h sequentially in the background, and reads stay exact throughout (D20).                                                                                                                                                                                                                                                                                                                                                                                                 |
| Status, writer counts, pass totals         | count(*) taking O(n+P): 30-90 min, or 6-28 h for the pairs term               | O(1)                                                                                                                                                                                                                                 | One `counts` row                                                                                                                                                                                                                                                                                                              | About 1 ms per call. A writer drain is O(Q).                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Write-back queue                           | O(Q²) per drain, offline serialization                                        | O(1)                                                                                                                                                                                                                                 | A few B per job                                                                                                                                                                                                                                                                                                               | `next_ready` is O(10 log Q). Offline: at most 16 root probes plus 16 folder probes per 30 s, off the drain thread.                                                                                                                                                                                                                                                                                                                                                                                                   |
| PDQ tier 1 and pairing                     | 864-928 GB (+320 GB reserve)                                                  | At most W + 8.4 MB in the W holder (dhat Rust heap at most W + 2 MiB). W = clamp(RAM/32, 0.5, 4 GiB), so about 2 GiB at 64 GB of RAM. W is one process-wide reservation.                                                             | 2.14 KB per light file, about 18 KB per heavy file, rows at most 512 per light file. That is 2.1 TB with no heavy files (150 GB at 70M), plus a 146 GB merge transient. Re-import peak about 4.4 TB.                                                                                                                          | First import: about 26 core-days of CPU, plus about 360 ns per heavy-heavy near-copy pair, plus about 370 TB of sequential I/O (+30 TB at 1% heavy). Pair lag at most about 70 min (1B fresh), at most about 5 min (70M). One image: 2-6 s on NVMe, about 21 s on SATA. On HDD, a sweep of the index is used instead of random reads.                                                                                                                                                                                |
| Borders list                               | 94 GB                                                                         | About 1 MB + FolderCache                                                                                                                                                                                                             | `files_borders_due` holds only flag-1 rows. A detector bump writes flag 2 with no index.                                                                                                                                                                                                                                      | Pass A 1-2 h, then the walk stream.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `gallery borders --csv`                    | 450-850 GB                                                                    | About 0.3 GB + the pipeline                                                                                                                                                                                                          | Scratch only for a directory above the sort budget. `<csv>.partial` up to the new CSV's size (about 150 GB) while the run is in progress (R7).                                                                                                                                                                                | Two listings (15-60 min each, warm) against about 1B decodes.                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `gallery borders --import-review`          | 300-450 GB, plus O(entries × folder) lookups                                  | ExtSort budget                                                                                                                                                                                                                       | About 100 GB of runs for 1B entries                                                                                                                                                                                                                                                                                           | Sort 20-40 min, plus 1B O(log n) probes, plus batched commits.                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Thumbnail pass                             | 118-236 GB + 200 GB sort                                                      | About 208 MiB fixed caches + under 15 MiB Rust                                                                                                                                                                                       | State at most B members × about 90 B, about 5.6 GB worst (94 MB at 189K files). `lost` at most about 1.5% of the cap.                                                                                                                                                                                                         | Eviction costs O(evicted). A cache-full round costs O(new events) + 1 memo probe. A first pass per height reads N rows (14-28 h), resumable across any number of preemptions.                                                                                                                                                                                                                                                                                                                                        |
| Thumbnail cache upkeep                     | Heal O(entries) on the GUI thread; evict walks every entry                    | O(heights)                                                                                                                                                                                                                           | `height_totals` table, `entries_height_lru`                                                                                                                                                                                                                                                                                   | Heal: about 10-30 s at 130M entries, chunked, in the background. Evict: O(H log E + victims).                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Dups report and maintainer                 | 25-80 GB + 16 GB + 50 GB sorter                                               | Report O(show); maintainer at most about 300 MiB + W (heavy peel)                                                                                                                                                                    | Derived tables 47 GB worst; `hash_runs` 6-60 GB; `pdq_near` 2-20 GB; `dup_log` at most CAP (about 1 GB)                                                                                                                                                                                                                       | Build 1-6 h, resumable. The Duplicates filter costs O(log n) per row.                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Grid list / sorted list / refresh          | 56 / 108-128 / 112 GB                                                         | About 11 MiB client + window at most 9.6 MB worst; build at most 330 MiB transient (256 MiB interactive share + 74 MiB)                                                                                                              | Run 80 GB (name order) / 96 GB (sorted), at most about 300 GB worst transient                                                                                                                                                                                                                                                 | A cold build takes 20-30 min warm (hours cold) in the background, with a live top-K preview. A refresh, Duplicates-filtered views included, costs O(changes × log n).                                                                                                                                                                                                                                                                                                                                                |
| Name filter                                | O(scope) scan per edit                                                        | The build's bounds + the Names role (16 MiB)                                                                                                                                                                                         | `names.sqlite` 30-80 GB                                                                                                                                                                                                                                                                                                       | O(candidates + lag + sort) for every needle length.                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Range-filter indexes (opt-in, 7.17)        | O(scope) scan per edit                                                        | The build's bounds; Shadow build within the background share                                                                                                                                                                         | 20-30 GB per opted-in column                                                                                                                                                                                                                                                                                                  | Build 1-2 h online per column. A selective query costs O(candidates + sort). Opt-out is one DROP TABLE.                                                                                                                                                                                                                                                                                                                                                                                                              |
| Folder tree                                | 2.3 GB + 0.4 GB + 170 MB                                                      | Under 4 MiB, + about 175 KiB of slots                                                                                                                                                                                                | 1.5 GB of per-generation files at 10M folders                                                                                                                                                                                                                                                                                 | Build 10-30 s.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Facets                                     | 50-80 GB GROUP BY sorter                                                      | Under 70 MiB                                                                                                                                                                                                                         | 12 GB contribution log, transient                                                                                                                                                                                                                                                                                             | Chunked.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Crops / changes_since                      | 8 GB / 8 GB                                                                   | Under 2 MiB / 96 KiB per page                                                                                                                                                                                                        | Crop id list: 8 B per bordered file                                                                                                                                                                                                                                                                                           | Streamed.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| In-app folder picker                       | O(folder) on the GUI thread                                                   | At most 4 MiB + 16 MiB sort                                                                                                                                                                                                          | Only for folders whose names total more than 4 MiB                                                                                                                                                                                                                                                                            | Off the GUI thread.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Qt/QML edge                                | i32 caps; float32 misplacement past about 645K rows                           | Slots at most floor((viewH+3000)/min_pitch)+4                                                                                                                                                                                        | None                                                                                                                                                                                                                                                                                                                          | Per frame O(slots).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Catalog rebuild (mismatch or corruption)   | One transaction: WAL about the DB size, 3x disk, days of random index inserts | About 274 MiB + largest index / 32768 for SQLite, O(1) Rust                                                                                                                                                                          | Kept + building + sorter at most 2 × largest index (R1)                                                                                                                                                                                                                                                                       | 6-12 h on NVMe, resumable at chunk or index granularity, behind the startup window. Then one walk per NTFS root, because `usn_*` is not carried.                                                                                                                                                                                                                                                                                                                                                                     |
| Post-freeze migration                      | n/a                                                                           | As rebuild                                                                                                                                                                                                                           | RebuildTable: in place, start size + one batch + layout delta. Shadow: about the index size.                                                                                                                                                                                                                                  | RebuildTable: exclusive, 5-9 h for `files`. Shadow: online, 1-2 h.                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Startup                                    | `prepare` blocks before any window for hours                                  | O(1)                                                                                                                                                                                                                                 | None                                                                                                                                                                                                                                                                                                                          | A normal start is unchanged. A rebuild or exclusive migration shows a stoppable progress window.                                                                                                                                                                                                                                                                                                                                                                                                                     |
| SQLite connections, temp, WAL              | 14 × 256 MiB; temp unbounded in RAM                                           | Role caches about 0.86 GiB (D3); temp on disk                                                                                                                                                                                        | WAL at most 512 MiB ceiling + one batch (at most 26 MiB); -shm about 1 MiB. Temp peaks: Listed tier gone, retry list 80 B × F, CREATE INDEX sorter at most 2 × the index, ExtSort runs as per row.                                                                                                                            | No read transaction exceeds 2 s (D31). Crash recovery at most 0.5 s on NVMe.                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| **Whole process**                          | About 1-1.3 TB + unbounded temp                                               | **At most about 3.62 GiB at defaults (64 GB RAM); about 2.12 GiB minimum; constant in n**                                                                                                                                            | Per row                                                                                                                                                                                                                                                                                                                       | Per row                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |

**Whole-process derivation.** Passes run one at a time in the runner. Caches fill lazily.

- **W** is one process-wide reservation. It is held in turn by the pair worker, the groups heavy peel (6.2) and tier-1 merges or bulk build. The bulk build sorts in the background share only.
- **Defaults peak:** role caches 0.86 GiB (884 MiB, see 1.2) + the largest runner pass term + interactive sort share 0.25 GiB + grid build beyond its share 0.07 GiB + facets 0.07 GiB + client, window, tree and picker 0.05 GiB + journal watch 0.02 GiB (a 16 MiB LRU plus 64 KiB per volume; its 4 MiB Watch cache is counted in the role caches). The largest runner pass term is groups 0.3 + W 2 = 2.3 GiB (6.2 heavy adjacency), against import pipeline 0.25 + W 2 = 2.25 GiB, thumbs 0.25 GiB, or a journal scan pass 0.14 GiB inside the background share. The sum is 0.86 + 2.3 + 0.25 + 0.21 = 3.62 GiB.
- **Minimum peak:** W = 0.5 GiB. The largest pass term becomes groups 0.3 + 0.5 = 0.8 GiB, and the sum is 0.86 + 0.8 + 0.25 + 0.21 = 2.12 GiB.
- Phase 8 asserts that PeakPagefileUsage is at most this sum.

## 3. Cross-area decisions

| #   | Topic                         | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Why / evidence                                                                                                                                                                                                                                                                                                                                                                                                                                  |
|-----|-------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| D1  | Schema route                  | **One coordinated BASELINE edit (Phase 2), then freeze.** After the freeze, `MIGRATIONS: &[Migration]` with `Migration::{Sql(O(1) DDL), Steps(&[Step])}`. The steps are `Step::RebuildTable` (exclusive), `Step::Shadow` (online access path), `Step::Upgrade` (online recompute or recount) and `Step::Sql`. A guard test refuses `CREATE INDEX` on `LARGE_TABLES` in migrations.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | `schema.rs`: MIGRATIONS is empty, and the baseline is `baseline.sql` (the migration chain was baked into it in phase 2). Dropping `analysis.borders_due` alone would need only DROP INDEX plus DROP COLUMN, but `files.last_scan` has an FK (`schema.rs:292`), so a 12-step rebuild is needed for it. Three areas had each claimed `MIGRATIONS[0]`.                                                                                             |
| D2  | Temp policy                   | `SQLITE_TEMP_STORE=0` compile flag, plus `temp_store=FILE` in SETTINGS.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Temp data must not occupy RAM.                                                                                                                                                                                                                                                                                                                                                                                                                  |
| D3  | Connection roles              | Every connection has a role, including prepare's, Upgrade, the Rebuild phases, Groups, GroupsScratch, Edits, ThumbPass and ThumbStore, the Ui reader, **Names** (`names.sqlite`) and **Watch** (the USN watch's read-only reader). Each pool is per role.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | The values are in 1.2.                                                                                                                                                                                                                                                                                                                                                                                                                          |
| D4  | `connect()` home              | `pg_schema::env`, which also owns `attach_uri` and the hooks.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | No dependency cycle.                                                                                                                                                                                                                                                                                                                                                                                                                            |
| D5  | Work lists                    | Keyset cursors for import and borders. The thumbnail Listed and Walk tiers are **replaced** by per-height state in the cache file (D35). Materialization survives only in the paged retry list (D31).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Materialized lists are the RAM cost today.                                                                                                                                                                                                                                                                                                                                                                                                      |
| D6  | Cursor snapshot semantics     | A cursor yields **the subset of today's start snapshot that still qualifies when visited and has not been re-stamped** (`seq ≤ S0`). Skipped rows go to the next pass, and `lower_total` keeps `done == total`. This is a stated deviation (§7 Q-D).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | `store_failure`'s `current()` does not check `missing` (`lib.rs:1286-1289`). Exactness needs either an O(n) start materialization or a snapshot held for the whole pass; both were rejected (§5).                                                                                                                                                                                                                                               |
| D7  | `analysis_removed`            | One body: `next_seq`; `pending=1, borders_due=0, seq, changed`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | One definition for every removal path.                                                                                                                                                                                                                                                                                                                                                                                                          |
| D8  | `pending` stamping            | Writers set `pending` only. Trigger `files_pending_stamp` (AFTER UPDATE OF pending, WHEN old=0 AND new≠0) stamps `changed` and `seq`. `pend_batch` and the `pdq_heavy_attr` queue belong to `files_pending_on`. Clearing and the requeue belong to `files_pending_off` (D25). Upgrade Pass A suppresses **only** the changed/seq stamp, through an in-transaction `counts.bulk` flag. It never suppresses `pend_batch`, its clearing, the requeue or the attr queue. Every CASE test reads `pending <> 0`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Stamps come from triggers, so each trigger duty has one owner. Two triggers stamping `pend_batch` had to be separated, and the scope of the bulk suppression had to be defined.                                                                                                                                                                                                                                                                 |
| D9  | Root scope and unpaired index | **A root is the pair (anchor, key range).** Folder keys merge drives: the anchor is not part of the key (`rules.rs:196-215`; test `the_drive_is_not_part_of_a_folder_key`, `:331`). So the key range of `E:\Photos` also covers `D:\Photos`, and `E:\` covers everything. The indexes are `files_unpaired_fresh(unpaired_anchor, order_key) WHERE unpaired=1 AND volume IS NOT NULL` and `files_unpaired_kept(unpaired_anchor, order_key) WHERE unpaired=1 AND volume IS NULL`. **Upkeep:** `files.unpaired_anchor` is `folders.anchor` (an `anchors.id`, immutable per folder) while unpaired=1, and NULL otherwise. It is written in the same row write by every statement that sets `unpaired` or `folder_id`. A scan reads its root's (anchor, key range) into an id-sorted spill. Rows whose folder path fails the exact `under_root!` byte test are dropped, so the set equals today's `folder_id IN under_root!`. Reader masks (D34) use the same key-range prefilter plus the path test.                                                                                                                                                                         | Today's fresh set is root-scoped by `folders.path` (`pair.rs:200-210`; `under_root!` at `lib.rs:80-84`). `folders.path` compares bytewise (`under_params`, `lib.rs:99-115`), so the path test keeps case-variant folders out exactly as today. A folder's anchor never changes, so add_root and remove_root never re-key. The upsert's DO UPDATE moves `folder_id` while keeping `unpaired` (`lib.rs:1223-1227`), so the anchor must follow it. |
| D10 | Missing writes                | Per-row writes only, enforced by an authorizer allowlist. Removal uses per-row pages with a done-prefix watermark.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Folder-wide UPDATEs of `missing` and per-folder removal statements were rejected (§5).                                                                                                                                                                                                                                                                                                                                                          |
| D11 | Dups derivation               | The dups design, with identity counters derived from `hash_runs`. All counters live in the `counts` row.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Derivation from `hash_runs` gives exact identity counters.                                                                                                                                                                                                                                                                                                                                                                                      |
| D12 | Facets                        | Chunked, with a contribution log.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Bounded facet computation.                                                                                                                                                                                                                                                                                                                                                                                                                      |
| D13 | Import flush                  | Store only, plus a pairing worker. `pdq_hashes` is deleted.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | Import no longer needs an in-RAM hash session.                                                                                                                                                                                                                                                                                                                                                                                                  |
| D14 | pg-spill                      | ExtSort, and `Budget` with an **interactive 256 MiB share plus a background 256 MiB share**. Also SpillQueue and SpillStack (mark and truncate), the segment stack, PagedFile, the codec, ReorderSpill, ConcurrentReader, ScratchRoot with a **reservation ledger**, and Staged. Out of space is a typed error (Q4 policy). The tier-1 bulk build and the journal pass take only the background share. W is a separate single reservation (§2).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | Interactive work must not wait behind background sorts. A single 512 MiB budget conflicted with this split.                                                                                                                                                                                                                                                                                                                                     |
| D15 | Guard toolkit                 | One toolkit in `pg_schema` (feature `testing`). Phase order is fixed: the toolkit (1.3) comes before keyset (1.5).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Guards must exist before the code they guard.                                                                                                                                                                                                                                                                                                                                                                                                   |
| D16 | Leases                        | One `work_leases` table. Kinds: pair, flush, build, merge, compact, groups, upgrade, read, adhoc, scan. The SQL helper is shared with the thumbnail cache's lease. Staleness is a **heartbeat counter that does not change over the adopter's own monotonic 60 s**, so no clock agreement is needed. **Adoption allocates a new id and directory.** Only immutable checkpointed state carries over: it is copied, or handed over by rename inside `<catalog>.scratch` (the journal pass's sealed runs). A token is checked inside every write transaction. Owners re-verify after a pause and before each file-write unit. Scratch directories are named by lease.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Works without a shared clock. The cross-OS pause limit is R9.                                                                                                                                                                                                                                                                                                                                                                                   |
| D17 | Runner schedule               | **Requested work first:** `[Crops, Scans (resume a pending journal pass, drain outbox, resume pending pairing; journal pass or walk per 3.9), Removals (slices of at most 60 s; hidden by D34, so never urgent)]`. Then `Upgrade`. **Then idle work:** `[Import (+pair worker), PairBacklog (settle units), Borders, Thumbs, Groups, Names, Tier1Maintenance]`, round-robin in slices of at most 60 s or one checkpoint. Merges are debt-driven inside the pair worker. A pass returning `UpgradePending` or `Busy{until}` is skipped, and the runner re-wakes itself on the earliest deadline. CLI `status` and `dups` still drive removals first.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | `runner.rs:303` restarts the round from the top on `Step::Preempted`, so an unsliced removal ahead of Scans starved requested scans. The Names idle pass was missing from the schedule. Pairing no longer depends on its timing (D25).                                                                                                                                                                                                          |
| D18 | WAL                           | `journal_size_limit` 64 MiB. WriteBatch commits at a frame budget F = 16 MiB/page (via cacheflush and CACHE_WRITE) and waits when the live WAL (PASSIVE checkpoint `pnLog`) is above WAL_CEILING = 512 MiB.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | Bounds WAL growth during bulk writes. Each batch's over-count of frames makes commits early, never late.                                                                                                                                                                                                                                                                                                                                        |
| D19 | Open API                      | `Catalog::open(path, rules, Role)` everywhere. New errors: `Rebuilding`, `Migrating`, `UpgradePending{steps}`, `RebuildBlocked`, `KeptChanged`, `NoSpace`, `OutOfSpace`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Callers must say which role they are.                                                                                                                                                                                                                                                                                                                                                                                                           |
| D20 | Version recomputes            | They become Upgrade steps. `prepare` writes the new mark and step keys in O(1). Pass A rewrites the flags in id order with no index writes (tri-state flags). Every reader goes through cursor-relative `pending_now!` and `taken_now!`, so it is exact. Fixed seeded step keys.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Replaces an in-place recompute. Writing flags through live indexes was the cost (§5).                                                                                                                                                                                                                                                                                                                                                           |
| D21 | Temp dir                      | `<catalog>.scratch/sqlite` (one ScratchRoot name). The directory is created first. On Windows it is aliased with `openable_folder` (in the home's `sqlite-aliases`); on Unix the path bytes go to `sqlite3_temp_directory`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | Windows temp paths need the aliased form.                                                                                                                                                                                                                                                                                                                                                                                                       |
| D22 | Counters                      | One seeded `counts` row: status, work totals, write queue, band and heavy-band, identity. Each family has its own recount cursor, `2^62` when idle. A definition can change only through a Migration Upgrade recount or a rebuild. Present pairs = Σ bands.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | Per-family cursors are kept for post-freeze migrations.                                                                                                                                                                                                                                                                                                                                                                                         |
| D23 | Rebuild protocol              | Move aside at Start into rollback-journal mode (refused if the file is in use). Manifest plus owner heartbeat. Chunked copy with FKs off. Indexes from exact text. Ordered FK merge. Derived state from sequential merges and ExtSort. Classified errors with corruption salvage. Swap by file identity. A `.rebuilt` note until it is acknowledged.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | The change counter protects the kept file (§5 rejects a write fence that keeps the path). A two-hop swap was rejected, because D24 turns out-of-order UNIQUE constraints into droppable indexes.                                                                                                                                                                                                                                                |
| D24 | Uniqueness                    | Every UNIQUE or PK constraint on a large rowid table becomes a named unique index. Every FK child column is indexed. Both are enforced by lints.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Needed by the rebuild's ordered merges and by the lints.                                                                                                                                                                                                                                                                                                                                                                                        |
| D25 | PDQ pair definition           | Missing files are ignored (Q3). **Pending exclusion uses the import-session clock.** `meta.pdq_batch` is bumped at each `import()` start, only after `import_work`'s gate passes and the list is non-empty. `pdq_indexed.batch` is set at store. `files.pend_batch` is set when `pending` leaves 0 (`files_pending_on`). **formed(x newer, y) = NOT(y.pend_batch IS NOT NULL AND y.pend_batch < x.batch)**. NULL means formed, in SQL and in Rust alike. `files_pending_off` (pending nonzero → 0) always sets `pend_batch = NULL`, and queues `pdq_heavy_attr` for a heavy file. It queues a requeue only when a `pdq_indexed` row still exists, so a store (which deletes the row before its REPLACE) queues none. A pending revert without a store re-indexes the file.                                                                                                                                                                                                                                                                                                                                                                                               | Today's clock is the index load at import start (`lib.rs:881-913`, called from `import/src/lib.rs:40` only after the empty-list return). It holds every present, non-pending analysed file, whatever its pending history. This reproduces it exactly, and pairing results become independent of when the pair worker runs. Without the clearing rule a re-stored file never again pairs as the older endpoint, so the NULL guard is required.   |
| D26 | PDQ storage                   | Thin sheets (light files) and fat sheets (heavy files, full dihedral). Heavy core with exactly-once emission. Attribute planes (plane P mirrors `files.pend_batch`). Published generations. Cursor-idempotent writer. HEAVY_ENTER = 512.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | The storage forms that were rejected are in §5.                                                                                                                                                                                                                                                                                                                                                                                                 |
| D27 | Duplicates filter             | Pure SQL. `hash_runs.present` with `present_xor`, `pdq_near.n` over rows, and `pdq_heavy.near` plus an exact unsettled correction. Duplicate-status flips stamp `files.changed`, so Duplicates views get deltas.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Views update by delta, not by full rebuild.                                                                                                                                                                                                                                                                                                                                                                                                     |
| D28 | Ordered sinks                 | Non-blocking spilling reorder buffer (`Order::Index`). The pipeline carries the item through to the sink on every path.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Results reach sinks in index order without blocking workers.                                                                                                                                                                                                                                                                                                                                                                                    |
| D29 | Qt numeric edge               | Ranks and counts as exact f64 (n < 2^48 by the SQLite file format). File ids as decimal strings. The scene graph sees only viewport-relative px < 2^14, and the slot layer sits outside the Flickable's content item (P10). Rectangles snap to whole pixels (sub-pixel probe). Every queued rank is an `ItemRef`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Float32 misplacement past about 2^24 px rules out absolute positions in the scene graph.                                                                                                                                                                                                                                                                                                                                                        |
| D30 | View layer                    | `WindowView.qml` (a plain Flickable plus a non-moving slot layer that is a sibling of its content item, P10) over a Rust `SlotWindow`. The ListView behaviours the views relied on are ported from Qt 6.12 and pinned by a golden table.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Keeps the ListView behaviour the views relied on.                                                                                                                                                                                                                                                                                                                                                                                               |
| D31 | Read snapshots                | READ_TX_MAX = 2 s for every catalog and cache read, except the rebuild's building file. Long sets are read by converging paged reads, which give an exact snapshot at their final instant. The harness enforces a census.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Bounds WAL pinning by readers.                                                                                                                                                                                                                                                                                                                                                                                                                  |
| D32 | Gates                         | `Catalog::gate(Need)` is checked inside the catalog API (import_work, borders_work, crops, groups), never in callers.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Callers cannot skip a required upgrade step.                                                                                                                                                                                                                                                                                                                                                                                                    |
| D33 | GUI writes                    | No catalog write runs on the GUI thread. An edit worker plus a client overlay is held until the commit stamp is covered.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Keeps the GUI responsive during long writes.                                                                                                                                                                                                                                                                                                                                                                                                    |
| D34 | Pending removals              | Readers (grid build and refresh, tree, facets, status) exclude the rows of pending removals: the order-key range as a prefilter, plus the exact `under_root!` path test on the row's folder (D9). Same-named folders on other drives stay visible. Status corrects present and missing exactly. Analyzed, failed, pending and pdq_pairs are as-of with a note, and the Duplicates filter counts twins under the removed root until they are marked (§7 Q-D). CLI `status` and `dups` drive pending removals first, so the CLI stays exact.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | The mask hides a removal at once, without a long write.                                                                                                                                                                                                                                                                                                                                                                                         |
| D35 | Thumbnail pass state          | Per-height state in the cache file: gen, sync cursor, walk cursor, laps, exact and range members, trigger-logged `lost`, lease. The cache layout changes once and **starts over**, then freezes with BASELINE.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | Replaces the Listed and Walk tiers (D5).                                                                                                                                                                                                                                                                                                                                                                                                        |
| D36 | Startup                       | `prepare` is O(1) except for a rebuild or an exclusive RebuildTable migration. Those run on a worker behind a stoppable progress window.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Startup is never blocked for hours without a window.                                                                                                                                                                                                                                                                                                                                                                                            |
| D37 | USN journal rescan            | **State-based.** The journal only names dirty folders: record parents, row folders, exception, stale and link parents, and directory candidates. **Listings through the walk's own `visit_folder` are the truth:** relisted folders replay every entry, and big folders are probed one name per new handle. **Cursor:** per root, holding the journal id, LowestValidUsn, next USN and a last-record fingerprint. FirstUsn is checked before and after the read. Roots with equal cursors share reads. **Report:** the mirror `usn_dirs` plus a census give exact ScanStats. **Order:** non-subtree intents are sorted by walk_key and merged with in-place sub-walks through the same ScanWriter, so ids and winners equal the walk's. **Staleness:** affected refs are opened by id for link counts and live values, and open sessions are tracked. **Access:** a logon-independent token projection, with an O(dirs) re-probe when it changes. **Read path:** unprivileged FSCTL_READ_UNPRIVILEGED_USN_JOURNAL on the volume-root handle, gated by a per-build self-test. The privileged read is used only when elevated. Every other failure falls back to the walk. | Q-S1 (b). Details in 3.9 and §5.                                                                                                                                                                                                                                                                                                                                                                                                                |
| D38 | Seen floor                    | `scans.floor`: a walk's floor is its own id. A journal pass's floor is its root's last full walk. Unseen ⇔ `file_seen.last_scan < floor`. A pass marks a row unseen by stamping the seeded sentinel `scans.id = 0`. mark_missing flips the unseen set in FolderPages/present_page order. Journal passes take SkipMerge from `usn_dirs_bad`. seen_identity stamps only rows of the scanned root.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | Avoids O(n) stamps per pass. The statements are unchanged apart from parameters, and the trigger stamp order equals the walk's.                                                                                                                                                                                                                                                                                                                 |

## 4. Phases

**Commit policy.** Phases 1-8 are one commit each, on the checked-out branch, when a commit is asked for (the standing rule, `BACKGROUND_JOB_AUDIT_FIXES.md:25`: commit only when asked). After all of them, run a single `just build && just lint && just test` and fix forward. Intermediate commits do not compile. Forward dependencies are marked with `→`.

**Oracles.** Every equivalence oracle is the old code or SQL kept under `cfg(test)` or the `fixture` feature.

### Phase 0: Preconditions (the user's)

- **0.1** The user lands the in-flight S-17 Unit E work before Phase 1 starts. Phases 1-2 depend on two parts of it:
  - `crates/platform/src/sqlite.rs`: `openable_folder` and `attach_bytes`;
  - the `carry_rows` `attach_bytes` change.

  The unrelated untracked picker and yandex edits are the user's.

### Phase 1: Shared foundations (no behaviour change)

**1.1 Temp storage on disk:**

- **Files:**
  - `.cargo/config.toml`
  - `crates/catalog/src/schema.rs` (SETTINGS)
  - `crates/thumbcache/src/lib.rs` (remove the MEMORY pragma)
  - `vendor/README.md`
  - `docs/plans/DATABASE.md`
- **Change:**
  - `LIBSQLITE3_FLAGS = "SQLITE_TEMP_STORE=0 SQLITE_DEFAULT_MEMSTATUS=0"` (forced; the fork route's value, as `catalog/src/core.rs` asserts).
  - SETTINGS gains `temp_store=FILE` and `journal_size_limit=67108864`.
  - Every production ORDER BY is total.
- **Tests:**
  - `catalog/tests/temp_storage.rs`: the compile options; a 400k-row sort raises the highwater by less than 8 MiB; a 200k-row trigger UPDATE journals on disk.
  - `tests/no_memory_temp.rs` lint.

**1.2 `pg_schema::env`: connect, roles, hooks, URIs:**

- **Files:**
  - `crates/schema/{Cargo.toml (+pg-platform), src/env.rs, src/lease.rs}`
  - `crates/platform/src/sqlite.rs` (`attach_uri(path, Mode::{Ro,Rw})`)
  - every `Catalog::open` caller (D19)
  - `app/src/app.rs` (split pools)
- **Change:**
  - **SqliteEnv.** `SqliteEnv{temp_dir (D21), cache: CacheBudget, hard_limit}`.
  - **connect(path, flags, Role)** sets up:
    - SETTINGS plus the role cache;
    - a 1 ms busy poll, up to 5 s;
    - one composed progress handler: the cancel and interrupt flag, plus a step counter under `instrumented`;
    - a statement cache of 64;
    - the ledger trace and statement tags;
    - under `instrumented`: the **authorizer** (an SQLITE_UPDATE on `files.missing` is denied unless the statement's tag is allowlisted, step 3.8) and **read-transaction span recording** (D31);
    - a **wal_hook** that chains the default autocheckpoint and feeds the high-water sampler.
  - **attach_uri.** Every ATTACH of the catalog uses a percent-encoded `file:` URI with `mode=ro` or `mode=rw`, and never CREATE. The callers are thumbcache `pending_in` (`lib.rs:768`, today a default-flags connection) and ThumbPass.
  - **Role table (MiB):**
    - Writer 256 (runner, prepare, Upgrade, Scans including journal passes);
    - Pair 64; ListLane 64; BrowseLane 64;
    - Ui reader 16 (`query_only`); Edits 16; Provider 8 × provider threads (8);
    - Keeper 16; WriteBack 16; ThumbPass 128 main + 64 catalog; ThumbStore 16;
    - Groups 64; GroupsScratch 16 (via connect, with no hardcoded `cache_size`);
    - Names 16 (`names.sqlite`, 7.16); Watch 4 (`query_only`, the USN watch, 3.9);
    - Rebuild: phase caches per D23, scaled to min(128, budget/2.5) under the harness;
    - Steady total 884 MiB, about 0.86 GiB.
  - **Pools.** `ui` is size 1 and is used only on the GUI thread (debug-assert). `provider` is capped at the provider thread count.
- **Tests:** `schema/tests/env.rs`:
  - per-role `cache_size`;
  - temp files appear in temp_dir and are gone afterwards;
  - a second `apply` with different values errors;
  - cancel still interrupts while steps count;
  - an `attach_uri` to an absent path creates nothing;
  - the authorizer denies a crafted folder-wide `missing` UPDATE;
  - a second process's frames are visible to WriteBatch through `pnLog`.

**1.3 Guard toolkit** (ordered before keyset)

- **Files:**
  - `crates/schema/src/testing.rs`
  - `crates/catalog/src/fixture/plan.rs` (`with_billion_stats`)
  - existing guards converted
- **Change:**
  - `Plan::of` (EQP plus an opcode census, including subprograms).
  - `assert_streaming`, `assert_temps`, `assert_bounded(allow)`, `assert_write_fanout`.
  - `steps_of`.
  - Every guard runs with no stats, and again with 1B-shaped stats.
- **Tests:** the toolkit's own tests, on crafted statements.

**1.4 Write discipline:**

- **Files:**
  - `crates/catalog/src/batch.rs` (new)
  - `lib.rs` (`write_transaction`)
  - `crates/jobs/src/control.rs` (`checkpoint_with`)
- **Change:**
  - **Gate:** a FIFO ticket per catalog path. Plus `WriteTx` and `read_transaction`.
  - **WriteBatch** opens lazily and commits at `rows`, at 250 ms, or when the transaction's frames reach **F = 16 MiB / page_size**. Every 64 driving rows it calls `sqlite3_db_cacheflush` and reads the `SQLITE_DBSTATUS_CACHE_WRITE` delta. That count over-counts frames, so commits come early, never late. Bound: at most (F + 64·P) pages, about 26 MiB, with P (pages per row) pinned in 3.8.
  - **Before each batch:** a `PRAGMA wal_checkpoint(PASSIVE)` reads the live `pnLog`. Above WAL_CEILING it tries `RESTART` with a 50 ms busy budget, then sleeps 100 ms (stoppable). Foreground WriteTx are exempt.
  - The lock yields 2 ms after being held for 1 s.
- **Tests:**
  - gate FIFO under 8 threads; no BEGIN before the first write;
  - commits at rows, age and frames (a statement dirtying 50 random pages per row commits at most F + 64·50 frames);
  - `set_rating` succeeds within 150 ms during a batch loop and while a reader holds a 5 s snapshot;
  - a second process grows the WAL past the ceiling and this process waits.

**1.5 Keyset and subtree primitives:**

- **Files:**
  - `crates/schema/src/keyset.rs`
  - `crates/catalog/src/subtree.rs`
- **Change:**
  - `KeySeek`: NULL-aware multi-key seeks, plus a single-key form. Rule: exactly one bounded lower limit per indexed column.
  - `folder_in_root!`, `FolderPages`, and `SkipMerge` with `reseed(cursor_path)`: O(depth) exact root probes, since roots are an antichain (`lib.rs:786-800`). SkipMerge takes a `SkipSource` (`ScanSkipped(scan)` or `UsnBad(root)`, 3.3).
  - **`root_range(root)`** (D9): the (anchor id, key lo, key hi) of a root, plus `folder_under!(folder_id, path)`, the exact `under_root!` byte test as a folders PK probe (FolderCache-backed in Rust). For a drive or UNC root, key hi is NULL. Every range text therefore uses `(hi IS NULL OR order_key < hi)`.
  - The `paged(read_page, apply_write)` driver.
  - The concrete `present_page` text lands in 3.3, because it needs `file_seen` from 2.1.
- **Tests:**
  - drains equal `ORDER BY` at budgets 1, 2, 7 and 1024;
  - SkipMerge equals the `under_root` oracle, including sibling, nested, UNC and drive cases, and after a reseed;
  - `root_range` plus `folder_under!` equals `folder_id IN under_root!` on two drives with mirrored layouts, case-variant folders and a drive root;
  - no TEMP B-TREE.

**1.6 `pg-spill` and platform primitives:**

- **Files:**
  - `crates/spill/src/{lib, extsort, budget, spill, segstack, paged, codec, reorder, concurrent, scratch}.rs`
  - `crates/platform/src/{file.rs (Staged, read_at/write_at, file identity), volume.rs (free_bytes, mount_root, detachable, volume_root_handle), process.rs (peak_private_bytes, run_in_job)}`
  - `crates/platform/src/usn.rs` (new: Journal trait, query, reads, wait_read, parser, resize, open_by_id, links, dir_path, unpriv_selftest, collation, VolumeGuard)
  - `crates/platform/src/token.rs` (new: access_projection, change_notify_enabled)
  - `crates/platform/src/dir.rs` (probe_entry, first_call)
  - `Cargo.toml`: windows-sys features Win32_System_Ioctl, Win32_Security, Win32_Storage_FileSystem, Win32_UI_WindowsAndMessaging, Win32_System_IO and Wdk_Storage_FileSystem (checked against 0.61)
  - `app/src/store.rs`
- **Change:**
  - **ExtSort:** stable, fan-in 64, `merge_reduce`.
  - **Budget:** a process-wide semaphore with an interactive share (256 MiB, grid) and a background share (256 MiB: dups, pdq bulk, review, picker, journal pass). Background holders never take the interactive share.
  - **SpillQueue and SpillStack.** SpillStack has `mark` and `truncate_to` in O(1).
  - **Segment stack** for borders ancestor tails.
  - **PagedFile.**
  - **Codec:** 64 KiB blocks that decode independently. A frame record holds:
    - the name in WTF-8 or UTF-16LE, whichever is shorter;
    - front coding against the previous name;
    - a zigzag varint id delta;
    - the volume, only when it differs.

    Worst case 27 + 2·units B.
  - **ReorderSpill:** 64 MiB segments with live counts, plus a PagedFile offset table of 12 B per index.
  - **ConcurrentReader:** IOCP with overlapped ReadFile on Windows, and N reader threads elsewhere. Queue depth 128 on SSD, 32 on seek-penalty volumes. Elevator sort per submission window.
  - **ScratchRoot** `<catalog>.scratch/`:
    - a reservation ledger (bytes reserved per lease in `work_leases.reserved`), checked against `free_bytes` before any work, with priorities: interactive > pass > maintenance;
    - every owner is lease-named: SQLite temp, scan frames, journal-pass runs, borders, reorder, picker, groups scratch, adhoc, rebuild, tier1 work, runs, tree generations and the facet log;
    - `hand_over(old_lease, new_lease)` renames a sealed directory (D16);
    - startup sweeps only stale leases, through lease GC.
  - **Staged.**
  - **mount_root:** lexical: a drive, `\\server\share\`, or the library root on POSIX.
  - **USN platform.** Windows only. Every other OS returns Unsupported. Everything sits behind `trait Journal` (with `Listing`), so FakeVolume and fault wrappers can stand in.
    - **JournalHandle::open(`X:\`):** FILE_READ_ATTRIBUTES|SYNCHRONIZE, share RWD, BACKUP_SEMANTICS|OVERLAPPED. Unprivileged.
    - **`query()`:** returns `{journal_id, first_usn, next_usn, lowest_valid_usn, max_usn, max_size, alloc_delta}`. Codes 1179 → NotActive, 1178 → Deleting, any other → Unavailable.
    - **`read(start, journal_id, end, each)`:**
      - READ_USN_JOURNAL_DATA_V1 with ReasonMask !0, ReturnOnlyOnClose 0, versions 2-3, via FSCTL_READ_UNPRIVILEGED_USN_JOURNAL (0x903AB).
      - The privileged FSCTL_READ_USN_JOURNAL on `\\?\Volume{GUID}` is used only when the token is elevated.
      - Fixed 1 MiB buffer.
      - Safe parser: length bounds, alignment 8, V3 high id bits 0, USN strictly increasing, otherwise Malformed. Names are never read.
      - Codes 1181 → EntryDeleted, 87 → BadStart.
    - **`wait_read`:** overlapped, cancellable by CancelIoEx.
    - **`resize`:** FSCTL_CREATE_USN_JOURNAL. Elevated only; keeps the ID.
    - **`open_by_id`:** returns `Live{end_of_file, last_write, creation, attributes, number_of_links}`. `links()` reads FileHardLinkInformation into a 1 MiB buffer; `dir_path()` reads FileNormalizedNameInfo. Codes 87, 2 and 3 → Gone; 5 → Denied.
    - **`probe_entry(dir, name)`:** opens a NEW directory handle per name, because the FileName pattern is fixed by the first query on a handle. It loops with RestartScan = FALSE until an exact UTF-16 match or no more files, so POSIX `<`, `>`, `"`, `*` and `?` names and case-sensitive directories are handled. It yields the same `Entry` as `Lister::each`.
    - **`first_call(dir)`:** Lister::open plus one listing call: the walk's own access check.
    - **`detachable()`:** true when the bus type is one of {Usb, 1394, Sd, Mmc, Virtual, FileBackedVirtual}, or RemovableMedia is set, or IOCTL_STORAGE_GET_HOTPLUG_INFO reports DeviceHotplug or MediaRemovable.
    - **`access_projection()`:** BLAKE3 over a canonical encoding of:
      - TokenUser;
      - enabled and deny-only group SIDs, except SE_GROUP_LOGON_ID groups (logon-type SIDs are kept);
      - restricted SIDs;
      - the AppContainer SID and capabilities;
      - the integrity SID;
      - the enabled state of SeBackup, SeRestore and SeChangeNotify.
    - **`unpriv_selftest(dir)`:** denies itself access to a file F and to a folder G, writes F and G\c through held handles, and requires those records in the unprivileged read. Result: Verified, Filtered or Unsupported.
    - **Collation:** a 64K upcase table (RtlUpcaseUnicodeChar), `collate`, a prefix-free `collation_key`, and `is_table_independent` (every unit < 0x80).
    - **VolumeGuard:** a message-only window with DBT_DEVTYP_HANDLE registration. On DBT_DEVICEQUERYREMOVE or GUID_IO_VOLUME_LOCK it cancels and closes the handles and raises a stop for that volume. It re-arms on unlock or a failed removal.
    - **Not used:** per-file FSCTL_READ_FILE_USN_DATA; the device handle for reads; a FILE_READ_DATA volume handle; NtQueryDirectoryFileEx pattern reuse; the volume's `$UpCase`.
    - **Memory:** a fixed 1 MiB journal buffer, 1 MiB link buffer, 64 KiB probe buffer, 128 KiB upcase table, and 64 KiB per watched volume.
- **Tests:**
  - ExtSort equals a stable sort;
  - SpillStack proptest, including mark and truncate;
  - codec proptest (non-UTF-8 names, unpaired surrogates, CJK, random 64-bit ids, block edges); 1M NTFS-like frames average at most 20 B;
  - ReorderSpill: segments are deleted when they empty;
  - ConcurrentReader gives the same bytes as ReadAt, with at most QD in flight;
  - the reservation ledger refuses an oversize reservation before work starts, and never sweeps a live lease; `hand_over` is a rename;
  - `run_in_job(200 MB)` stops a 400 MB hog;
  - **USN platform:**
    - parser vectors, truncation at every byte, and fuzzing (never panics);
    - real NTFS unelevated (PG_TEST_DIR): query and read see the test's own create, rename, link and delete, with the right refs, parents and reasons;
    - a mid-record start gives 87;
    - probe_entry: at least 3 probes in one 20K-entry folder; POSIX `<`, `>` and `"` names; a case-sensitive Foo/FOO pair;
    - links() lists both links;
    - unpriv_selftest is Verified on Windows 10 build 19045;
    - access_projection is equal across two logons of the same user, and differs between elevated and not;
    - detachable() is true for a VHD;
    - FSCTL_LOCK_VOLUME succeeds while a wait_read is armed.

**1.7 pg-model shared types:**

- **Files:** `crates/model/src/{pdq.rs, ids.rs, num.rs, dups.rs, grid.rs, path.rs}`
- **Change:**
  - `min_quality!()`, `PdqSeq`, FolderCache (which also caches folder path and anchor for `folder_under!`), `Listed` and `Fetched`.
  - **Numeric contract:**
    - `Rank(u64)` (serde-transparent), `GlobalRow`, `ListEpoch`, `ItemRef{rank, epoch, file}`;
    - `LayoutY(u128)`, in 2^-32 px, with `from_local` and `to_local`;
    - `MAX_CATALOG_BYTES = 2^48`, from the file format: `typedef u32 Pgno` (`sqlite3.c:16993`) and page size at most 65536 (`:14995`);
    - `rank_to_qml` and `rank_from_qml`, and `FileId::to_decimal` and `parse_decimal`.
  - **PDQ model** (D25):
    - the pair definition `formed(x, y) = !(y.pend_batch.is_some_and(|b| b < x.batch))`, `HEAVY_ENTER=512`, `PairKey`, `PairRecord`, `Band`;
    - the parity lemma.
  - **dups:** `group_distance!`, `pdq_bands!`. There is no `COUNTS_KEY` (D22).
  - Const assertions: every record id width (entry id, query index) against its cap; PDQ_BANDS partitions 0..=31.
- **Tests:**
  - round trips at 0, 2^31±1, 2^32+7, 2^46 and 2^48-1;
  - `rank_from_qml` rejects NaN, infinity, fractions and 2^53;
  - LayoutY associativity is exact;
  - weight and parity proptests;
  - legacy-free seq ordering;
  - `formed` with a NULL or None `pend_batch` is true, and agrees with the SQL text on random inputs.

**1.8 pg-jobs pipeline and progress:**

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
    - a sink returning false is final.
  - Control gains `lower_total`, `raise_total`, and `Phase::{Pair, Index, Groups, Upgrade, Rebuild, Removal, ScanJournal, AccessReprobe}`.
  - Messages: `PassPair`, `PassIndex`, `PassGroups`, `PassUpgrade`, `PassWorkingOpen`, `PreparingCatalog`, `PreparingPhase`, `RemovalInProgress`, and the journal notices (`JournalInactive`, `JournalWrapped`, `JournalBaseWrapped{needed}`).
- **Tests:**
  - paged items are read once, in grant order;
  - one 500 ms-slow head among 10^5 fast items: RAM stays at most window_bytes, the spill is deleted afterwards, sink indices strictly increase, and workers stay busy;
  - a run of failed opens with window 1 in Index mode;
  - a stop during 10^6 Empty fetches ends within one fetch;
  - after sink returns false, no further sink calls;
  - FeedFailed comes after the in-flight sinks.

**1.9 Lease helper:**

- **Files:** `crates/schema/src/lease.rs`
- **Change:** These are SQL texts over a `work_leases`-shaped table. The catalog's table is created in 2.1, and the cache's lease reuses the helper.
  - `claim(kind, target, unique)`, which returns an id and a token;
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

**2.1 BASELINE objects** (the single schema edit)

- **File:** `crates/catalog/src/schema.rs`. Literals come from macros (`max_attempts!`, `match_distance!`, `group_distance!`, `min_quality!`).
- **files table:**
  - Drop `last_scan`.
  - Add `borders_due INTEGER NOT NULL DEFAULT 0` (0, 1 or 2), `pend_batch INTEGER` (NULL unless pending, D25), and `unpaired_anchor INTEGER` (D9). `pending` is documented as 0, 1 or 2.
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
  - Add `anchor INTEGER NOT NULL`: the `anchors.id` of the path's drive or UNC share. It is immutable, like `path`. It is declared without REFERENCES, so it needs no FK index; a lint checks it against `anchors`.
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
    - Which columns come when: `id`, `path`, `started` and `deferred` are in the schema-1 baseline (`crates/catalog/src/baseline.sql`). `hidden`, `report`, `finished`, `cursor_path` and `cursor_id` come in TO_10 with the unique index on `path WHERE finished IS NULL`, as `BACKGROUND_JOB_AUDIT_FIXES.md` (the `root_removals` line, 54) states. `key_lo` and `key_hi` come with the reader mask, later.
  - `scans` gains `pair_tier`, `pair_cursor` (the pairing continuation, 3.2), `finished`, `kind` (0 walk, 1 journal, 9 sentinel), `floor` (D38; NULL means its own id), and `usn_state BLOB` (phase, committed walk_key position, seal id, E, fingerprint).
  - Seed the sentinel `scans(id 0, root X'', started 0, finished 0, kind 9)`. Readers that list scans use `id > 0`.
- **USN journal tables:**
  - `usn_builds(build PK, result)`.
  - `usn_volumes(serial PK, collation_ok, collation_note)`.
  - `usn_roots(root_id PK → roots CASCADE, volume, root_ref, journal_id, lowest_valid_usn, base_scan → scans, next_usn, fp_usn, fp_ref, fp_reason, fp_time, access_proj, rules_hash, verify_due, last_reason, census: folders, files, entries, reparse_files, extra_links, foreign_ids)`. The base_scan FK is indexed by `usn_roots_base(base_scan)`, as the D24 FK lint requires.
  - `usn_dirs(id PK, root_id, volume, ref, parent_ref, path, wkey, state (0 listed, 1 unreadable, 2 reparse), error, entries, media, reparse_files, counted)` with:
    - `usn_dirs_ref` UNIQUE(volume, ref, root_id);
    - `usn_dirs_path` UNIQUE(path);
    - `usn_dirs_parent`(volume, parent_ref);
    - `usn_dirs_bad`(root_id, path) WHERE state <> 0.
  - `usn_entries(root_id, volume, ref, parent_ref, name, kind (1 multi-link, 2 reparse media, 3 Foreign), PK(all but kind)) WITHOUT ROWID`, with `usn_entries_parent` and `usn_entries_foreign(volume, ref) WHERE kind = 3`.
  - `usn_stale(root_id, volume, ref, open_session, PK(root_id, volume, ref)) WITHOUT ROWID`.
  - `usn_wake(root_id, volume, ref, PK) WITHOUT ROWID`.
  - Scratch: `usn_drop(scan, folder_key, file_id, PK) WITHOUT ROWID`, where folder_key is FolderPages' order, so a PK scan equals present_page order; and `usn_scope(scan, path, kind (0 folder-only, 1 subtree), PK(scan, path)) WITHOUT ROWID`.
- **Grid:** `files_log(seq PK, id, what)`.
- **PDQ** (D25):
  - `pdq_indexed(seq PK, file_id → named unique pdq_indexed_file, batch)`. It has no `pend_batch`; that lives on files.
  - `index_runs` (kind pdq or pdq_heavy; state staged, live or retired; `retired_gen`);
  - `pdq_heavy(file_id PK, seq UNIQUE, gen, batch, dihedral, counted, near, transfer)`;
  - `pdq_heavy_stage`, `pdq_band_stage`, `pdq_heavy_gone` (with `acct` and `groups` flags, and `pend_batch` copied from `files.pend_batch`), `pdq_heavy_flip`, `pdq_heavy_attr`, `pdq_requeue`, and the view `pdq_heavy_unsettled`;
  - `pdq_pairs` gains `CHECK(distance BETWEEN 0 AND match_distance!())`;
  - `pdq_near(file_id PK → files CASCADE, n)`.
- **Dups:**
  - `dup_member`, `dup_group_member`, `dup_group`, `dup_group_rank`, `dup_log`;
  - `hash_runs(kind, hash, total, present, present_xor, PK(kind, hash)) WITHOUT ROWID`.
- **Shared:**
  - `work_leases(id, kind, target, owner, token, beat, inputs, seq_lo, seq_hi, dead, progress, reserved)`, with unique partial indexes for pair, groups, upgrade and scan (per root);
  - the `counts` row (D22): files, present, pending, borders_due, analyzed, failed, failed_retryable, writes, writes_failed, writes_left, band_0..3, heavy_band_0..3, the 4 identity counters, the per-family recount cursors, and `bulk`. `SEEDS += ('counts','one')`.
- **Thumbnails, catalog side** (D35):
  - `catalog_opens(id PK, seq, nonce)`;
  - views `thumb_clock(seq, epoch, horizon, extent)`, `thumb_walk`, `thumb_removed`, `thumb_opens`.
- **Meta seeds:** every key is fixed and seeded, and none is ever added or deleted at run time:
  - `view_epoch`, `log_horizon`, `thumb_epoch`, `opens_ack`;
  - `pdq_seq`, `pdq_paired_hi`, `pdq_batch`, `pdq_write`, `heavy_vis`, `heavy_gen_next`;
  - `lease_next_id`;
  - `groups_state='pending'`, `groups_version`, `groups_gen`, `groups_consumed`, `groups_transition`, `groups_count`, `groups_files`;
  - `upgrade_import`, `upgrade_borders`, `upgrade_taken`, `upgrade_counts_<family>` (four families), `bulk_walk_import`, `bulk_walk_borders`.
- **Triggers:**
  - **grid:** `files_removed`, `files_renamed`, `analysis_cut_changed`, `analysis_shape_changed`, `analysis_removed` (D7);
  - **work lists:** `files_identity_changed`;
  - **D8:** `files_pending_stamp` (changed and seq only; suppressed by `counts.bulk`);
  - **folders.present:** three counter triggers;
  - **PDQ:**
    - `pdq_indexed_removed`;
    - `files_pending_on` (old=0 AND new<>0): sets `pend_batch = meta.pdq_batch`, and queues `pdq_heavy_attr` for a heavy file;
    - `files_pending_off` (old<>0 AND new=0): sets `pend_batch = NULL`, queues `pdq_heavy_attr` for a heavy file (with the NULL write), and queues a requeue only when a `pdq_indexed` row exists;
    - `files_heavy_presence`;
    - `pdq_heavy_removed`, which copies `files.pend_batch` into `pdq_heavy_gone`;
    - the dead-count triggers for both run kinds.

    None of these is suppressed by `counts.bulk`.
  - **dups:** the coalesced log with a gen refresh and the overflow switch (6.6), band counters, `hash_runs` (with `present_xor`) and `pdq_near` upkeep, and the shared files BEFORE DELETE trigger (presence-off transition first);
  - **counters:** about 12 triggers, each gated by its family cursor;
  - **Exit triggers:** turn flag 2 into 1 on a key change or a missing flip;
  - **Unpark trigger:** `files AFTER UPDATE OF folder_id, missing`;
  - **USN:** `files_usn_wake_u` (AFTER UPDATE OF volume, file_ref, folder_id) and `files_usn_wake_d` (AFTER DELETE). Each runs `INSERT OR IGNORE INTO usn_wake SELECT root_id, volume, ref FROM usn_entries INDEXED BY usn_entries_foreign WHERE kind = 3 AND volume = OLD.volume AND ref = OLD.file_ref`. That is one partial-index probe, nearly always empty.
- **Lists:**
  - CARRIED and NOT_CARRIED: every `usn_*` table is NOT_CARRIED (one walk per root after a rebuild). The wake triggers are recreated by BASELINE.
  - SMALL_TABLES = meta, session, views, roots, root_removals, scans, scan_skipped, anchors, usn_builds, usn_volumes, usn_roots.
  - LARGE_TABLES += usn_dirs, usn_entries, usn_stale, usn_wake, usn_drop, usn_scope.
- **Tests:**
  - `CARRIED ∪ NOT_CARRIED == tables`;
  - the seeded key set (scans 0 included) equals the expected set, and `mismatch()` is None at every Upgrade stop point;
  - lints: autoindexes only on SMALL_TABLES; every FK child column indexed; no upsert on analysis; no OR REPLACE on files or file_writes; every `folders.anchor` exists in `anchors`;
  - FK-cascade child triggers fire (pinned);
  - each partial-index `INDEXED BY` text prepares;
  - the dups DDL snapshot forces a `GROUPS_VERSION` bump;
  - the wake trigger's EXPLAIN uses `usn_entries_foreign`.

**2.2 Catalog rebuild: chunked, resumable, crash-safe:**

- **Files:**
  - `crates/catalog/src/rebuild.rs` (the file already exists; this step extends it)
  - `schema.rs` (CARRY specs; the old `copy_tables` kept under `cfg(test)` as the oracle)
  - `lib.rs` (`prepare`, `open`, `rebuild`, `make_replacement`, `move_aside`, `acknowledge_rebuilt`)
  - `platform/src/{sqlite.rs, file.rs, volume.rs}`
  - `schema/src/env.rs`
  - `docs/plans/DATABASE.md`
- **Change.** A state machine driven by `<catalog>.rebuild`, the manifest: JSON written by Staged, plus fsync. It holds:
  - the build digest, the reason, the kept base name and the building path;
  - the kept file's change counter and size;
  - the built file's OS identity;
  - the dup-carry decision, the phase and the counts.

  The heartbeat goes in `<catalog>.rebuild.owner` (1.9). Phases: Start → Copy → Index → Verify → Derive → Finish → Swap. Every step is one transaction on `.building` that also writes its progress row.

  1. **Detect.** In prepare, under `pg_schema::Lock`:
     - if a manifest exists, resume (a different digest restarts at Copy);
     - otherwise run the mismatch check.

     A file at version v < current is rebuilt to v, then migrated.
  2. **Start.**
     - Checkpoint the old file (TRUNCATE, checking busy=0).
     - `PRAGMA journal_mode=DELETE` must return 'delete'. It succeeds only when no other connection is open. Otherwise the result is Error 'catalog in use' (deviation h).
     - Assert that no -wal, -shm or -journal file is left.
     - Check free space (step 9).
     - Choose the first free `.replaced[.n]` base, record it, and rename the one main file.
     - An unreadable source renames its parts individually.
     - Resume renames only to the recorded base, idempotently.
     - A foreign file at the path gives `RebuildBlocked{path}`, with nothing moved.
  3. **Connections.**
     - Building: journal_mode=DELETE, synchronous=FULL, `main.locking_mode=EXCLUSIVE`, foreign_keys=OFF, temp_store=FILE. Cache 64 MiB for Copy, Verify and Derive; 128 MiB for Index (the PMA is sized from it).
     - Source: `ATTACH attach_uri(kept, Ro) AS carried`, `carried.locking_mode=EXCLUSIVE`, 16 MiB.
     - Kept integrity is checked at every resume, at every phase boundary, after any CORRUPT, NOTADB or IOERR, and before Swap: no kept-wal, no hot journal, and the change counter and size match. Otherwise the result is `KeptChanged`: building is discarded, and the rebuild restarts at Copy.
     - First run on building: `bring_to(BASELINE)`, then save and drop every secondary index and trigger into `rebuild_objects`.
  4. **Copy.**
     - Each CARRY table, in batches of at most 100k rows, by rowid or KeySeek PK, with `INSERT ... SELECT ... WHERE key > ?c AND key <= ?hi ORDER BY key`. These are appends and are not journaled.
     - The 2.2 conversions run inline: `borders_due`; `file_seen`; the `temp` cast; `anchors` and `folders.anchor` from `folders.path`; `unpaired_anchor` from the row's folder when unpaired=1; and `pend_batch`, which depends on the source:
       - a legacy (pre-BASELINE) source has no `files.pend_batch`, so it is set to 0 for pending rows and NULL otherwise;
       - a BASELINE-shaped source carries `pend_batch`, `pdq_indexed.batch` and `meta.pdq_batch` verbatim.

       Rebuild oracle case: a BASELINE catalog with pend_batch = k and unpaired seqs pairs the same after the rebuild.
     - `pdq_pairs` is filtered to `distance BETWEEN 0 AND 31`, and the rows it drops are counted into `dropped_pairs`. Radius 31 means none exist.
     - Legacy catalogs (no `pdq_heavy`) do not copy `pdq_pairs`. Writer seqs are assigned in file_id order, with `batch = 1` and `pdq_paired_hi = 0`. A pending row's `pend_batch = 0 < 1` excludes it, exactly as today's index load does.
     - `usn_*` is not carried.
  5. **Index.** One saved index per transaction, from its exact text, through SQLite's sorter.
  6. **Verify.** For each FK child, a paged, resumable, ordered merge (`SELECT DISTINCT col ... INDEXED BY ... ORDER BY col` against the parent's id order). An orphan is a data error.
  7. **Derive.** All of it is sequential merges plus ExtSort plus key-order appends, and resumable:
     - `folders.present`: a keyset over `files_present` in 100k-entry pages;
     - the `counts` status columns;
     - `hash_runs` (with `present_xor`) and the identity counters;
     - `pdq_near` and `band_0..3`: two ExtSort passes over pairs;
     - the dup tables carried if `groups_version` matches, otherwise `groups_state='pending'`;
     - heavy state carried with `index_runs` (validated by catalog_id).

     Derived state is always recomputed from rows, never carried verbatim (deviation d).
  8. **Errors.**
     - Environmental (FULL, IOERR, CANTOPEN, interrupt or stop, NoSpace, KeptChanged): Err, and the manifest is kept for resume.
     - Data (UNIQUE, FK, CHECK, NOT NULL): `Carried::Nothing`, as today.
     - CORRUPT or NOTADB from carried with an unchanged counter: retry once on a fresh attach. If it reproduces, **salvage**: bisect the table-key ranges, skip and record unreadable ranges, delete only the children of skipped parents, and report "N rows in M unreadable ranges dropped".
  9. **Free space.** Checked at Start, at each phase and at each resume. Needed = Σ remaining outputs (row bound × sampled bytes per row ÷ 0.9) + 2 × the largest remaining index + 1 GiB. A shortfall gives `NoSpace`.
  10. **Finish.**
      - `locking_mode=NORMAL`.
      - One transaction: meta, `view_epoch += 1`, `log_horizon = seq`, the `counts` row, the triggers recreated from their saved text, and the progress tables dropped.
      - Record the OS identity.
  11. **Swap.**
      - Assert that no building journal exists, rename, and verify the identity on resume.
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
  - (j) Corruption is salvaged by range, instead of the whole library being dropped.
- **Tests:**
  - The equivalence oracle at batch sizes 1, 2, 7 and 100k: full dumps, the sqlite_schema set, and the derived oracles (including `folders.anchor`, `unpaired_anchor` and `pend_batch`);
  - `fail_after(n)` at every commit, manifest write and rename;
  - a half-done move resumes;
  - a Windows child-process kill resumes;
  - an orphan or duplicate gives Nothing;
  - `max_page_count` forcing FULL keeps the manifest;
  - heartbeat: a 120 s statement is not taken over;
  - a write to kept gives KeptChanged;
  - a corrupt page is salvaged; a transient one is retried;
  - after Swap there is no journal;
  - the thumbcache ATTACH creates no file at the path;
  - Derive is resumable mid-folder;
  - a legacy catalog re-pairs to the oracle;
  - after a rebuild, an NTFS root's next scan walks (no `usn_roots`);
  - Scale: flat heaps, SQLite heap at most 2 × cache + 16 MiB + index/32768, temp at most 2.2 × the largest index, no WAL.

**2.3 Post-freeze migration machinery:**

- **Files:**
  - `schema.rs` (`Migration`, `bring_to`, `expected_for`, `version_of`)
  - `rebuild.rs` (shared copier, verifier, estimator)
  - `crates/schema/src/{shadow.rs, lib.rs (difference_allowing)}`
  - `catalog/src/upgrade.rs`
- **RebuildTable** (exclusive; runs in prepare, behind the startup window):
  - The first transaction sets `user_version = (1<<30) | (V+1)`, so older builds take the refusal branch (`lib.rs:597-603`). It also saves and drops T's indexes, triggers and dependent views, and creates `T__new` plus the progress tables.
  - Each batch copies into `T__new`, then runs `DELETE FROM T WHERE key <= ?hi`. Only the table B-tree is touched, and freed pages are reused.
  - Then: drop the empty T; a `legacy_alter_table=ON` rename; new indexes from exact text; the FK merge; recreate views and triggers; clear the flag.
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
  - A RebuildTable(files), with views and triggers, equals the 12-step oracle at batch sizes 1, 7 and 100k;
  - `fail_after` at every commit;
  - an older-build fixture refuses, and the file is byte-unchanged;
  - a migrated file's `mismatch()` is None;
  - no secondary index exists during batches;
  - Shadow equals `SELECT key, id ... ORDER BY` under concurrent writes and kills at every phase;
  - `set_rating` stays within 150 ms throughout.

**2.4 Writers adapted to the new columns:**

- **Files:**
  - `crates/catalog/src/lib.rs`, `pair.rs`
  - fixtures (`raw.rs`, `state.rs`)
  - `tools/shots/fixture.py:101`
- **Change:**
  - **AnalysisWriter:**
    - the store transaction runs in this order: `delete_indexed`; then the analysis REPLACE and the pending → 0 update; then the insert of the new `pdq_indexed` row (new seq, batch = meta.pdq_batch). So the store's pending → 0 queues no requeue, and a test asserts that a store queues no `pdq_requeue` row;
    - a pdq seq is assigned when quality is at least 50 and a dihedral exists, with `pdq_indexed.batch = meta.pdq_batch`;
    - `borders_due` goes to files;
    - `store_failure` clears `borders_due` and the indexed row;
    - `store_pdq_pair` is removed (5.7).
  - **BordersWriter.cut** and **crop_source** move to files.
  - **ScanWriter:**
    - no held statements; `file_seen` stamps;
    - `folder()` upserts the anchor and inserts `folders.anchor`;
    - the insert, its DO UPDATE (which changes `folder_id` while keeping `unpaired`, `lib.rs:1223-1227`) and displace (`lib.rs:1219`) each set `unpaired_anchor` in the same row write. Each statement has its own form, because SET expressions read the row's values from before the update:
      - the INSERT uses `(SELECT anchor FROM folders WHERE id = ?4)`, since unpaired is the literal 1;
      - displace always uses `(SELECT anchor FROM folders WHERE id = files.folder_id)`, since it sets unpaired = 1;
      - DO UPDATE uses `CASE WHEN files.unpaired = 1 THEN (SELECT anchor FROM folders WHERE id = excluded.folder_id) END`.

    An 8.2 ledger check refuses any statement that derives `unpaired_anchor` from a column the same statement assigns.
  - **pair.rs rebind and replace** stamp `file_seen` and keep `unpaired_anchor` in step. The clear sets `unpaired = 0, unpaired_anchor = NULL`.
  - **mark_pending** sets `pending=1` only (the D8 trigger stamps).
  - Fixture shims.
- **Tests:**
  - the existing suites;
  - `each_user_visible_change_names_its_file`, extended and schema-derived: every view-visible column has a stamping trigger;
  - the dups trigger proptests;
  - the rebind seq bump;
  - an onset sets `pend_batch` to the session clock; a store and a revert both clear it; a revert queues one requeue and a store none; a file re-imported in session k pairs as the older endpoint with a file imported in session k+5 (the oracle is today's `pdq_hashes` load);
  - invariant proptest: after any writer sequence, `unpaired_anchor = CASE unpaired WHEN 1 THEN folders.anchor END` for every row, including moves across drive letters.

**2.5 Upgrade pass: version recomputes off prepare, exact reads:**

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
    - **`pend_batch` is left to `files_pending_on` and `files_pending_off`,** which `counts.bulk` does not suppress (D8). Their write lands on the row's already-dirty page, and no index holds `pend_batch`, so it costs no extra page.
      - The 3.8 fan-out pin and the §8 version-bump fan-out assertion both count the nested pend_batch UPDATE.
      - A lint requires every AFTER UPDATE trigger on files to name its columns (OF ...), so none fires on pend_batch.
      - Pass A CPU is stated with the nested statement included.
    - **There is no tier-1 gate and no backlog drain.** By D25, a backlog file x (session < bump) pairs with every y, whatever y's stamp, exactly as today's inline pairing did. Imports after the bump are gated.
  - **Borders Pass A.**
    - Missing rows, and rows with pending≠0, get `borders_due = coalesce(has_borders, 0)` directly.
    - Present rows with pending=0 go to 2/0.
    - Files without an analysis are untouched.
  - **Taken step:** batches over `analysis_taken_utc`. The zone fingerprint is re-read each batch. The step also marks `sh_taken` (7.17) not ready.
  - **Count steps:** per-family recounts (2.6).
  - **Shadow steps** (2.3, 7.17).
  - **Gates (D32):** `Catalog::gate` returns `UpgradePending{steps}` from import_work (import), borders_work (import, borders), crop list creation and crop_source (import, borders), and the groups maintainer and builder (import). Scans, removals, thumbs, writes and tier-1 are not gated.
  - **Exact reads.** `pending_now!(f)` = `CASE WHEN f.id < ?c THEN f.pending <> 0 ELSE <stale_of!(f)> END`, and `taken_now!(a)` likewise. Every reader of these values uses them, and a grep guard enforces it. Two statement variants are plan-guarded.
  - **CLI:** `with_upgrade(|| cmd)` drives exactly the steps a command needs (import, borders find, status). Tier-1 is not gated (D25). `gallery write` labels its trailing count provisional.
- **Deviations:**
  - Scans, removals, thumbs and writes may run before the steps finish (ordering only).
  - The status line shows upgrade progress instead of counts.
  - A process time-zone change during a pending taken step restarts it. This is impossible on Windows, where the CRT fixes the zone at first use.
  - D6's "import-version bump bumps seq" is replaced by `thumb_epoch` plus the gate.
- **Tests:**
  - The oracle (today's three statements) at batch sizes 1, 7 and 100k, with stops and re-bumps;
  - keys stay mismatch-free at every stop;
  - two processes take the lease in turn;
  - prepare is O(1) on 1M rows;
  - gates fire inside the API;
  - exact-read proptest: Status filters, the taken filter and sort, and `ids(build_run)` equal an instant full recompute at every cursor position under concurrent writes;
  - pairing during Pass A equals the oracle under D25; `pend_batch` after Pass A equals its definition.

**2.6 Counters and status:**

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
  - `status()` reads one row. Present pairs = Σ(band + heavy_band), folded plus staged ≤ vis. While a removal is pending, the 3.4 correction applies.
  - Totals: borders total = `counts.borders_due`; import total = `counts.pending + counts.failed_retryable`. Both are read with S0 in the cursor's first transaction.
  - A post-freeze definition change is an Upgrade recount step: zero the family, set its cursor to 0, add each batch's true contribution and advance the cursor in the same transaction, then set the sentinel.
  - There is no prepare-time recount and no `COUNTS_KEY`.
- **Tests:**
  - A proptest against `old_status` (`cfg(test)`) after every step: upserts, REPLACE, cascades, toggles, and file_writes producers and consumers;
  - pin tests for upsert and REPLACE trigger semantics;
  - recount one family while others write: all families equal their oracles;
  - VM_STEP for `status()` is O(1);
  - golden `gallery status` output.

**2.7 Startup off the GUI thread; opens log:**

- **Files:**
  - `desktop/src/{lib.rs, app.rs, preparing.rs}`
  - `desktop/qml/Preparing.qml`
  - `cpp/gpu_device.cpp`
  - `app/src/app.rs` (`open_with(config, PrepareHooks{progress, stop})`)
  - `cli/src/lib.rs`
  - `catalog/src/lib.rs` (`prepare` appends `catalog_opens(meta.seq, 63-bit nonce)` before any migration, when the table exists; `ack_opens` prunes, keeping at most 65,536 rows)
- **Change:**
  - `run()` starts the GPU device and `gui_app`, then spawns the open worker: prepare, ThumbCache open, Ui pool, Reader.
  - Within 300 ms it proceeds as today.
  - Otherwise it joins the GPU thread, loads `Preparing.qml` into a separate engine, and shows it with `show_windows(prep, gpu)`. Progress is queued through `qt_thread()`.
  - Close or Quit sets the 1.2 interrupt; the worker returns `Cancelled`, and the next start resumes.
  - On success it drops `prep` and loads Main as today.
  - The CLI prints stderr progress lines, and Ctrl+C interrupts.
- **Tests:**
  - A stub prepare that is slow shows the window after 300 ms, rendered with the app's device;
  - cancel exits within 100 ms;
  - exactly one `show_windows` over Main's roots;
  - a fatal error shows today's message;
  - `tst_preparing.qml`;
  - `prepare_records_its_open_before_any_migration`.

### Phase 3: Scanner and pairing

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

**3.1 Walk:**

- **Inline pass.** Up to 16,384 kept entries in the reused Vec, exactly as today.
- **Validate pass**, when a folder overflows: O(1) state (no-id flag, I/O result, count). A failure leaves the folder unreadable, as today, with nothing recorded.
- **Relist pass.**
  - It streams each entry through today's per-entry function, in listing order, through WriteBatch.
  - `frames.mark()` is taken before the first entry.
  - On failure: `truncate_to(mark)`, then `scan_skipped`, `clean=false`, no descent. The rows already recorded stay.
- **Frames** go to a codec SpillStack beyond 16K. Paths are rebuilt from the in-RAM ancestor chain.
- **Out of space** (scratch ENOSPC or cap, or SQLITE_FULL):
  1. drop the SpillStack, deleting its files;
  2. roll back the open batch;
  3. return `Error::OutOfSpace{Scratch|Catalog}`.

  The result is the state a crash would leave.
- **Stop and pause** inside a listing: `Listing::each` returns ControlFlow, and `checkpoint_with(batch.commit)` runs every 1,024 entries.
- **Also:**
  - the `claim().seen_this_scan()` rule; volumes collected into `Walked`;
  - **`visit_folder(lister, frame, mode Walk|Dirty, sink) -> FolderTally`**, extracted with today's per-entry decisions unchanged. It emits `SinkOp::{Record, SeenIdentity, Skip, Descend, Listed}`. FolderTally is exactly the walk's ScanStats increments for that folder;
  - **Sinks:** `DirectSink` (ScanWriter through WriteBatch) and `SortedSink` (ExtSort by walk_key, used only for journal intents outside sub-walks);
  - **walk_key (order.rs):** a folder is `wk(parent) ++ 0x01 ++ invert(collation_key(name))`; a file is `wk(D) ++ 0x00 ++ collation_key(name)`. Lexicographic order equals the LIFO walk's order, and `subtree_end(wk)` bounds a subtree;
  - **Collation check:** every adjacent pair of a listing must collate Less. The first violation sets `usn_volumes.collation_ok = 0`;
  - **Mirror upkeep** on capable roots (Windows, local NTFS): subfolder diffs against `usn_dirs` (paged, differences only), tallies, subtree deletes for skipped or vanished folders, `usn_entries` kinds 1, 2 and 3, and `usn_stale(R)` upkeep (one `open_by_id` per kind-1 identity or stale member). `usn_wake(R)` is cleared at finish;
  - **Base cursor, crash-safe:**
    - **begin_scan:** a walk deletes `usn_roots(R)` in its own transaction;
    - **before listing:** query the journal; fingerprint the last record below E0; take the access projection and `rules_hash`;
    - **at finish:** re-query the journal. Store `usn_roots` in finish_scan's transaction only if the journal id and LowestValidUsn are unchanged and FirstUsn ≤ fp_usn. Otherwise store nothing, set `last_reason`, and show `JournalBaseWrapped{needed ≈ (NextUsn_end − E0) ÷ walk hours × rescan interval × 1.5, rounded up to 1 GiB}`;
  - seen_identity stamps only rows of the scanned root.
- **Deviations:**
  - (a) A relist failure keeps the rows recorded before it.
  - (b) A stop during a relist leaves part of the folder recorded.
  - (c) Out of space is typed.

**3.2 Pairing** (with the D9 scope, the continuation, and the USN cursor rule)

- Fence `hw`.
- **Read the root's scope once.** The root's (anchor, key range) of `files_unpaired_fresh` is read into an id-sorted ExtSort spill (16 B per row). Rows whose folder fails `folder_under!` are dropped (FolderCache), so the set equals today's `folder_id IN under_root!`.
- **Tiers.** The 4 tiers iterate the spill in visited-bounded pages. They are read optimistically, and re-done in a WriteTx only when a candidate exists.
- **`pair(root, scan, walked, floor, pace)`:** every tier's `< ?scan` on `file_seen` becomes `< ?floor` (D38). A walk passes floor = scan, so its statements are unchanged.
- **`Walked.volumes`** for a journal pass:
  - {v0} if census files − duplicates > 0;
  - otherwise, only when some fresh row has volume ≠ v0, {v0} if an identity probe finds a row for some kind-2 entry;
  - else ∅.
- Exact tier skips.
- `pair_ambiguous`.
- The outbox is written inside replace.
- The clear (`unpaired = 0, unpaired_anchor = NULL`) and the `unpaired_with_data` count (fresh plus kept, over the (anchor, range)) are paged. Q-S2 decides how that count is kept.
- **A stop is honoured between pages.**
  - `scans.(pair_tier, pair_cursor)` persists the position, and `finished` stays NULL.
  - The next Scans round resumes that scan's pairing first. It rebuilds the spill from the scope, skips ids ≤ cursor in the current tier, then runs mark_missing, then finishes.
  - The result equals uninterrupted pairing, for the same interleavings that Q6 accepts.

**3.3 mark_missing:**

- Signature: `mark_missing(root, scan, floor, scope, skip)`. Per row: `UPDATE files SET missing=1 WHERE id=?1 AND folder_id=?2 AND missing=0 AND (SELECT last_scan FROM file_seen WHERE id=?1) < ?3`, over `present_page` (`INDEXED BY files_present`, LIMIT counts visited rows) plus SkipMerge, through WriteBatch.
- **Walks:** WholeRoot + `ScanSkipped(scan)`, with ?3 = scan.
- **Journal passes:** one ordered stream merging, by (FolderPages folder key, file id):
  - `usn_drop` (in its PK order);
  - `present_page` over each `usn_scope` entry, in path order (folder-only or subtree).

  Duplicates are collapsed. SkipMerge reads `UsnBad(root)` from `usn_dirs_bad`. ?3 = floor. So the flipped set and its order equal the walk's whole-root pass, and the seq, changed and files_log stamps match.
- **Drop and scope stamps** (per row, D10):
  - `stamp_dropped`: `UPDATE file_seen SET last_scan = 0 WHERE id = ?1`, plus an insert into `usn_drop`;
  - `stamp_scope`: `UPDATE file_seen SET last_scan = 0 WHERE id = ?1 AND last_scan < ?scan AND last_scan >= ?floor`.

**3.4 Root removal** (D34)

- `remove_root` inserts `root_removals(path, key_lo, key_hi)` from `scope_keys`, and returns.
- **Membership:** a row belongs to the removal iff its order_key is in [lo, hi) (prefilter) and `folder_under!(folder_id, path)` holds (the exact `under_root!` byte test). The probe is paid only for rows inside the range. Same-named folders on other drives stay visible.
- **Readers mask at once (D34):**
  - grid build and refresh add `NOT (order_key >= lo AND (hi IS NULL OR order_key < hi) AND folder_under!(folder_id, r.path))`;
    - A drive or UNC root has no upper key bound, so `root_removals.key_hi` and the `root_range` hi are NULL. Every range text (1.5, D9, 3.4) therefore uses `(hi IS NULL OR order_key < hi)`.
    - Tests: a drive-root range test in 1.5; a 3.4 test that removes `E:\` while `D:\` stays visible.
  - an open run receives a filtered delete over the key range (O(run rows in range)), not a range delete;
  - the tree subtracts the removed folders' counts from merged nodes, and hides only nodes left empty;
  - facets use the same condition;
  - **status:** present = `counts.present` − Σ `folders.present` over folders under a pending removal's path (a `folders_path` range), and missing = (`counts.files` − `counts.present`) + that sum. Both are exact. They cost O(folders under pending removals), and apply only while a removal exists;
  - analyzed, failed, pending and pdq_pairs are shown as of the removal start, with a `RemovalInProgress` note (§7 Q-D);
  - the Duplicates filter counts twins under the removed root until they are marked (§7 Q-D).
- **run_removals** pages per row, with a forward-only done-prefix watermark (`cursor_path`, `cursor_id`), in runner slices of at most 60 s after Scans (D17).
  - Each batch adds `hidden + ?n`, where n is summed in Rust, in the same transaction as its marks.
  - Resume is O(depth), through `SkipMerge::reseed`.
  - There is no lease: the per-row recheck makes concurrent pagers exact.
  - It also pages out the root's `usn_dirs`, `usn_entries`, `usn_stale`, `usn_wake` and `usn_roots` rows.
- Completion sets `finished`. The app's row is deleted. A `report=1` row is left for the CLI, which reads `hidden`, deletes the row, and prints today's text. A runner sweep deletes report rows older than 24 h.
- CLI `status` and `dups` drive pending removals first.
- `add_root` cancels a pending removal. An `add_root` that replaces nested roots deletes their `usn_roots`.
- **Orphan cleanup:** `usn_dirs` has no FK, so the cascade misses it. Before mirror upkeep, a walk's begin_scan pages out every `usn_dirs`, `usn_entries`, `usn_stale` and `usn_wake` row under the root's path whose root_id ≠ R, one row at a time through WriteBatch. These are the orphans of replaced or removed roots. The same rule is in 3.7 (begin_scan) and in the journal pass housekeeping.
- **Tests:** add a parent root over two journaled roots, then walk; re-add a root while its removal is pending.
- Stated internal deviation: per-row seq and changed stamp order follows folder path, not folder_id. No user-visible result depends on it (§7 Q-D).

**3.5 Outbox:**

- Pages of at most 1024 rows.
- One `forget_many` cache op per page (logged by the cache triggers, 4.4).
- Temps deleted.
- Exact-id acknowledgement.

**3.6 `import --again`:**

- Target normalization.
- `mark_pending_at` per row, through the pager. Folder targets page through FolderPages (path-exact).
- **File targets** resolve through `file_by_path`, which becomes a `files_order` probe: `order_key = folder_key ++ name_key AND missing = 0 ORDER BY order_key, id`. It iterates every row with that order_key (the same key on other drives) until the `rules.same` folder check matches.
- Deviation: ties among matching rows return the lowest id.

**3.7 Housekeeping:**

- `prune_files_log()` (LOG_KEEP 1M, down to 512k, in paged deletes, advancing `log_horizon`) runs at the end of each Scans pass and each CLI scan.
- `forget_scan_scratch(scan)` always runs. It also deletes `usn_drop` and `usn_scope` pages.
- `begin_scan(root, kind, floor)` deletes the `pair_ambiguous` and `scan_skipped` residue of earlier scans of the same root, paged. It deletes `usn_roots(R)` when kind = walk.

**3.8 Missing-writer allowlist and fan-out** (D10)

- Allowlisted statement tags:
  - per-row mark_missing;
  - per-row removal;
  - ScanWriter displace (`lib.rs:1219`) and upsert DO UPDATE (`lib.rs:1223`);
  - pair.rs rebind (`:237`) and replace (`:289`);
  - fixture-only `raw.rs:68` and `state.rs:79`.
- Also listed, as file_seen and usn_wake writers rather than missing writers: the two stamp-0 statements and the `files_usn_wake_u` and `files_usn_wake_d` triggers.
- A source test matches only SET clauses: `UPDATE\s+files\s+SET[^;]*?\bmissing\s*=`.
- c = Δ`sqlite3_total_changes` per toggle, at degree 0, 1, 10 and 1000, is pinned to the derivation from the trigger text (at most 14). P (pages per row) is pinned.

**3.9 Journal rescan** (D37, D38; Q-S1 (b))

The journal only says where to look. Listings are the truth, and results replay in walk order through the same writer.

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
  - `usn_dirs(R)` in pre-order, paged 1024 at a time, with an O(depth) skip stack.
  - `first_call` on each directory. Descendants of a failing or state-2 directory are skipped, as the walk skips them.
  - Transitions become R3 intents. Listed → unreadable is a StateChange: W1 drops the mirror below the folder, and its rows are kept and excluded from missing through UsnBad, as the walk's scan_skipped does. Unreadable → listable is a SubWalk.
  - The projection is stored at W7. A stop discards the work. Cost O(dirs) (R-USN3).
- **R1 read** [cursor, E = NextUsn at R1 start]:
  - one ExtSort by ref, holding OR'd reasons, a dir flag, the last record's reasons, `closed_after_last_change` and the last USN;
  - a second, uncapped ExtSort of distinct (ref, parent) pairs;
  - roots with equal cursors share one read;
  - afterwards, a re-query: the same journal_id and FirstUsn ≤ fp_usn, else NeedWalk. FirstUsn only increases, so nothing unread was purged.
- **R2 classify** (streamed in ref order):
  - A merge-join with `files_identity`, plus `usn_dirs_ref`, `usn_entries(R)` and `usn_stale(R)` lookups.
  - **Relevance:** a ref is relevant if it has a row in R, an exception or stale entry in R, is a mirror dir of R, or has a parent that is a mirror dir. Only relevant records count toward the crossover.
  - **Links:** a ref with HARD_LINK_CHANGE, at least 2 distinct parents, or kind-1 gets `open_by_id.links()`, and every link parent in mirror(R) becomes dirty. If Denied, the union of kind-1 parents, record parents and the row folder is dirty, flagged Relist.
  - **Staleness open:** skipped only for FILE_CREATE without HARD_LINK_CHANGE and with a CLOSE after the last change. This skip is conditional: it stays only if the real-NTFS platform test (1.6) shows that such an entry equals the live values from open_by_id after close. Otherwise every affected media ref is opened. Otherwise `open_by_id` gives NumberOfLinks and live values.
  - **Output:**
    - ExtSort D of (folder wkey, ref, flags). Directory candidates contribute their old parent, their new parent, and themselves when their reasons include SECURITY_CHANGE, REPARSE_POINT_CHANGE or BASIC_INFO_CHANGE.
    - Spill A of affected rows, keyed (FolderPages key, file id, ref).
    - A sorted membership file of affected refs, with a fence index.
- **R3 dirty folders**, from D in wkey pre-order, with an O(depth) skip stack over SubWalk, DropSubtree and newly unreadable roots.
  - **Directory candidates** (resolved by `dir_path` or by the parent's relist):
    - moved within R: DropSubtree(old) + SubWalk(new);
    - moved out: DropSubtree;
    - created or moved in: SubWalk;
    - into a skip name, a reparse point, or unreadable: a StateChange;
    - a directory in D itself gets `first_call`.
  - **Relist mode** applies when:
    - entries ≤ max(4096, 64·k);
    - or an affected ref is Denied;
    - or a probe contradicts `open_by_id`;
    - or opening a probe_entry handle fails. This is treated as the walk's open error: P is relisted, and a list failure follows the 3.1 relist-failure rules;
    - or the order rule fails: more than one intent-carrying name (files, or separately child folders), and some name has a unit ≥ 0x80.

    In Relist mode, `visit_folder(P, Dirty)` emits an intent for EVERY media and reparse media entry, exactly as the walk does. Subfolders are compared with the mirror, the tally is recomputed, and P becomes a folder-only scope.
  - **Probe mode** otherwise: `probe_entry` for each affected link in P. Found → intent. Absent → drop candidate. Absent while `open_by_id` listed that link in P → switch to Relist, never absent. The tally changes by the difference from the before-state.
  - **Staleness** (both modes):
    - A ref enters `usn_stale(R)` when it has more links than R lists, or its live values differ from the listed values, or there is no CLOSE after the last change.
    - It leaves when CLOSE has been seen and all listed entries equal the live values.
    - Denied → J-a(ii).
  - **Multi-link:** for every intent whose identity has at least 2 listed links in R, every listed link gets an intent, and the kind-1 rows are rewritten.
- **R4 SubWalk plan:**
  - SubWalk roots with their wkeys;
  - `subtree_census(old path)` for in-R moves;
  - a moves map (old prefix → new prefix) for remapping kind-1 link parents.
- **R5 recurring sets:**
  - every state-1 directory gets `first_call`, as the walk retries it;
  - `usn_stale(R)` members are re-probed;
  - `usn_wake(R)` members get a claim probe (and, if they are no longer Foreign, a `probe_entry`); then the wake rows are deleted.
- **Cost rung (Q-S4):**
  - est_pass = c_e·(Σ relist entries + Σ subtree_census(moved) + state-1 entries) + c_p·probes + c_o·opens;
  - est_walk = c_e·census.entries + c_d·census.folders;
  - walk when est_pass > 0.8·est_walk. The constants are calibrated in Phase 8.
- **R6 seal**, under the scan lease: the sorted non-subtree intents, the SubWalk roots, the DropSubtree and StateChange lists, the moves map, A, the membership file and the scopes. Then `begin_scan(R, kind 1, floor = base_scan)` = J, with `usn_state = {W1, ∅, seal, E, fp}`.
- **W replay:**
  - **W1:** mirror and `usn_entries` deletions under DropSubtree old paths and under new state-1/2 directories, per row, with the census.
  - **W2 merged replay** in walk_key order. The sorted intents go to DirectSink, merged with each SubWalk run **in place** at its position: the 3.1 frame loop, `visit_folder(Walk)`, DirectSink and MirrorWriter. Sub-walk entries need no scratch.
    - **Inline rule:** after the walk's claim probe, when the hit row is in R, is not claimed by J, and (lies outside every DropSubtree of this pass, or is kind-1, or is in the membership file, or is stale), call `links()`. The winner is the listed link with minimum walk_key (remapped through the moves map). When two links of the identity sit in one folder and have a non-ASCII name, that folder is listed for ordinals, and its listing order decides. The winner upserts; the others count as duplicates and write kind-1 rows. Denied → abort to walk.
    - Otherwise a plain upsert: a move by construction, with no open.
    - **Unexplained results** (an unaffected relisted entry that is Changed, a relisted folder's unseen row not in A, or a claim hit outside every DropSubtree with no record) are traced and set `verify_due = now` (Q-S5).
    - WriteBatch commits advance `usn_state.position`.
  - **W3:** in (folder key, id) order, `stamp_dropped` each A row not claimed or stamped by J and not under UsnBad, then `stamp_scope` each scope. The sealed files are deleted.
  - **W4:** SkipSource = UsnBad(R).
  - **W5:** pair(root, J, Walked{clean = complete ∧ no state-1/2 directory, volumes exact}, floor), with the 3.2 continuation.
  - **W6:** mark_missing (the 3.3 ordered merge).
  - **W7:** finish_scan, which calls `store_cursor(E, last record below E, lowest_valid, access_proj, census)`, then forget_scan_scratch. The cursor moves only here.
  - ScanStats = replay counts + mark_missing + pairing + census. Report counts come from the census: errors = state-1 count; skipped_reparse = reparse_files + state-2; duplicates = extra_links + foreign_ids; failures = the first FAILURES_KEPT state-1 rows by wkey; loops = Σ tally.loops.
- **Stop, kill and resume:**
  - **R0-R6 or R-A:** a stop discards scratch and writes nothing, and the root goes back to the front.
  - **W1-W3:** the next Scans round adopts the sealed files by rename (D16), and resumes from `usn_state`. A SubWalk resumes from its root: it prunes children with subtree_end ≤ position, and skips entries ≤ position in the folder that holds the position. Affected refs met inside a SubWalk get the R2 staleness open. The 24 h lost-or-old-seal abandonment applies only before W3 completes, since nothing sealed is needed after W3.
  - **Lost or old seal:** if the sealed files are gone, or older than 24 h, J is abandoned (no pairing has started), and a fresh pass runs from the unchanged cursor. The result equals a stop plus a rescan (Q6).
  - **W5-W6:** resume through the 3.2 continuation and `usn_drop` / `usn_scope`.
  - **Abort to walk** (a Denied winner check or an impossible inconsistency): J ends like a stop (complete = false, nothing marked missing, cursor unchanged), and the same pass runs a full walk. This equals stop plus walk under Q6.
- **Equivalence invariant I:** after every pass of root R, for every row r of R not under a state-1/2 directory, last_scan(r) ≥ floor_R ⇔ the walk of that tree saw r. Rows, missing flags, mirror, `usn_entries` and census equal the walk's. The argument:
  - untouched entries yield Same;
  - the merged replay is the walk's non-Same subsequence, in order;
  - drops plus scopes are the walk's unseen set, flipped in its order;
  - pairing gets the same inputs;
  - the report is Σ tallies.
- **Runner, CLI and app:**
  - **Scans pass:** `scan()` tries the journal pass. On NeedWalk it walks in the same pass, and records the reason in `usn_roots.last_reason`, in a trace event, and in `gallery scan --explain`.
  - **Verification walks:** when `verify_due ≤ now`, the root walks (Q-S5, `journal.verify_days` = 30).
  - A pending J resumes first, and queued roots with equal cursors share R1.
  - **CLI:**
    - `gallery scan --walk`;
    - `gallery journal status`: cursor, size, FirstUsn age, days of history, last reason, verify due, self-test;
    - `gallery journal size <volume> <bytes>`: elevated, keeps the ID. 4 GiB is recommended at 1B (Q-S6);
    - the notices print the fsutil equivalent and the estimate.
  - **Journal sizing with consent (Q-S6, decided):**
    - **Sizing estimate:** each journal pass records the volume's record rate. target = rate × max(`journal.verify_days`, `journal.retention_days`, default 30) × 1.5, clamped to [32 MiB, 8 GiB], plus an AllocationDelta of target/8.
    - **When to offer:** when the current MaximumSize holds less than the retention days, or when R-USN2 has just forced a walk. The app then offers, once per volume per size bucket: "Enlarge the change journal on E: to N GiB (keeps its history)".
    - **On accept:** the per-session helper (`pg-volume.exe`; silent elevation here, a UAC prompt elsewhere) runs a new closed request, `ResizeJournal { volume, max_size, delta }`. It calls `FSCTL_CREATE_USN_JOURNAL` and keeps the journal ID.
    - **On decline:** the choice is stored in `gallery.json`, and is not asked again until the estimate doubles.
    - **Non-admins:** never resized; the notice explains why.
    - The CLI `gallery journal size` stays as the manual path. Nothing ever resizes silently.
    - **Tests:** the FakeVolume estimate; the offer, decline and re-offer rules; the helper request validation (local NTFS volume only; bounds checked); the journal ID kept after a resize on the test VHD.
    - `docs/plans/NTFS_MFT_SCAN_PLAN.md` gets the `ResizeJournal` request.
  - **App:** Rescan uses the journal; a 'Full rescan' action forces the walk.
  - **Watch** (app only, Q-S7):
    - **Handles:** one thread holds an overlapped `wait_read` per capable volume, registered with VolumeGuard.
    - **Relevance:** it checks relevance through a `Role::Watch` read-only connection and a 16 MiB LRU. A record is relevant when its parent or ref is a mirror dir of a root on that volume, or a `files_identity` hit there.
    - **Own writes ignored:** records for the catalog, -wal, -shm and journal refs, and records under `<catalog>.scratch` or the thumbnail cache, are ignored. Those files are recreated, so their refs are read again at each open and checkpoint.
    - **No waiting read:** if waiting reads are not supported, there is no watch, and Rescan still works.
    - **Queueing:** a relevant record queues a Scans request, debounced 5 s. The watch never writes.
    - **Shutdown and locks:** at shutdown, CancelIoEx, then join. A volume lock or removal closes the handles. A pass's handles are registered too: a lock raises a stop (R discards; W keeps its prefix).
- **Testkit:** `crates/testkit/src/ntfs.rs` FakeVolume implements Listing, Journal, and the token and bus probes on every OS. It models:
  - refs with sequence numbers and reuse;
  - multi-link stale directory entries, refreshed silently by opens through another link;
  - open sessions with lazy index updates;
  - POSIX delete; reparse points; ACL denial (including OpenFileById denied while listing works);
  - token changes; case-only and POSIX-wildcard names; NTFS collation, with an optional divergent non-ASCII `$UpCase`;
  - a bounded journal with wrap, purge during a read, ID change, rollback, and LowestValidUsn jumps;
  - an unprivileged-read filter toggle; a bypass mode; transient open and list errors; volume locks.

  A **parent-ambiguity mode** randomises the record parent for HARD_LINK_CHANGE, FILE_DELETE and RENAME_OLD_NAME. The `walk_with` and `journal_with` fixtures live in `scan/src/lib.rs`.
- **Memory:** at most about 140 MiB from the background share, constant in n:
  - read buffer 1 MiB;
  - R1 groups at most 40 MiB, shared with the pairs sort;
  - D at most 24 MiB;
  - intents at most 24 MiB;
  - merges 8 MiB;
  - mirror LRU 16 MiB;
  - in-place SubWalks: the 3.1 bounds;
  - skip stack at most 256 KiB; fence index at most 4 MiB; link buffer 1 MiB; probe buffer 64 KiB.
- **Disk:** scratch about 60 B per record, 150 B per D pair or non-subtree intent, 40 B per A row or scope, and 0 per sub-walk entry. Catalog: `usn_dirs` about 280 B per directory.
- **Time** (1B, NVMe warm; measured in Phase 8): see §2. A full walk takes 1-4 h.
- **Deviations** J-a to J-f are in §7 Q-D.
- **Tests:**
  - All walk tests at the default budget, and at `Budget{1,1}` with `Limits{rows:1, age:0}`.
  - Validate and relist failures.
  - 40,000 subfolders pop in reverse listing order.
  - Out of space leaves scratch empty, and makes zero commits after the failure.
  - A stop in the validate pass writes nothing; a stop in the relist gives `3 × BATCH` rows.
  - A pause lets a second connection's BEGIN IMMEDIATE succeed within 100 ms.
  - Two-root pairing visits only its root's (anchor, range). Two drives with mirrored layouts pair and remove independently; a drive root reads only its anchor.
  - A stop between pairing pages, then resume, equals the uninterrupted result.
  - Removal:
    - a 25,000-row single folder pages at most PASS_BATCH visited rows per pass;
    - kill and resume;
    - concurrent pagers give `hidden` = 50;
    - the reseed with a nested root;
    - the mask hides immediately in grid, tree and status, and leaves `D:\Photos` visible while `E:\Photos` is removed;
    - status present and missing are exact during the removal.
  - Outbox id reuse.
  - The authorizer test; the fan-out pin.
  - The differential proptest against `reference_pair`, `reference_mark_missing` and the reference walk, at page sizes 1, 2 and 1000, with floor = scan.
  - An interrupted scan leaves no residue after the next scan of that root.
  - `file_by_path` with equal order keys on two drives.
  - **Journal walk equivalence:**
    - the DirectSink sequence equals itself sorted by walk_key;
    - FolderTally sums equal ScanStats;
    - the mirror equals an oracle rebuild;
    - a kill or stop mid-walk leaves no `usn_roots`;
    - a wrapped base gives no cursor, and the notice;
    - drops in /b (low ids) and /a (high ids), plus a scope, give stamps equal to WholeRoot;
    - census = Σ tallies;
    - UsnBad equals `scan_skipped`;
    - the wake trigger inserts exactly the kind-3 roots;
    - remove_root leaves no usn residue.
  - **Runner and CLI:**
    - runner: NeedWalk → walk; resume; verify_due → walk; a shared read;
    - CLI golden output: `scan` and `scan --walk` give identical lines, except elapsed time;
    - watch: one queued scan per relevant create; nothing for WAL, scratch, thumbnail or outside-root churn; FSCTL_LOCK_VOLUME and safe removal succeed on a VHD.
  - **Journal oracle** (`scan/tests/usn_oracle.rs`): A and B are byte copies after a base walk. A script of operations runs, and at each quiescent checkpoint A runs its schedule while B runs `--walk`.
    - **Compared:** canonical dumps of files (every column, including id, pending, seq, changed, unpaired, unpaired_anchor, missing and order_key), folders (id, path, key, anchor, present), the file_seen seen bit, scan_cleanup, counts, files_log order, the usn tables, ScanStats and scans.
    - **Schedules:** all-J; J → walk → J on the new floor; J stopped in W5 with the continuation resumed; J abandoned and redone; R-A after a token change.
    - **Seeds:** at least 10K FakeVolume seeds (parent ambiguity in half, Budget{1,1} and defaults), 200 real-NTFS seeds, and `just usn-vhd` (second-user and elevated mutators).
    - **Operation set:**
      - moves, renames and case-only renames; delete and recreate; save-over;
      - hard links within, across and outside roots, including a new link earlier in walk order;
      - junction swaps; deny and allow; a rewrite with mtime restored; extension changes;
      - create and delete in one pass; a 20K-entry folder with k = 1, 3 and 50 probes;
      - `<`, `>` and `"` names, and non-ASCII probe-mode names;
      - a renumbered volume;
      - files held open with lazy updates; POSIX deletes of open files;
      - ACL changes on mirror directories and the root.
    - **Deviation tests:** each of J-a to J-f differs in exactly the stated rows.
  - **Journal faults** (`scan/tests/usn_faults.rs`):
    - (i) failing journal calls (the k-th call of each kind; FirstUsn past the cursor while the read succeeds; a purge during R1);
    - (ii) a child abort at each W commit, including inside SubWalks;
    - (iii) random stops in R, R-A and W;
    - (iv) ENOSPC and SQLITE_FULL give typed OutOfSpace;
    - (v) VHD journal delete, create, wrap (including during the base walk), rollback and clone;
    - (vi) a projection change runs R-A; SeChangeNotify off walks;
    - (vii) a concurrent mutator, then one more pass equals the walk;
    - (viii) a collation violation walks; a divergent `$UpCase` relists;
    - (ix) a Filtered self-test walks;
    - (x) a volume lock during a pass stops it, and it resumes later.

    Every case gives an identical catalog, or a fallback walk with an identical catalog, never a mixed one.

### Phase 4: Work lists, write-back, thumbnails, borders CLI

**4.1 `import_work` and `borders_work` cursors** (D5, D6, D31)

- **Files:**
  - `catalog/src/work.rs`
  - `lib.rs` (`pending_import`, `pending_borders` and `work_list` are removed in 5.1)
  - `library/src/borders.rs`
- **Change:**
  - Two keyset streams are merged in (volume, record, id) order from frontier F:
    - the partial index (`INDEXED BY`);
    - while `bulk_walk_<kind> ≠ 'done'`, a walk of `files_present_order` from max(F, P) that returns flag-2 rows, examining at most 4096 entries per fetch.
  - Rows with `seq > S0` are skipped, and lower the total.
  - P is persisted in each writer's store transaction, and advances only past consumed keys.
  - **Retry list:** materialized into TEMP on the pass connection, in 65,536-row keyset chunks over the failures index, with the `seq ≤ S0` filter per chunk (D31). S0 and the `counts` totals are read in the first chunk's transaction, through ATTACH.
  - `import_work` reports whether its gate passed and its list is non-empty, so 5.1 can bump the session clock (D25).
- **Tests:**
  - Oracle drains equal the old SQL constants at PAGE 1, 2, 7 and 1024, including bulk flags and preemption at every fetch;
  - total examined ≤ n + preemptions × 4096;
  - `done == total` at Exhausted;
  - a mutation yields each id at most once.

**4.2 Borders pass** on the cursor. The final flush runs on every `Ended`, then `end_work`.

**4.3 Write-back queue:**

- **Files:**
  - `catalog/src/writes.rs`
  - `writeback/src/lib.rs`
  - `platform/src/volume.rs`
- **Change:**
  - **next_ready:** at most 10 probes. For t in (0, 1), then a in 0..=4: `... INDEXED BY file_writes_ready WHERE attempts < 5 AND parked_folder IS NULL AND (temp IS NULL)=?t AND attempts=?a AND queued <= ?now - ?settle*(?a+1) ORDER BY queued, rowid LIMIT 1`. Take the minimum (queued, rowid) over the first t that has a hit.
  - **attemptable** is O(1).
  - **recent_errors:** latest first, on `file_writes_failed`. This is a bug fix: the doc says "latest".
  - **Hierarchical parking:**
    - an offline failure parks the job on its (mount root, folder);
    - a Prober thread checks at most 16 roots and 16 folders per tick, with PROBE_WAIT 5 s, and at most one probe outstanding per root;
    - a present folder unparks its oldest job as a probe;
    - producers' upserts and the files trigger unpark.
  - **W4:**
    - sniff from a 4 KiB prefix;
    - hash with `file_hash_of` (streaming);
    - `copy_into` through `io::copy`, with a 1 MiB buffer;
    - verify through a read-only mapping, dropped before `remove_file`.
- **Deviations:** online jobs no longer wait behind offline ones. A share slower than PROBE_WAIT is parked, not counted as an attempt. recent_errors shows the latest errors.
- **Tests:**
  - the order oracle against today's NEXT_READY;
  - VM_STEP with 10^6 rows all settling;
  - 10^5 parked folders on an absent root cost one probe per 30 s;
  - a hung probe does not stall the drain;
  - W4 gives identical bytes and digest, and peak RAM stays below 64 MiB on a 1 GB file.

**4.4 Thumbnail cache layout: the one edit, then start over** (D35)

- **Files:** `thumbcache/src/{lib.rs, layout.rs, key.rs}`
- **Change:**
  - **Removed:** `checked` and `entries_lru`.
  - **Added:**
    - `clock`, and `heights` (gen, sync cursor, tomb, reval, walk cursor w as an order-preserving 14 B BLOB, lap, counters, opens id and nonce, compact, owner, beat, used, healed, epoch);
    - `todo` (exact and range members, laps), with `todo_file`;
    - `lost`, with `lost_order`;
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
  - CACHE_VERSION stays 1. `judge` starts existing caches over once (keeping the cap), and publishes `StartedOver`. Later cache changes use 2.3 Shadow.
- **Tests:**
  - each entry write logs exactly its rows, including outer OR IGNORE and REPLACE;
  - counters equal count(*) per (height, gen);
  - retired-gen deletes leave live counters unchanged;
  - key order matches SQLite;
  - a cache with `checked` starts over.

**4.5 Cache writer ops** (D35)

- **Change:**
  - **Ops:**
    - Claim (IfFree or Steal; owner from `clock`; Drop beyond HEIGHTS_KEPT);
    - heartbeat, through the 1.9 helper;
    - Release;
    - Reset and Drop by gen (O(1));
    - Sync, Put plus Settle (atomic), Compact, Opens;
    - Gc of retired gens, in 4 MiB WAL slices;
    - StateRecount.
  - **evict:**
    - a K-way merge over per-height keyset cursors on `entries_height_lru`, heights other than keep, in the total order (last_used, height, bytes, id);
    - collect at most 1024 victims, reset the statements, delete, and re-seek;
    - **short-circuit** on the main path only: if Σ `height_totals.bytes` over heights other than keep is less than the excess, return false without walking. The slice loop is unchanged.
  - **Heal:**
    - `open` only queues `HealIfDue`;
    - the writer runs 65,536-id chunks (5-10 ms each) between ops, accumulating in `heal_acc` under the gated triggers;
    - the final step fixes totals and `height_totals`, then publishes `TotalsFixed`;
    - a second process sees `healed` advanced, and skips.
  - **Touches** beyond 65,536 ids spill to `temp.touch`.
  - **forget_many:** one op per at most 1024 ids.
  - **Inbox:** a byte cap of 32 MiB on `PutOne` payloads drops the oldest PutOne (best effort; regenerated on demand).
- **Deviations:**
  - One pass per height at a time: the CLI steals.
  - `TotalsFixed` may follow open.
  - A PutOne dropped under a sustained writer stall is regenerated later.
- **Tests:**
  - The lease tests.
  - Reset and Drop are constant-time, and GC never touches live rows.
  - The evict oracle (today's ORDER BY), with keep_height dominating; available = excess − 1, excess, or excess + slack.
  - Heal is exact under puts from another process, and open returns in under 10 ms on 1M entries.
  - Touch spill while another process holds the lock.
  - The inbox cap.

**4.6 ThumbPass** (ordered batches, cache-full memo)

- **Files:**
  - `thumbcache/src/pass.rs`, `pass/compact.rs`, `key.rs`
  - Cargo.toml (the pg-catalog dependency is dropped)
- **Change:**
  - **`pass(catalog, height, Claim) -> PassStart{Disabled, Busy{until}, Pass}`.**
  - **Open check** (one read transaction over both files). Reset when any of these holds:
    - S < c, or the catalog's opens log shows a rollback or fork;
    - **`thumb_epoch` changed** (the import-version bump, D20).
  - **Chunked live-resolution sync**, each chunk at most 4096 rows and stoppable:
    - tombstones from `thumb_removed` (revalidate if `tomb < horizon`);
    - the delta over `files_seq`;
    - the `lost` log.

    Resolution R(id): delete any exact member, then insert into lap L if ok, key ≤ w, and no lap-L range covers it. Abort rule: Reset when `sync_rows` > `walked_rows`.
  - **Budget:** B = max(2^20, N/16) members. Compaction merges the smallest gaps into ranges (two-level histogram), with hysteresis B → B/2. w is never lowered.
  - **Stream:** lap L (exact and range members), then the walk from w over `files_present_order`.
  - **SettleTracker** advances only over the settled prefix.
  - **Batches use `Order::Index` (1.8),** so a batch is the first BATCH successes in feed order, which is deterministic.
  - **Cache-full memo (Q-T4 a, decided):** before decoding, if `rejected[h] = (cap, total, own_bytes(h), digest of the next BATCH candidates' (id, seq))` matches, and the stored rejection's candidates all succeeded, stop as cache-full without decoding. §7 Q-D lists the exactness condition.
- **Theorem:** in a quiescent system, at a lap start, a round feeds exactly today's FULL `pending_sql(false)` rows, in (volume, record, id) order. Any other round feeds those rows minus the files that already failed or changed in this lap.
- **Tests:**
  - The differential proptest: identity rebind across w in both directions, id reuse, pruning, restore and advance, B tiny, steal;
  - a wake every k items still advances w;
  - an offline volume compacts to one range;
  - a cache-full round decodes 0 after the memo;
  - batch membership is stable across worker schedules;
  - plan guards.

**4.7 thumbs::run, runner and CLI:**

- **Files:**
  - `thumbs/src/lib.rs`
  - `library/src/runner.rs`
  - `cli/src/lib.rs`
  - `library/tests/{video,writeback}.rs`
  - `bench/benches/ir/catalog.rs`
  - `docs/perf/baseline.json`
- **Change:**
  - `Disabled` → complete with nothing done. `Busy` → `elsewhere`.
  - Sync, then `ack_opens`.
  - The pipeline keeps the file_state check.
  - `first_failed` is removed.
  - Every `Ended` sends the final Settle and Release.
  - `needed: Option`.
  - The runner keeps a `retry_at` and waits on it. The CLI steals.
- **Deviations:**
  - The app yields to `gallery thumbs` for at most 30 s after a release, or at most 150 s after a CLI crash.
  - At most E + BATCH duplicate decodes per steal.
  - A failure is retried once per lap.
- **Tests:**
  - The CLI takes the height (duplicate decodes at most E + BATCH);
  - the app resumes after a release or a crash;
  - Disabled completes.

**4.8 `gallery borders --csv` and `--import-review`:**

- **Files:**
  - `cli/src/{borders.rs, sorted_walk.rs}`
  - `jobs/src/pipeline.rs`
  - pg-spill
- **CSV:**
  - **Counting pre-walk:** today's `walk` and `is_image`, with `n += 1`, returning every error at today's point.
  - **SortedWalk per input:** each directory's entries are sorted by OsStr through ExtSort. Ancestor tails go to the one segment stack, written once per directory.
  - **K-way merge of inputs** by (path, input index), which equals today's stable `out.sort()`.
  - **The pipeline in `Order::Index`.**
  - **Output:**
    - rows go to `<csv>.partial`, exclusively locked;
    - on Ok: rename if no CSV exists; otherwise copy in place (truncate, write, flush), which keeps the file's identity, ACL and hard links; then delete the partial;
    - on Err before the in-place copy starts: delete the partial, and the CSV is untouched;
    - on Err during the copy: keep `<csv>.partial` and report its path. The CSV may be truncated, as with today's `fs::write` (`borders.rs:134`).
- **Review import:**
  - A serde map visitor over a BufReader;
  - ExtSort by (key, ordinal), with keep-last;
  - nothing is marked before `de.end()`;
  - marks are applied in key order through the `file_by_path` probe (3.6), inside WriteBatch.
- **Deviations:**
  - (a) A tree that changes between the two walks.
  - (b) The `.partial` file while the run is in progress.
  - (c) serde's error text for non-UTF-8 input.
  - (d) Marks are committed in batches.
- **Tests:**
  - Byte-identical CSV against the `cfg(test)` oracle: nested and overlapping inputs, non-UTF-8 names, junctions, and ExtSort budgets of 1 to 7 records;
  - a walk error leaves the CSV untouched;
  - an injected copy failure keeps the partial and reports it;
  - a stop gives the sunk rows in order;
  - hard links keep their identity after success;
  - the review oracle: last-wins; invalid JSON marks nothing;
  - a scale CLI variant.

### Phase 5: PDQ tier 1 and import

**Files:**

- `crates/dedup/src/{layout, canonical, sheet/*, join/*, peel, stop, components}.rs`
- `crates/tier1/src/{store, gc, tail, pair, classify, publish, transfer, account, partners, schedule, maintain, budget, reclassify, planes}.rs`
- `crates/catalog/src/tier1.rs` (the HeavyPartners trait, and bounded readers and writers)
- `crates/query/src/lib.rs` (`pdq_near_pair`)
- `import/src/lib.rs`, `import/Cargo.toml`
- runner
- CLI `gallery tier1 maintain|check|rebuild|status`
- crate graph: pg-catalog does not depend on pg-tier1; pg-tier1 depends on pg-catalog, pg-dedup and pg-spill.

- **5.1 Import.**
  - `import()` bumps `meta.pdq_batch` (the session clock, D25) only after `import_work`'s gate passes and the list is non-empty. This matches today's index load, which happens after the empty-list return (`import/src/lib.rs:35-40`).
  - Paged cursor (4.1), store only. The pairing worker runs on `Role::Pair`.
  - Drain is stoppable.
  - Flush on every `Ended`.
- **5.2 Layouts and canonical rules:**
  - EVEN31 for equal parity (including odd-odd), FULL31 for different parity, and R22 and R22o.
  - `query_canonical` gives at most 8 thin emissions per pair.
  - `pair_canonical` gives exactly-once emission with the exact (d, v).
- **5.3 Sheets:**
  - Thin sheets: 33 B records, cap 2^29.
  - Fat sheets: about 261 B records, cap 2^26.
  - CRC32C per 4 KiB block on every read; on-disk prefix tables.
  - Fat sidecars: a position table, plus planes P, G and B, with journaled updates. Plane P mirrors `files.pend_batch`, fed through `pdq_heavy_attr` by `files_pending_on` and `files_pending_off`. For a heavy file, `off` queues `pdq_heavy_attr` together with its NULL write.
    - `pdq_heavy_removed` copies the plane-P value that the published pairs were formed under. Once attr updates are applied in order, that equals `files.pend_batch`.
    - Test: revert a heavy file, then run a newer import before the requeue runs. The gone accounting equals the oracle.
- **5.4 Join engine:**
  - sparse or sweep mode, chosen by measured q*;
  - co-iteration that skips promoted thin entries;
  - thin reducer capped at 4,104 per older file, with exact recovery;
  - fat and in-batch exactly-once emission;
  - sinks; unit mode; peel.

  Budget W split: B_batch W/2, probe W/8, match W/8, readers W/8, sketch 16 MiB. W is the one process-wide reservation (§2).
- **5.5 Calibration:** IOPS and sequential bandwidth are measured at store open, into tier1 meta. q* is per store.
- **5.6 Merges and debt:**
  - 4-way tiered, ratio-4 merges, with a liveness pass and checkpoints.
  - Bulk build: one extraction, then one sheet at a time. Sorts use only the background share (D14).
  - **Run-count cap** = the log4 ladder + 8 capped runs. When live runs exceed it, the pair-lease holder (the CLI included) runs bounded, resumable merge units before its next batch.
- **5.7 Pair worker protocol** (D25):
  - P0: select seq > `pdq_paired_hi`. **No pending skip on queries:** today pairs a file at store whatever a later onset does. P0 does not write `meta.pdq_batch`; it is the import-session clock (D25).
  - P1: sweep.
  - P2: classify, with light degree = row count (two range scans with LIMIT 513) plus new partners.
  - P3: rows for pairs with at least 1 light endpoint, skipping non-current seqs and those failing `formed` (older.pend_batch IS NOT NULL AND older.pend_batch < newer.batch).
  - P4: heavy run 'staged', `pdq_heavy` rows.
  - P5: stage.
  - P6: publish (CAS `pdq_paired_hi`, `heavy_vis`).
  - P7: transfers in counted terms.
  - A fold applies the stages.
  - Every phase is cursor-idempotent (`meta.pdq_write`).
- **5.8 account_heavy:**
  - Gone units run first, then flip units.
  - All accounting is in counted terms.
  - Publish re-reads presence inside IMMEDIATE.
  - Gone rows are acknowledged twice (`acct`, `groups`).
  - Reports show "as of gen G, N pending" while anything is unsettled. `gallery dups` waits, stoppably; `gallery status` prints the note unless `--wait` is given.
- **5.9 Readers and retention:**
  - The pure `pdq_near_pair`.
  - HeavyPartners in pg-tier1, injected by runner and CLI.
  - A `read` lease records `heavy_vis` before the reader's snapshot. Runs are opened at step start (Windows: no FILE_SHARE_DELETE).
  - GC deletes a retired run only when no lease is older than its `retired_gen`.
- **5.10 Scheduling and lag:**
  - A batch starts at backlog ≥ B_batch, at import end, or at backlog age ≥ L_lag = clamp(5 × t_sweep, 30 s, 60 min). Sweep duty is at most 20%.
  - `tier1.max_pair_lag` is set in gallery.json.
  - A `reclassify` job runs for a lowered HEAVY_ENTER.
  - `tier1 status` reports runs, heavy files, unsettled count, odd fraction, q* and IOPS.
- **Deviations:**
  - Missing files are ignored when pairs form (Q3). This adds raw `pdq_pairs` rows that never exist today. They become visible through `present_pdq_pairs` when a missing file returns.
  - A pending revert re-indexes the file (D25).
  - Two imports in two processes no longer miss each other's pairs.
  - Legacy catalogs are re-paired.
  - Reports carry an as-of note while unsettled.
- **Tests:**
  - The brute-force oracle (a sample, plus planted copies at 0 to 31 flips, odd weights) at every layer, under budgets of 4 KiB, 64 KiB and unlimited, with HEAVY_ENTER forced to 2 and to 8;
  - crash points between every transaction;
  - a 100k planted clique: temp is O(k);
  - the flip race between two connections;
  - promotion while a partner's flip is pending;
  - two gone partners;
  - **Session-clock oracle:** an onset during and before the session; a re-store mid-session; a file re-stored after an old onset pairs as the older endpoint in a later session; a never-pending file (NULL) pairs in SQL and in Rust; an empty or gated import does not bump `pdq_batch`;
  - retention: a reader paused across merge plus GC;
  - the run-count cap holds after a long import;
  - crate-graph test: no cycle;
  - dhat at most W + 2 MiB; W is never held twice (the pair worker, groups peel and merge contend for it);
  - oracle comparison: (a) raw `pdq_pairs` filtered to the old predicate, (b) `present_pdq_pairs`, with missing-file fixtures.

### Phase 6: Duplicate groups and the Duplicates filter

**Files:**

- `crates/dedup/src/{components, build}.rs`
- `crates/catalog/{Cargo.toml (+pg-dedup), src/groups/{mod, graph, store, scratch, build, counts, maintain, report, oracle}.rs, src/dups.rs}`
- `crates/query/src/lib.rs` (`duplicates!`)
- `library/src/dups.rs`
- `cli/src/dups.rs`

- **6.1** Edge-granular balanced checks, plus a residual per group, capped. GraphReader adds HeavyPartners per frontier batch.
- **6.2 Build.** A disk-DSU build from bitmaps, with sorted appends. Heavy adjacency is handled by a seq-ordered RAM-batch peel, plus one sweep per batch with plane B. The peel takes the process-wide W reservation (§2).
- **6.3 Maintainer.** A single maintainer, under the `groups` lease (with lease-named scratch, opened through `connect` as `Role::GroupsScratch`), with watermark `groups_gen`.
  - Op-5 heavy adds are batched against plane G.
  - Heavy removals take seeds from the gone dihedral.
- **6.4 Report.** Counters (in `counts`, D22) make the report O(show). The report takes the `groups` lease while paging (D31).
- **6.5 Duplicates filter:**
  - The hash part: `hash_runs.present − self ≥ 1`.
  - The PDQ light part: `pdq_near.n ≥ 1`.
  - The heavy part: `heavy_near!()`, which is n = near + stage, plus the unsettled correction.
  - Builders wait while more than 4,096 changes are unsettled.
  - **Stamps:** a run crossing present 1↔2 stamps `files.changed` on both affected members, found in O(1) through `present_xor`. `pdq_near` crossing 0↔1 stamps its file. A heavy publish crossing `near` 0↔≥1 stamps that file. So Duplicates views take deltas, and the grid's "always rebuild" exception is removed.
  - **Deviations:**
    - A Duplicates-filtered run reflects a heavy file's presence change when the settle unit publishes, not at the instant of the flip (the read-time correction applies at build time).
    - While a removal is pending, twins under the removed root still count until they are marked (D34).
- **6.6 dup_log overflow switch.** When `groups_gen − groups_consumed > CAP` (2^22 keys), the next logging statement sets `groups_state='pending'`, and logging stops. A rebuild follows, and the maintainer re-checks the state in every write transaction.
- **6.7 Ad-hoc `--distance`** under an `adhoc` lease (non-unique), with lease-named scratch.
- **6.8** The Groups pass is an idle round-robin pass (D17).
- **Tests:**
  - Partition proptests against `oracle_groups` (the parameter grid, HEAVY_ENTER = 2, t ∈ {10, 22, 31});
  - step-counter tests: ladder, chain, 1M clique;
  - the two-connection interleaving;
  - resume inside a 50k run;
  - a cut-vertex re-import;
  - the Duplicates filter equals today's EXISTS forms, including cascade deletes of present and missing files with twins;
  - stamp completeness: every status flip stamps;
  - a storm drives the overflow switch;
  - golden stdout for `--show 0`, `1`, `100` and for `--distance 10`, `22`, `31`.

### Phase 7: Grid, browse, tree, Qt edge, edits

**Probe findings (2026-11-11).** These are measurements, not decisions. Each finding sits under the step it informs. The probes covered scene-graph precision (a render test, P10), sub-pixel snapping, the aspect distribution (3000 photo headers; the catalog itself is empty), the window size, the density formula, `ItemRef` bursts under assumed flick speeds, `RankMap` cost from 1e6 to 1e9, and re-sync reach from 1e5 to 1e7 with three row caps. Still unmeasured: `ItemRef` queue depth in the real pipeline, because no pipeline exists yet. Recommended but not confirmed: a capped re-sync with a frozen tail (7.6), and the rule for a move from none (7.9). The grid design is not final.

**Files:**

- `crates/runs/`
- `catalog/src/browse/read/{changes, facets, grid, tree}.rs`
- `catalog/src/names.rs`
- `catalog/src/range_index.rs` (new, 7.17)
- `app/src/{reader.rs, edits.rs, app.rs, library.rs, views.rs, images.rs}`
- `client/src/{layout, layout/density, layout/provisional, grid, grid/refs, window, window/listview_rules, tree, controller/*, synthetic}.rs`
- `desktop/src/{gallery, shell, filters, facet_choices, settings, animation, video_source, yandex, folders, picker, picker/listing, test_hooks}.rs`
- QML: `WindowView.qml` (`WheelListView.qml` is deleted), `GalleryGrid.qml`, `GridRow.qml`, `GridCell.qml`, `Main.qml`, `Viewer.qml`, `Hud.qml`, `MediaView.qml`, `TopBar.qml`, `FilterBar.qml`, `TestHooks.qml`, `YandexPanel.qml`, `FolderTree.qml`, `FolderPickerDialog.qml`, and stubs
- `library/src/crop.rs`

- **7.1 Paged ChangeFeed** over (changed, id) plus `files_log`. `docs/plans/DECODE_ONCE_PLAN.md` Phase 4 builds its first form, and this step extends it rather than replacing it:
  - `files_log` with the `files_removed` and `files_renamed` triggers;
  - the `log_horizon` and `view_epoch` seeds, and `prune_files_log`;
  - the duplicate-status triggers, which stamp on a 1↔2 transition (with `LIMIT 2` probes; D27's `hash_runs` replaces them);
  - stamped grid results;
  - the cursor-paged feed (this step adds the converging read, D31, over it);
  - the behavioural schema test that every column `grid_sql` reads is covered.
- **7.2 Facets** as chunked steps, plus a contribution log. The removal mask is applied path-exact (D34).
- **7.3 Lanes**, with List coalescing. Every catalog-sourced result carries its snapshot stamp s = max(changed) (D33).
- **7.4 build_run.**
  - Single-lower-bound steps of 65,536 examined rows. The sidecar sort removes duplicates.
  - **The post-build delta is a converging paged read** (D31).
  - The removal mask (D34: key-range prefilter plus `folder_under!`).
  - The name driver (7.16) and the range-index driver (7.17).
  - **Sorted views:** a live top-K preview (K = 4096, a bounded heap over the rows examined so far, marked provisional) while the build runs. It re-anchors at the same top item when the build lands.
- **7.5 refresh_run:** an uncapped two-phase delta (converging), a RankMap, and persistence across restarts. A pending removal arrives as a filtered delete over its key range.
  - *Probe P9:* rank updates through a Fenwick tree are O(log n) and bit-equal to a full recompute. The row-layout delta regenerates the tail, so it inherits 7.6's invalidation problem. At 1e7 items with 1e4 changes it regenerates 104k to 110k of about 120k rows, a 1.2 to 3x gain over a full relayout.
  - *Probe RankMap:* a Fenwick over every slot of `i32` costs 5 bytes per slot, 4.66 GiB at 1e9 slots, which the scale rules out. A blocked form (u64 presence words, a Fenwick over per-word counts, popcount inside a word) gives checksums equal to the plain form at 1e6 and 1e7, passes brute-force checks at 1e6 through 1e9, and costs 0.19 bytes per slot (179 MiB at 1e9). Rank takes 13 to 83 ns, and update 12 to 77 ns. The universe's Fenwick should be the blocked form.
- **7.6 Directory:**
  - *Probe P4, not supported:* after one edit, the layout's re-sync with the old boundaries is heavy-tailed. On continuous aspects, over 1000 edits at 1e6 items, p99 is 103,112 items and 1,244 rows past the edit. On photo-like aspects, 69 of 1000 edits never re-sync. Uniform 3:2 never re-syncs in 200 edits. Bounded invalidation is therefore not supported.
  - *On the user's aspects* (3000 header aspects resampled to 1e6 items, probe `real`): 980 of 1000 edits re-sync. Median 796 items and 9 rows past the edit; p99 264,079 items and 2,712 rows; max 444,031 items and 4,561 rows. The other 20 never re-sync. The synthetic result holds, and the real tail is no shorter.
  - *Re-sync reach grows with the catalog* (probe `scale`): p99 items past the edit are 37k at 1e5, 148k at 1e6 and 448k at 1e7 (4.5% of N). The max at 1e7 is 1.75M (17% of N). Exact regeneration stays cheap (47 to 138 µs per edit).
  - A cap on the regenerated rows, with a frozen tail, keeps the y error within about one row pitch: max 65 px at 1e7 for caps 256 to 4096. The cost is membership: in the frozen tail, items sit in the wrong row until the background re-sync catches up. Recommended but not confirmed: cap at 4096 rows, which covers 95% of edits at 1e6 and 99% at 1e7.
  - *Density formula (derived and checked):* ρ ≈ (W+S) / ((ā·T + S)(T + S)), where ā is the mean of the clamped aspects. Row height is at most T and close to it, so a row holds about (W+S)/(ā·T+S) items, and has pitch T+S. Predictions against measurements at W 7680: the user's aspects (ā = 1.2557) 1.513 against 1.530; synthetic Photo 1.592 against 1.607; Continuous 1.284 against 1.311; densest (ā = 0.05) 17.15 against 17.16. The clamp caps the density at 17.15 items/px at W 7680, below the 22.91 in the earlier plan.
  - *Open:* the re-anchor that completes a capped re-sync in the background, and whether a frozen tail's membership error is acceptable in the UI.
  - u128 fixed-point checkpoints, bit-identical across parallel, splice and sequential builds.
  - `density.rs`: k_max, h_min, min_pitch and rho. The fixed rho = 22.91 is unreachable (see the formula). Replace it with the formula, evaluated from the catalog's own mean aspect.
  - `Directory::periodic` is compiled unconditionally.
  - **ProvisionalGeometry**, and a re-anchor that keeps contentY.
- **7.7 Client grid:**
  - *Probes P2 and P3, semantics hold, not yet implemented:* epoch-checked `ItemRef` dispatch misapplied 0 of 100k random interleavings, and the index-only control misapplied 99,964. `RefBook`, with two retained maps, resolves in-window references and rejects older ones without silently remapping them. The largest map was 78 KiB at 10k entries.
  - *Probe P8:* a checkpoint is 32 B. The full row list at 1e9 items is 224 to 276 MiB. The 11 MiB client figure holds only with checkpoints every 4096 items (7.45 MiB at 1e9), not with the row list.
  - Rank everywhere; u64 versions.
  - **RefBook** (at most 2 retained RankMaps, with a proven bound).
  - The overlay folds stamped Pendings until s ≥ c.
  - `held` becomes a pinned run handle.
  - `position_of`, `entries` and full relayout are removed.
- **7.8 SlotWindow:**
  - *Probe P6, golden rows for `listview_rules.json`:* (1) a same-count reset with no current row: a ListView goes to the top, while a Flickable keeps `contentY`, so the port saves and restores it. (2) A same-count reset with a current row: a ListView scrolls to show it. (3) A current item moved by reset: a ListView scrolls to its row index, and following by identity needs an explicit lookup. (4) A window origin move of +3 slots by reset: a ListView resets to the top, and the port needs `contentY -= 600`. (5) The same +3 move by removing the top three and appending three: a ListView keeps the visible content, and the port needs `contentY -= 600`. All five were run against a running Qt 6.12 ListView (slotprobe run 2) and match. `golden/listview_rules.json` is not yet in the repo.
  - Synchronous slot assignment for viewport ± 1500 px.
  - *Window size (probe `window`, and `real` for the user's aspects):* the window (viewport ±1500 px) holds 1,705 items at 1920×1080, 4,140 at 3840×2160, 4,698 at 5120×1440, and 11,462 at 7680×4320 for the user's aspects. Synthetic aspects give similar figures. The clamp's densest case (aspect 0.05) holds up to 127,368 items at 7680×4320. The capacity bound is ρ_max·(vh + 2·1500) + 2·k_max, with ρ_max = (W+S)/((0.05·T+S)(T+S)) and k_max the item count of the densest row. At 7680×4320 that is about 127,700, against 127,368 measured.
  - *Items entering per frame (assumed flick speeds):* at 10,000 px/s, at most 316 items enter per frame at 7680×4320 on the user's aspects, and 86 at 1920×1080. Entries arrive in row-sized bursts. A seek replaces the whole window, so its burst is the window's count (11,462 at 8K). This is the queue model only; no pipeline exists to measure it.
  - Slab rebase only at idle, and only beyond 2^40.
  - ListView rules ported from the Qt 6.12 sources (current remap, keys, follow, Contain/Beginning, reset), pinned by `golden/listview_rules.json`.
  - The bar's seek is coalesced per frame.
- **7.9 Desktop Qt edge:**
  - *Probe P1, absence at QML receivers:* a typed `int` receiver or `int` parameter turns an absent value into 0, and `select(int)` given `undefined` or `null` selected row 0. Only a `var` or a Rust-checked `QVariant` keeps absence, so Rust must check before a value reaches a typed side.
  - *Probe P5 (property level):* QML doubles round-trip exactly to 2^53 for integers, and `contentY` 2e11 reads back exactly. This does not show what the scene graph renders.
  - *Probe P10 (rendered; supersedes the P5 claim that a Flickable at 2e11 positions its children correctly):* a 1 px marker inside the Flickable's content item, moved by `-contentY` in the scene graph, lands on the wrong row from `contentY` 2^24. Float32 spacing explains it: 2 px in [2^24, 2^25), 4 px in [2^25, 2^26). By 2^28 the markers collapse to one row. A sibling marker whose y is computed in double is exact at every offset tested, up to 2e11. The slot layer must therefore be a sibling of the Flickable's content item, never a child.
  - *Probe sub-pixel:* a 1 px rectangle at y = 10 + 0.25 renders in row 10, and at 10.5 or 10.75 in row 11. Qt snaps positions to whole pixels, for literal and double-computed positions alike, so float error does not reach the rendered result. Images and text were not tested.
  - *Probe P7, read-only:* the controller cannot yet be the sole writer of the grid's current item. Click selection is written in QML (`Main.qml:574`). The controller learns of changes by echo (`input.rs:65`). The viewer's index shares its field (`Main.qml:605`). The anchor is the view's top item, which needs view geometry (`GalleryGrid.qml:41-42`).
  - *Rule for a move from none (recommended, not confirmed):* from none the position is −1, so a step of +d lands on row d−1, and the result is clamped to the rows. Up from none lands on row 0. The probe's old comment ("starts at row 0") was wrong for steps above 1. The code is kept.
  - *Version width (reconciled with the validation doc, item 7):* versions are `u64` in Rust. They cross as `i64` with a checked conversion, and QML's `real` holds them exactly up to 2^53. A version above 2^53 fails loudly.
  - f64 properties; `viewedFile` string.
  - `loadedAnchor`; the `lastFileIndex()` invokable; `save_last_file(Option)`.
  - ItemRef stamping on every queue hop.
  - `to_i32` and `qt_int` are deleted. i32 conversions carry a `qt-edge-bound` reason.
- **7.10 QML WindowView:**
  - A plain Flickable, plus a non-moving slots layer, plus a Repeater. The slots layer is a sibling of the Flickable's content item (P10).
  - Typed scalar signals.
  - doubles throughout; `'f',0` formatting; `file` strings.
- **7.11 Folder tree:**
  - The streaming per-generation tree files.
  - FolderModel as a `SlotWindow<Uniform>`.
  - Model-owned `currentRow`, with ported rules; native attached bar.
  - Row LRU of at least 2·S.
  - The tree is merged across drives (`browse/read.rs:1`). A pending removal subtracts its folders' counts from merged nodes, and hides only nodes left empty.
- **7.12 Folder picker:**
  - In RAM up to 4 MiB of names, synchronous.
  - Beyond that, spill-sorted on a worker into a PagedFile with offsets, with generation cancel.
  - Slot model.
- **7.13 Crops:** a streamed list, with a prepass refusal and unordered resume.
- **7.14 Live pages.**
- **7.15 Edit worker and pools:**
  - The `Edits` worker (FIFO of 1024) runs Rate, Turn, BorderMark, SaveListing, SaveView, DeleteView, AddRoot and RemoveRoot through the Gate. Its ack carries the commit stamp c.
  - The `ui` pool is query_only.
  - `App::stop` drains the queue.
  - Deviation: an edit shows at once, and reverts if the write fails.
- **7.16 Name index:**
  - The `names.sqlite` FTS5 trigram file (contentless, detail=none), with names padded with two U+0001, opened as `Role::Names`.
  - Fed by the ChangeFeed in the `Names` idle pass (D17).
  - Needles of 1 or 2 characters go through `fts5vocab` term-range prefix expansion.
  - It is used when Σ doc < scope/8. Otherwise the build scans.
- **7.17 Opt-in range indexes** (Q-G1 a, decided):
  - `gallery.json grid.range_indexes` lists any of size, mtime, taken, megapixels and duration.
  - **Build.** Each listed column is a 2.3 Shadow access path, `sh_<col>(value, id) WITHOUT ROWID`, built online as an Upgrade step (Phases R, M and L) in the background share. A plain CREATE INDEX is ruled out: the freeze guard refuses it on LARGE_TABLES, and it would make `mismatch()` non-None and force a rebuild.
  - **Upkeep.** After ready, the shadow is maintained by triggers whose names are covered by `difference_allowing`, so `mismatch()` stays None.
  - **Readers.** build_run and facets use the shadow when the range's candidate estimate is below scope/8, as 7.16 does. Candidate ids are ExtSorted by the view key, then filtered by scope, the removal mask and the other predicates. Results are identical to the scan.
  - **taken.** The taken step (2.5) marks `sh_taken` not ready, and rebuilds it afterwards. Readers scan in the meantime.
  - **Opt-out.** The triggers and the table are dropped in one DROP TABLE: reads are O(size), and the WAL is about pages/1000.
  - **Cost:** 20-30 GB per column at 1B. The build takes 1-2 h online.
- **Tests:**
  - `ids(build_run) == ids(list_grid_reference)` across SortKey × direction × query (Duplicates, name needles of 1 to 5 characters, the removal mask on mirrored drives, each range index) × scope × rules;
  - VM_STEP per step is O(step);
  - the delta proptest, with 50 chained refreshes at page size 1, with writes between pages;
  - Duplicates deltas;
  - the window, grid, tree and density laws at 2^31 + 2^20 + 7 and 2^40;
  - the RefBook proptest;
  - the golden ListView table, reproduced in Rust and by a real ListView in qml_host;
  - `tst_windowview`, `tst_gridrow`, `tst_viewer`, `tst_media_view`, `tst_foldertree_rules`, `tst_folder_picker_dialog`;
  - picker order equals `PathBuf::sort`;
  - overlay folding: a turn plus `refresh_turned`, a rating plus a delta, and an out-of-order arrival;
  - crop refusal writes nothing;
  - name index: lag, tombstones, reset;
  - range indexes: results equal the scan for every column, direction and scope, both during a build and after; opt-out and opt-in round trip; `mismatch()` stays None; `set_rating` at most 150 ms during the build.

### Phase 8: Verification harness, ledger, lints, docs

- **Files:**
  - `crates/scale/{src/bin/{synth, sql_ledger}.rs, tests/scale.rs, expectations.toml}`
  - `catalog/src/fixture/synth.rs`
  - `testkit/src/{alloc.rs, ntfs.rs}`
  - `scan/tests/{usn_oracle, usn_faults, usn_vhd}.rs`
  - `desktop/tests/qt_edge_lint.rs`
  - `tools/compare_shots.py`
  - `tools/shots/asserts/*.json`
  - `docs/perf/sql-plans.toml`
  - `Justfile` (`usn-vhd`)
  - `.github/workflows/{ci, scale}.yml`
  - `docs/SCALE.md` (new), and `docs/plans/{DATABASE, PLAN, REVIEW}.md`, `docs/reference/{STORAGE, HEAP_PROFILE}.md`, and `d28e2f72^:docs/temp/REFACTOR_PLAN.md`
  - `crates/README.md`
  - `.env.example`
- **8.1 Generator.** It produces every trigger-maintained value, each checked against its recount oracle:
  - `folders.present`, `folders.anchor`, `anchors`, `borders_due`, `pend_batch`, `unpaired_anchor`, `file_seen`;
  - `pdq_indexed` with batch;
  - the `counts` row, `hash_runs`, `pdq_near`;
  - `view_epoch`, `catalog_opens`;
  - for the journal rows: a `usn_dirs` mirror, and census and `usn_roots` consistent with a FakeVolume tree.

  `Spec` gains a planted near-duplicate dihedral distribution and a multi-drive mirrored layout. Generation runs in a child process.
- **8.2 Ledger** (`just plans`):
  - statement tags;
  - class, temps, fan-out and total order;
  - "folders.path, folders.key and folders.anchor are immutable": no UPDATE OF those, no DELETE, and trigger bodies are checked too;
  - every statement that writes `unpaired` or `folder_id` also writes `unpaired_anchor` (D9);
  - the **read-transaction census** (max at most READ_TX_MAX, with offenders listed);
  - the missing-writer allowlist (3.8);
  - open findings.
- **8.3 Lints:**
  - `qt_edge_lint`;
  - autoindex, FK-index, no-upsert-on-analysis and no-REPLACE (2.1);
  - partial-index literal terms;
  - `pending_now` and `taken_now` usage;
  - `no_memory_temp`;
  - scans readers use `id > 0`.
- **8.4 Assert scenarios:**
  - `--synthetic-list`, `--dump-state`, `--bar-at`;
  - `$Asserts` in compare-shots, run on the working tree only, failing on mismatch: `huge-list` at 2,148,532,231 and `huge-keys` at 4,294,967,303.
- **8.5** The scale harness and matrix are in §8. Docs are updated, including the test-layer split and the journal operations guide (`gallery journal`, fsutil, Q-S5).
- **8.6 Calibration** of the 3.9 cost-rung constants c_e, c_d, c_p and c_o, from the scale runs, into `expectations.toml`.

## 5. Alternatives considered and rejected

Each entry was proposed and not adopted. The reason is given so that it is not proposed again without new evidence.

**Schema, rebuild and migration:**

- Keep the old catalog file at its path during a rebuild (copy without delete, +1x disk): rejected. The kept file is moved aside and protected by its change counter instead.
- A write fence that keeps the path: rejected. The moved file is kept by the rebuild, and its change counter catches any commit, so the rebuild restarts at Copy (KeptChanged). After Swap, a descriptor opened without prepare in the close-to-rename window is today's known window (`move_aside`, `lib.rs:347`).
- A two-hop swap: unnecessary, because D24 turns out-of-order UNIQUE constraints into droppable indexes.
- An `immutable=1` source for the kept file: rejected.
- A removal lease: rejected. The per-row recheck, the hidden counter committed with each batch, and the forward watermark make concurrent pagers exact. The CLI reads the finished row. A lease with no target would block the CLI for hours.
- A stepped in-place recount after every change: partly overturned. D22 keeps per-family cursors for post-freeze migrations, and the stepped recount is used only for those.
- A plain CREATE INDEX on a large table after the freeze: refused by the guard. Shadow (2.3) is used.
- An id-order shadow backfill: 1B random inserts.
- A durable sort for bulk_due: the walk is cheaper.
- Version recomputes that let desktop values go stale while they run, or that rewrite a pending batch through live indexes: rejected. D20 keeps reads exact and writes no index pages.
- A tier-1 gate and backlog drain during the import step: removed. D25's session clock makes pairing independent of the step.
- A snapshot, or an O(n) materialization at start, to make the cursor exact: rejected. D6 is a stated deviation instead.
- The status pair counter as total minus touching-missing: replaced by Σ bands, which Phase 5 already maintains.
- `set_counts` at prepare, and a `COUNTS_KEY`: removed (D22).
- A per-root partition of the unpaired set: overturned by D9. Folder keys omit the drive, so a root's scope is (anchor, key range).
- Re-keying the unpaired index by order_key alone: drives merge in the key, so the anchor must be part of the scope (D9).
- Re-ordering the stamp order for removals: rejected. The stamp order is internal, and it is a stated deviation (§7 Q-D).

**Thumbnails:**

- A sweep cursor t, to keep catalog changes prompt: rejected. Laps are used.
- A per-member gap counter: goes stale.
- A catalog witness that checks one row: too weak.
- A durability barrier, or synchronous=FULL, for the cache: not needed, because the cache is rebuildable.
- Skipping the decode at cache-full unconditionally: rejected. The memo (Q-T4 a) skips only while the stored rejection still matches.
- A snapshot of the cache taken at open: O(entries) at startup. The chunked heal on the writer was chosen.
- The old Listed and Walk tiers, and the `checked` and `entries_lru` tables: removed (D5, D35).

**PDQ:**

- A per-run heavy bitmap (n/8 bytes of RAM): rejected.
- Query-major resolution: about 2 PB of work.
- A witness row plus a function that reads runs; signed correction stores; demotion; WAL-checkpoint retention; history-free pairs: rejected.
- A batch-start pairing clock: a deviation whenever an onset happens mid-import. The session clock (D25) replaces it.
- A pending skip on queries: dropped. Today pairs a file at store whatever a later onset does.

**Scan and USN journal:**

- OpenFileById live values as the truth: the walk records directory-index values (§3, value truth).
- A links table: changes the walk's semantics.
- Stamping every seen row: O(n) WAL per pass.
- Ids in record order: changes the ids.
- Sealing sub-walk entries: sub-walk entries need no scratch.
- Opening every moved entry: moves under a DropSubtree skip the opens.
- Hashing the whole token: a new logon would force a re-probe. The projection is logon-independent instead.
- Dropping every logon-type SID: rejected. Only the per-session SID (the group flagged SE_GROUP_LOGON_ID) is left out, while well-known logon-type SIDs are kept, because they change access.
- The per-folder count check, and polling every Foreign identity: too costly.
- Rewriting `scan_skipped`.
- Reading `$UpCase`: not openable unprivileged. The 64K table is used in probe mode, and only where the name order is table-independent.
- Base-time link counts: admin-only, or O(n) opens.
- An elevated helper process for scanning: rejected. The privileged read is used only when the process is already elevated. The per-session helper is for MFT reads and journal sizing.
- One cursor per volume: each root keeps its own cursor, and roots with equal cursors share reads.
- ReturnOnlyOnClose or reason filtering.
- Trusting detachable buses by default: Q-S3 (a) is recommended instead.
- Auto-resizing the journal: resizing needs consent (Q-S6).
- Carrying `usn_*` through a rebuild: each NTFS root walks once after a rebuild.
- An unfiltered watch: most records would be the app's own writes. The relevance filter is needed.
- Directory identity on `folders` rows: lazily created `folders` rows cannot carry it (`scan/src/lib.rs:300-301`). A separate `usn_dirs` mirror holds every directory.
- `scans.full_every` for verification walks: replaced by `journal.verify_days` (Q-S5).
- An in-order DFS walk: changes row ids and hard-link winners (§6 R3).
- A volume service: §1.3.

**Borders:**

- One walk plus a 100-150 GB ExtSort: scratch grows with the library, at 1B. Chosen: two listings, with a non-blocking reorder.
- Renaming the CSV into place: drops the ACL and hard links. Chosen: copy in place.
- Truncating the CSV from its first row: a midway error loses the old CSV (Q-B1 b).

**Work lists and grid:**

- A `changed > last` cursor alone: misses deletions and renames. `files_log` is required.
- Seeding the dups `done` count, or writer-side draining for dups: rejected.

**Qt edge:**

- Owning Flickable inertia: relies on private Qt logic (`qquickflickable.cpp:1150-1170`, `:1498-1531`). Rejected.
- A ScrollWindow QObject property: rejected in favour of a Rust SlotWindow.
- Model-driven scroll bars for all views: rejected.
- Capping the lead L: rejected. Synchronous slot assignment for viewport ± 1500 px is used.

**Startup:**

- A prepare that blocks the window: replaced by an O(1) prepare, plus a progress window (D36).

## 6. Residuals: what is proven impossible to do better

Each residual is a cost that no design in this plan avoids, with the argument for why.

1. **R1. Rebuild disk ≥ size(old) + size(new), about 2 TB at 1B.**
   - The old file is kept whole (`lib.rs:143-145`), and the new one holds every carried row, so two copies coexist at the swap.
   - Only an in-place migration avoids this. After the freeze, that is RebuildTable (2.3). The BASELINE route is the user's decision.
2. **R2. Rebuild time is Θ(n log n): 6-12 h on NVMe. A resume loses at most one bulk statement (one CREATE INDEX, at most about 30 min).**
   - The row layout changes, so every row is rewritten: Ω(n).
   - Building a B-tree index over unsorted keys needs Ω(n log n) comparisons.
   - SQLite fills an index only through one CREATE INDEX statement. An online build through the WAL (Shadow) doubles the I/O in a pass that is already exclusive.
3. **R3. Huge-folder frame scratch is Θ(pending subfolders), only above 16K subfolders, about 8-20 B per frame.**
   - The walk pops frames LIFO (`scan lib.rs:236`, `:292`), so it visits subfolders in reverse listing order.
   - Reversing an m-item one-pass stream with M words needs p·M ≥ m passes. Re-listing 1B entries 61K times is infeasible.
   - An in-order DFS would change row ids and hard-link winners.
4. **R4. Borders report: scratch is O(entries) for a single directory above the sort budget.**
   - Munro-Paterson: sorting n keys from read-only input in p passes needs p·M = Ω(n).
5. **R5. A missing toggle costs Θ(degree) reads, and `pdq_near` and band upkeep cost Θ(degree).**
   - A toggle changes the presence of every incident pair.
   - Exact O(1)-report counters must move by that many. Any per-file pre-aggregate moves the same work onto the partners. This is also why a pending removal shows pdq_pairs as-of (§7 Q-D).
6. **R6. CREATE INDEX holds one page buffer per PMA: index/32768, which is 1.6 MiB at 1B.**
   - vdbesort opens every leaf PMA reader at merge time, and maps temp files only below 2 GiB.
   - The heap P + P + index·pgsz/P is minimized at Θ(√(index·pgsz)). No fixed P makes it constant.
7. **R7. A borders CSV run in progress holds up to the new CSV's size beside an existing CSV.**
   - Today's contract is "untouched on error" plus "replaced in place on success". With no RAM buffer, the new bytes must be stored outside the old file until success.
   - Truncating up front breaks the first half of the contract (Q-B1).
8. **R8. Cold-cache removal and mark_missing cost Ω(uncorrelated indexes − 1) random reads per row: 3-80 h on NVMe, possibly months on a cold HDD.**
   - SQLite B-trees have no batched update. Any order is sequential in at most one index.
   - Dropping and re-sorting the indexes would make the catalog unusable for hours, which contradicts the online removal of D34.
9. **R9. Cross-OS (Windows and WSL) lease exclusivity under an unbounded pause holds for at most one in-flight transaction.**
   - No shared atomic primitive exists across the boundary (`schema/src/lib.rs:136-139`).
   - Gray and Cheriton: a lease is safe only under bounded pause and drift.
   - The token fence, pause detection, counter heartbeat and kept change counter narrow it to one transaction.
10. **R-SCAN. A full walk reads every listed entry, plus one catalog identity probe per entry: 200-400 GB, 1-4 h at 1B.**
    - **When it still runs:** step 3.9 uses the USN journal on local fixed NTFS volumes, so a full walk runs only in the fallback cases:
      - a root's first scan;
      - a 3.9 ladder fallback: journal inactive, wrapped, recreated or rolled back; a fingerprint, rules or collation change; an unverified unprivileged read; SeChangeNotify disabled; a cost-rung crossover;
      - a Q-S5 verification walk;
      - a requested `scan --walk` or Full rescan;
      - Linux/WSL roots, network shares, ReFS and other filesystems, and detachable volumes;
      - the first scan after a rebuild (`usn_*` is not carried).
    - **Why:** without a filesystem change log, a change can be any entry. An adversary argument shows that every entry must be listed and compared with its stored state. The probe is the minimal comparison.
11. **R-PAIR. First-scan pairing costs about 20-25B B-tree probes.**
    - Each fresh row needs its tier lookups, and each lookup is an index probe. The tiers' candidate sets have no common order (identity, folder/name, file_ref), so the probes cannot be merged into fewer sequential scans without O(n) sorts per tier, which costs more.
12. **R-PDQ1. Pairing CPU of about 26 core-days at 1B.**
    - 8 × 32-bit chunks minimize probes plus accidental candidates.
    - A subquadratic exact Hamming join at d ≈ 8.5 log₂ n would refute SETH (Williams 2005; Alman and Williams 2015).
13. **R-PDQ2. k² CPU (not storage) for dense clusters: about 360 ns per near-copy pair (a 1M cluster is about 50 core-hours).**
    - Band counts, as a function of the present subset, determine every pair's band: Θ(k²) bits.
    - o(k²) storage must recompute, and one flip needs Ω(deg).
14. **R-PDQ3. About 18 KB of disk per heavy file.**
    - Stateless exactly-once emission needs the full dihedral at all 64 sort positions.
    - The alternatives are Θ(pairs) state or Θ(pairs) random reads.
    - HEAVY_ENTER = 512 equalizes the worst light and heavy cost.
15. **R-PDQ4. Pair lag is at most L_lag + t_sweep. Lone-image latency is bounded by the distinct probe blocks (about 1.9M at 1B).**
    - Above q*, one sequential read of the index is the cheapest exact read.
    - Fewer runs would need merge transients above 146 GB.
16. **R-PDQ5. Exact PDQ counts while heavy presence changes are unsettled need Ω(Σ deg) work.**
    - Hence the as-of note, or a wait (5.8). Today's exact `stats()` is itself O(rows).
17. **R-TH1. A height with no usable state walks N rows once (14-28 h at 1B), resumable.** This covers a new height, a start-over, a rollback, a fork or the abort rule.
    - Nothing records which files lack a current entry at that height, and the walk is proportional to its own output (the decodes).
    - The abort rule costs at most 2× the cheaper of its two options.
18. **R-TH2. The progress total is unknown while a walk, range or revalidation is open (Q-T6).**
    - A range's needed count is a join across two databases with no subtree counts.
19. **R-TH3. Compaction re-walks at most N/2 rows per compacted height per lap.**
    - Pigeonhole: more than B arbitrary keys in B members must lose information. Smallest-gap merging minimizes the loss.
20. **R-TH4. Fork detection fails with probability 2^-63 per fork.** Any bounded detector is a fingerprint.
21. **R-UP1. A version bump writes Ω(n) flag pages, about 1-2 h sequentially.**
    - The flag lives in every row. A version-relative flag would need a permanent 30-40 GB index, and could not be expressed in the `files_borders_due` partial index.
22. **R-RD. A converging read's final iteration is bounded by the rows changed during the previous iteration.**
    - An exact snapshot must read every changed row in one snapshot. The WAL ceiling (D18) slows the batch writers that would otherwise let the reads converge.
23. **R-QT1. Flickable contentY drifts between idle frames. The double keeps 2^-11 px up to 2^40 px.**
    - Reaching that takes about 4.6 years of continuous maximum-velocity flicking.
    - A running Flickable timeline cannot be translated (`qquickflickable.cpp:406-412`).
24. **R-QT2. Scrollbar resolution is about n/h items per bar pixel.** This is inherent to any scrollbar. Exact reach is kept through the wheel, keys and reveal.
25. **R-QT3. Delegates per row are set by geometry, as today: up to about 171K cells at 7680 px, T 60, 1:20 strips.** Only a product change to `clamp_aspect` or the minimum row height would lower it.
26. **R-Q1. Unindexed predicates cost O(scope) per query edit. Every filter costs Θ(matches).**
    - An adversary can make any unexamined row match.
    - Per-column range indexes are opt-in (Q-G1 a, 7.17). Columns that are not opted in keep this bound.
27. **R-RO. The ordered-sink spill holds up to the full output size in the extreme.**
    - A row cannot be emitted before every earlier row exists, so the later rows must be stored somewhere.
28. **R-USN1. A journal pass lists a dirty folder of m entries in full (O(m)) in four cases:**
    - one of its changed files denies FILE_READ_ATTRIBUTES to OpenFileById;
    - a probe contradicts `open_by_id`;
    - more than one of its intent-carrying names (files, or separately subfolders) is non-ASCII, since NTFS `$UpCase` tables can differ outside ASCII, and the volume's table cannot be read unprivileged;
    - m ≤ max(4096, 64k), where listing is the cheaper choice.
29. **R-USN2. Changes beyond the journal's MaximumSize between two passes, or during a base walk, force a full walk.**
    - The journal is the only record of them.
    - Resizing it needs an administrator. The app offers a consented resize through the helper, sized from the measured record rate (3.9, runner and CLI). Declined offers and non-admin users fall back to the walk.
30. **R-USN3. A change of the token's access projection costs an O(dirs) access re-probe of the root's directories (about 5-25 min at 10-50M directories), instead of a walk.** A changed projection means a different user, group set, elevation, integrity level, or SeBackup/SeRestore state.
    - Access is evaluated per directory against the token. No record reports a token change.
    - Alternating an elevated and an unelevated scanner pays this at each alternation.
31. **R-USN4. Each pass costs O(volume records since the cursor) to classify, even for records outside the library, plus O(|state-1 directories| + |usn_stale| + |woken Foreign identities|).**
    - The journal is per volume, so a record's relevance is known only after it is read.
    - The walk retries every unreadable folder, so state-1 directories are retried too.
    - The crossover counts relevant records only.

## 7. Open questions and accepted deviations

**Decided** (recorded so that later steps can cite them):

- PDQ radius stays exactly 31.
- One BASELINE edit, then freeze (D1).
- **Q3:** missing files are ignored when pairs form (recommended, taken).
- **Q4:** scratch lives on the catalog volume, with typed out-of-space errors (recommended, taken).
- **Q5:** budgets as stated in §2 (recommended, taken).
- **Q6:** batched commits are accepted. Per-row missing updates, pairing pages and removal pages are committed in batches.
- **Q10:** indeterminate progress is shown while an unsized step runs (recommended, taken).
- Exact global grid layout, provisional briefly while a run builds (7.4 and 7.6).
- **Q-D:** the stated deviation set below is accepted as a whole.
- **Q-T4 (a), cache-full memo:** skip the decode when the cache state and the next batch's (id, seq) match a stored all-success rejection (4.6).
- **Q-G1 (a), range-filter indexes:** none by default, opt-in per column in gallery.json, built as online Shadow access paths (7.17).
- **Q-S1 (b), NTFS change journal:** the USN-journal incremental rescan is included as step 3.9, with D37 and D38, the full-walk fallback, and deviations J-a to J-f.
- **Q-S6, journal sizing:** the app offers a resize with consent, through the per-session helper, sized from the measured record rate (3.9). It never resizes silently. `gallery journal size` stays as the manual path. There is no volume service.
- Process rules: implement everything, build once at the end, commit on the checked-out branch, and Phase 0 is the user's.

**Accepted deviations (Q-D).** These are accepted by the user as a set.

- **D6, cursor subset:** no failure row for a file deleted mid-import. A file modified mid-pass waits for the next pass.
- **Pairing:**
  - Q3: extra raw pairs exist while files are missing;
  - a pending revert re-indexes the file;
  - concurrent imports in two processes no longer miss each other's pairs;
  - legacy catalogs are re-paired;
  - PDQ counts are as-of while heavy changes are unsettled (`gallery status --wait` for exact; `gallery dups` waits).
- **Scan:**
  - partial recording on a relist failure, or a stop inside a huge folder;
  - out of space is typed;
  - **(J-a)** A hard-linked or held-open media file keeps the values last recorded when its directory entry in the library either:
    - (i) was already stale at its root's last full walk: changed through a link that no walk of the root lists, or written while open, before that walk; or
    - (ii) belongs to a file that denies FILE_READ_ATTRIBUTES to OpenFileById.

    This lasts until the entry is refreshed without a journal record and its folder is next relisted (which then also queues a verification walk), or until the next full walk. Nothing else differs.
  - **(J-b)** Writes that bypass the NTFS change journal are seen only when their folder is next relisted (a full comparison; an unexplained difference queues a verification walk), or at the next full walk (Q-S5, 30 days by default). Examples:
    - another OS's NTFS driver (Linux ntfs3 or ntfs-3g, macOS drivers) on any disk, including internal fixed dual-boot disks;
    - offline edits; chkdsk /f repairs and NTFS self-healing;
    - an image restore that keeps the journal ID and position.

    Detachable buses always walk unless the volume is opted in. These are USB, 1394, SD, MMC, virtual, and hot-pluggable or removable media, including Thunderbolt/USB4 NVMe and eSATA.
  - **(J-c)** An entry modified while a journal pass runs, or between the read side and a resumed replay (up to the 24 h resume bound), is recorded with the values read. The next pass corrects it, because its journal range contains the change. The walk has the same window per folder.
  - **(J-d)** A folder with no journal record since the cursor, which would now fail to open or list for a non-access reason, is treated as readable as last listed. The reasons are: sharing violation, ERROR_FILE_CORRUPT or disk corruption, device not ready, resource exhaustion, or a relist failure. Its failure, error count, clean flag and missing-exclusion appear when the folder is next dirty, at a re-probe, or at the next full walk. Folders already unreadable are retried every pass, as the walk does.
  - **(J-e)** An ACE that names a logon-session SID (S-1-5-5-X-Y) on a library folder is re-evaluated only when the folder or its parent has a journal record, or at the next full walk. The per-session SID is left out of the access projection, so that a new logon does not force a re-probe.
  - **(J-f, kept only if the real-NTFS journal test (3.9) shows that NTFS writes no record at a POSIX unlink before the last close)** A file POSIX-deleted while another process holds it open stays present until that handle closes.
- **Rebuild:** (a) to (j), in 2.2.
- **Upgrade:** ordering of ungated passes; the status line shows progress; a zone restart (non-Windows only).
- **Removal:**
  - internal stamp order (§5);
  - **mask (3.4):** while a removal is pending, the app's analyzed, failed, pending and pdq_pairs counts are shown as of its start, with a note. Present and missing are exact. The Duplicates filter still counts twins under the removed root until they are marked. The CLI is exact, because it drives removals first.
- **Thumbnails:**
  - the app yields to `gallery thumbs`;
  - retry once per lap;
  - at most E + BATCH duplicate decodes per steal;
  - `TotalsFixed` after open;
  - a dropped PutOne under a stall;
  - cache-full memo (Q-T4 a): a source that changed on disk without a catalog or cache change is not re-tried until the next such change.
- **Borders CLI (4.8 a-d):** a tree that changes between the two walks; the `.partial` file while the run is in progress; serde's error text for non-UTF-8 input; review marks committed in batches.
- **Duplicates views** reflect heavy flips at settle-publish.
- **GUI edits** are optimistic, and revert on failure.
- **Write-back:** offline parking cadence; PROBE_WAIT; recent_errors shows the latest errors.
- **`file_by_path`** returns the lowest matching id on ties.

**Open questions.** The recommended option applies unless the user says otherwise.

1. **Q-B1. Borders CSV.**
   - (a) Write `<csv>.partial` beside the CSV, then copy it in place. The old CSV is untouched on error before the copy, and keeps its identity. Disk peaks at old + new while the run is in progress (recommended).
   - (b) Truncate in place from the first row. 1x disk, but an error midway loses the old CSV.
2. **Q-B2.** Print indeterminate stderr progress during the borders pre-walk (recommended: yes).
3. **Q-R1. Rebuild indexes.**
   - (a) Regenerate every index from the rows, as today. This heals a corrupt index (recommended).
   - (b) Copy unchanged tables' indexes through xfer: about 2x faster, but carries corruption over.
4. **Q-W1.** WAL_CEILING of 512 MiB (recommended), or larger.
5. **Q-W2.** PROBE_WAIT of 5 s (recommended), or longer.
6. **Q-T1. Thumbnail lease.**
   - (a) The CLI steals, and the app retries (recommended).
   - (b) The CLI waits.
   - (c) The CLI refuses.
7. **Q-T2. Failure retry cadence.**
   - (a) Once per lap (recommended).
   - (b) Every round, which brings back starvation at 1B.
8. **Q-T3. State budget.**
   - (a) max(2^20, N/16): at most 5.6 GB at 1B (recommended).
   - (b) N/32: 2.8 GB, at about twice the compaction re-walk cost.
9. **Q-T5. Existing thumbnail caches.**
   - (a) Start over once at this layout edit, as the cache's documented policy says. This takes minutes at 189K files (recommended).
   - (b) A one-time in-place upgrade.

   In both cases the cache layout freezes with BASELINE.
10. **Q-T6.** Indeterminate thumbnail progress while a walk, range or revalidation is open (recommended: show it).
11. **Q-P1.** HEAVY_ENTER = 512, the break-even point (recommended).
12. **Q-P2.** Expose `tier1.max_pair_lag` (default L_lag = clamp(5 × t_sweep, 30 s, 60 min)) in gallery.json (recommended: yes).
13. **Q-P3.** Legacy catalogs re-pair under the new definition (recommended), instead of carrying history-dependent rows. Keeping them would break the 512-row bound.
14. **Q-S2. `unpaired_with_data` count.**
    - (a) Keep it O(that root's unpaired rows), read through the two (unpaired_anchor, order-key) ranges (recommended).
    - (b) Maintain a per-root counter, with triggers on every child table plus re-attribution on root changes.
15. **Q-S3. Detachable volumes.**
    - (a) Volumes whose bus type, hotplug or removable flags mark them detachable always use the full walk, unless listed in gallery.json `journal.trust_detachable` (recommended).
    - (b) Trust the journal on them by default. J-b then becomes common, because such disks are often written by drivers that keep no journal.
16. **Q-S4. Cost crossover.**
    - (a) When the estimated pass cost (relist, moved-subtree and state-1 entries, probes, opens) exceeds 0.8 × the walk's estimate, use the walk. Results are identical; only cost differs (recommended).
    - (b) Always use the journal when the ladder passes.
17. **Q-S5. Verification walks.**
    - (a) A full walk per root every `journal.verify_days` (default 30, 0 = never), also queued at once by an unexplained difference in a relisted folder. This bounds J-a, J-b and J-d (recommended).
    - (b) Never; journal passes only.
18. **Q-S7. Journal watch in the app.**
    - (a) The app watches each capable volume, and queues a Scans request 5 s after a relevant journal record. The record filter is relevance-based, the watch never writes, and it releases the volume on lock or removal (recommended).
    - (b) No watch. Rescan stays manual, and journal passes run only when a scan is requested.
19. **Q-G2.** Front-coded order keys for HDDs: not adopted (recommended). SQLite index pages cannot front-code, so it would need a custom index.
20. **Q-G3.** Live mode ("N+", scrollbar off): not needed, since ProvisionalGeometry covers it (recommended: no).
21. **Q-G4.** Persist the current grid run across restarts (recommended: yes, as 7.5 has it).
22. **Q-DP1.** `--distance N` other than 22 stays exact through a one-shot build (hours at 1B, under the adhoc lease) (recommended).
23. **Q-M1.** Expose the memory budgets (role caches, W, Budget shares) in gallery.json (recommended: yes).
24. **Q-SC1.** Where the weekly 10M and occasional 100M scale runs go: a self-hosted runner (recommended), or D:. D: has 802 GB free, and a 1B library does not fit there.

## 8. Verification

**Harness:** `crates/scale/tests/scale.rs`, `#[ignore]`, `--test-threads=1`.

- Catalogs at N and 10N come from a child `pg-scale synth`, cached by schema hash: smoke 100k and 1M, full 1M and 10M, manual 10M and 100M.
- Two configurations:
  - **Uniform:** `CacheBudget::uniform(4 MiB)`, hard_limit = Σ open connections × 4 MiB + 16 MiB margin;
  - **Defaults:** real role caches, W and Budget.
- A counting global allocator; `pg-catalog/instrumented` (steps, authorizer, read-transaction spans).
- Samplers every 50 ms: the temp dir; `<catalog>.scratch/*` per owner (journal-pass runs included); `<catalog>.tier1/`; runs; WAL; plus the `wal_hook` frame high-water and WAL bytes per row.
- Journal rows run on FakeVolume over synthesized catalogs. The real-NTFS and VHD suites are in `just test` and `just usn-vhd`.

**Assertion classes** (between N and 10N):

| Class   | Meaning                                                                                         |
|---------|-------------------------------------------------------------------------------------------------|
| RAM     | Rust peak Δ ≤ max(4 MiB, 5%); SQLite highwater flat and under the limit; PeakPagefileUsage flat |
| STEPS-N | VM steps ratio ≤ 15 (O(n) passes)                                                               |
| STEPS-Δ | O(changes)                                                                                      |
| STEPS-1 | flat (O(1), O(show), O(window), O(page))                                                        |
| DISK    | the named sampler ≤ the stated bound                                                            |
| WAL     | per-commit frames ≤ F + 64·P; live WAL ≤ ceiling + batch                                        |
| RTX     | max read transaction ≤ 2 s                                                                      |
| ABS     | Defaults configuration: PeakPagefileUsage ≤ the §2 derived sum                                  |

**Matrix: every §2 row has a scenario and an assertion.**

| §2 row                                    | Scenario(s)                                                                                                                                                                                                                                                                                                                                                                              | Assertions                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
|-------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Scan walk                                 | First scan of: one N-file folder, N subfolders, a balanced tree. No-change rescan walk. A 10N-subfolder single folder. Validate and relist injection. A stop inside a listing. A first walk with mirror upkeep.                                                                                                                                                                          | RAM; STEPS-N; WAL; DISK (entry scratch 0; frames ≤ 20 B × N + one block; `usn_dirs` ≤ 280 B/dir); rescan WAL ≤ about 14 B per row                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Journal rescan                            | No-change pass at N and 10N; 10^4 and 10^6 records (FakeVolume) over N and 10N catalogs; a renamed folder of N/100 and of 0.5·N files; an R-A re-probe; a multi-root shared read; stop in R and W, then resume; Budget{1,1}                                                                                                                                                              | RAM (counting allocator ≤ the 3.9 bound); STEPS-1 for a no-change pass, as long as \|state-1\|, \|usn_stale\|, \|usn_wake\| and the root's unpaired rows are held fixed between N and 10N (8.1 generator); otherwise STEPS ∝ those sets. A pass with zero relevant records and empty recurring sets stops before R6: no scans row, no W5 or W6. STEPS-Δ (∝ records + dirty entries + sub-walk entries + affected rows); WAL; RTX; DISK (scratch ≤ 60 B/record + 150 B per non-subtree intent, 0 per sub-walk entry); half-library rename ≤ 0.6 × the walk's time; oracle-equal to `--walk` |
| Pairing                                   | Two roots; two drives with mirrored layouts; a drive root; save-overs; rebinds; ambiguous; moved; deleted; a stop between pages, then resume; a journal-pass floor                                                                                                                                                                                                                       | RAM; STEPS (rows visited ≤ that root's (anchor, range) unpaired rows); 0 write transactions when nothing pairs; resume equals uninterrupted                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| mark_missing / removal / `import --again` | Remove an N-file single-folder root (with a mirrored same-named root on a second drive); delete N in one folder, then rescan; `--again` over N; warm and cold timing; a scan request during a removal                                                                                                                                                                                    | RAM; WAL; STEPS-1 per page; the mask hides at once and leaves the other drive visible; status present and missing exact; a requested scan starts within one 60 s slice; time per row recorded (R8)                                                                                                                                                                                                                                                                                                                                                                                         |
| Import work list (+retry)                 | Import with a stub analyzer; `--retry`; preemption at every fetch                                                                                                                                                                                                                                                                                                                        | RAM; examined ≤ n + preemptions × 4096; done == total                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Version bump                              | Bump import, borders and zone at N and 10N, preempted at every batch; a reader during Pass A                                                                                                                                                                                                                                                                                             | prepare STEPS-1; Pass A no index writes (fan-out); RTX; exact reads equal the oracle; `pend_batch` equals its definition                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Status / writer counts                    | `gallery status`, `gallery write` poll, writer drain of 10^5                                                                                                                                                                                                                                                                                                                             | STEPS-1                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Write-back queue                          | Q failed rows; 10^5 parked on an absent root                                                                                                                                                                                                                                                                                                                                             | `next_ready` STEPS-1; at most one probe per 30 s                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| PDQ                                       | (1) Incremental: a fixed 10k new images against N and 10N indexes. (2) Maintenance and the debt cap after a long import. (3) Bootstrap and legacy re-pair at smoke size. (4) Lone image (Q=1). (5) Planted clusters of 10k and 100k. (6) Re-import peak. (7) Flips of a whole cluster. (8) Re-store after an old onset, then a later session. Explicit B_batch and T keep R=31 feasible. | RAM ≤ W + 8.4 MB (dhat Rust heap ≤ W + 2 MiB); DISK (temp O(k) in k; tier1 dir; re-import ≤ 2 × steady + 146 GB); run count ≤ cap; sweep reads each sheet byte once; odd fraction reported; (8) pairs equal today's index-load oracle                                                                                                                                                                                                                                                                                                                                                      |
| Borders list                              | Borders pass after a detector bump                                                                                                                                                                                                                                                                                                                                                       | RAM; STEPS-N; no index writes in Pass A                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `borders --csv`                           | CLI under `run_in_job` over a SynthTree; one directory of 10N; a 2,000-level chain; a slow head; an injected copy failure                                                                                                                                                                                                                                                                | RSS flat; DISK (directory spill only above budget; partial ≤ CSV size; reorder spill freed); the partial kept on a copy failure                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `--import-review`                         | 10N entries                                                                                                                                                                                                                                                                                                                                                                              | RSS flat; DISK runs about 100 B per entry; probes STEPS-1 each                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Thumbnail pass                            | Evict 1K at a walked height; a preempted first pass; 1% persistent failures, with a stop every PAGE; an offline volume holding 20%; cache-full × 20 (memo); mass seq bump; mass deletes past the prune; two-height alternation; restore and advance                                                                                                                                      | STEPS-Δ or STEPS-1 per round; w monotone; DISK (state ≤ budget + lost bound); RAM                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Thumbnail cache upkeep                    | Heal due at E and 10E; evict at the cap with keep dominating                                                                                                                                                                                                                                                                                                                             | open STEPS-1; RTX; evict steps per victim flat                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Dups                                      | Build and replay; report; storm (overflow switch); Duplicates filter over a k-member, mostly-missing run                                                                                                                                                                                                                                                                                 | RAM ≤ 300 MiB + W; DISK (dup_log ≤ CAP); report STEPS-1; filter flat in k                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Grid                                      | Name and sorted builds; deltas of 1k and 3M; window scroll at 2^31+2^20+7 (periodic); Duplicates-filtered refresh; top-K preview; removal mask on mirrored drives                                                                                                                                                                                                                        | RAM (client ≤ 11 MiB + window bound; build ≤ 330 MiB); STEPS-Δ for refresh; DISK runs ≤ 96 GB at 1B, scaled; RTX (converging delta)                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Name filter                               | Needles of 1, 2 and 5 characters, rare and common                                                                                                                                                                                                                                                                                                                                        | STEPS ∝ candidates; scan fallback at scope/8                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Range-filter indexes                      | Opt in size and taken at N/10N; build under concurrent writes; a taken step; a selective and an unselective range; opt-out                                                                                                                                                                                                                                                               | Results equal the scan; STEPS ∝ candidates below scope/8; DISK ≤ 30 B per row per column; `mismatch()` None; `set_rating` ≤ 150 ms during the build; RTX                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Folder tree                               | Build plus slot scroll at 3e9 rows (Uniform); a pending removal on one of two merged drives                                                                                                                                                                                                                                                                                              | RAM ≤ 4 MiB + slots; merged counts exact                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Facets                                    | With concurrent writes; a pending removal of `E:\Photos` with a mirrored `D:\Photos` (facet counts equal the oracle with only E: masked)                                                                                                                                                                                                                                                 | RAM ≤ 70 MiB; DISK log                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Crops / changes_since                     | Crop list of N                                                                                                                                                                                                                                                                                                                                                                           | RAM ≤ 2 MiB                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Folder picker                             | 3×10^6 synthetic names                                                                                                                                                                                                                                                                                                                                                                   | RAM ≤ 20 MiB; order equals the oracle                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Qt/QML edge                               | `huge-list` and `huge-keys` assert dumps; window laws                                                                                                                                                                                                                                                                                                                                    | Exact strings; slots ≤ S                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Catalog rebuild                           | Forced rebuild at N and 10N; kill at random points                                                                                                                                                                                                                                                                                                                                       | RAM ≤ 2 × cache + 16 MiB + index/32768; DISK temp ≤ 2.2 × the largest index; no WAL; resume identical                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Post-freeze migration                     | Synthetic RebuildTable(files) and Shadow                                                                                                                                                                                                                                                                                                                                                 | DISK ≤ start + batch + delta; set_rating ≤ 150 ms during Shadow                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Startup                                   | Normal open; open needing a rebuild                                                                                                                                                                                                                                                                                                                                                      | open STEPS-1; window after 300 ms; cancel ≤ 100 ms                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| SQLite connections / temp / WAL           | Every scenario                                                                                                                                                                                                                                                                                                                                                                           | WAL; RTX; temp per owner; role caches sum to 884 MiB                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Whole process                             | All passes in one process, Defaults configuration, including a journal pass and the watch                                                                                                                                                                                                                                                                                                | ABS; RAM                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |

- **CLI variant:** a child under `run_in_job(limit)` for every subcommand, including `tier1 *`, `dups --distance`, `borders --csv`, `--import-review`, `thumbs`, `remove`, `status --wait`, `scan`, `scan --walk` and `journal status`.
- **Self-checks** (feature `self-check`):
  - an injected O(n) Vec fails the Rust flatness check;
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
just scale-smoke                       # 100k and 1M, Uniform and Defaults (also in Windows CI)
just scale                             # 1M and 10M, weekly
just usn-vhd                           # elevated: journal delete, recreate, wrap, rollback and clone; second-user and elevated mutators; volume lock
just perf --update
just scale-dhat <pass> <size>
```

**Oracles kept in-tree:**

| Area                   | Oracle                                                                                                                                                                  |
|------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Pairing                | `reference_pair` / `reference_mark_missing` / reference walk (floor = scan); `folder_id IN under_root!` for the root scope                                              |
| Journal rescan         | `gallery scan --walk` at the same checkpoints (A and B catalogs; all-J, mixed, resumed and abandoned schedules), on FakeVolume (with parent ambiguity) and on real NTFS |
| Work lists             | old pending SQL constants                                                                                                                                               |
| Upgrade                | `set_import_version` / `set_borders_version` / `set_taken_zone` bodies                                                                                                  |
| Status                 | `old_status`                                                                                                                                                            |
| Write-back             | old `NEXT_READY`                                                                                                                                                        |
| Rebuild                | `cfg(test)` `copy_tables`                                                                                                                                               |
| Thumbnails             | `OLD_FULL_SQL` and the old evict ORDER BY                                                                                                                               |
| PDQ                    | brute force, the old `Pairer` (two comparisons), and the old `pdq_hashes` session load                                                                                  |
| Dups                   | `oracle_groups` / `old_*_counts` / old EXISTS forms                                                                                                                     |
| Grid                   | `list_grid_reference`                                                                                                                                                   |
| Range-filter indexes   | the unindexed scan of the same query                                                                                                                                    |
| Tree                   | `reference_tree` / FOLDER_COUNTS                                                                                                                                        |
| Facets                 | per-facet statements                                                                                                                                                    |
| Borders CSV and review | old `image_files` / `report` / `import_review`                                                                                                                          |
| ListView               | golden table, reproduced by a real ListView                                                                                                                             |

---

## Critical files for implementation

- `D:/GitHub/photo-gallery/crates/catalog/src/schema.rs`
- `D:/GitHub/photo-gallery/crates/catalog/src/lib.rs`
- `D:/GitHub/photo-gallery/crates/catalog/src/pair.rs`
- `D:/GitHub/photo-gallery/crates/scan/src/journal.rs` (new)
- `D:/GitHub/photo-gallery/crates/thumbcache/src/lib.rs`
- `D:/GitHub/photo-gallery/crates/platform/src/usn.rs` (new)
- `D:/GitHub/photo-gallery/crates/catalog/src/usn.rs` (new)
- `D:/GitHub/photo-gallery/crates/library/src/runner.rs`
