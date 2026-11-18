# Scan step: optimization plan

Date: 2026-11-15, at HEAD `a8a7eac`, with the working tree's uncommitted work. Reevaluated 2026-11-17 at HEAD `b4515c7`:
see **Reevaluation** below, which says what changed and what it does to each phase.

The validation report's "shipped" numbers came from the main working tree at the time of its commit (`4d898d8`). The
report names no commit, and the scan code has moved since: `ScanWriter::new` now stamps `held_exts` on every batch
(`writers.rs:84`), and `Catalog::prepare` changed. So the report's figures are the reason for each phase, not its
baseline. Each phase is measured against its parent commit (see **Measurement protocol**).

Scope: the scan step of import (`pg_scan::scan`). The identity pairing at the end of a scan (`pair_after_scan`,
`pg_catalog::pair`) is out of scope, except where its queries read what this plan changes, or where it takes the write
lock on a scan that changed nothing. It is not the duplicate pairing pass of `PAIRING_PASS_PLAN.md`.

The brief is the most proper design, with churn free. Each item is the right fix, not the smallest one, and correct
first. Every design choice is decided here, with its reason. A measurement decides only two kinds of thing:

- tuning values (the worker count, the prefetch window);
- the gated items whose speed can't be known without a run: two in Phase 5, and the allocator. Their rule for adopting
  them is fixed here.

Line numbers are as of 2026-11-15, in a working tree that is still changing. Recount them before each phase.

**Review history.** The plan was validated adversarially on 2026-11-15:

- by an independent review of the whole plan;
- twice by three independent reviews of its parts: scan correctness, schema/writer/build, and evidence and citations
  (the second round also covered Phase 4 on its own).

Each finding was checked against the code or the SQLite source before it was taken in. The main results:

- the listing-order bug (Phase F) and the removal race (Phase F2), both in the shipped code;
- the hard-link gap in the first version of Phase F's fix;
- the Linux lister's per-walk state and the ordering of the parallel hand-off (Phase 4);
- the table rebuild this plan had added (a Phase 0) is not needed here: `last_scan` drops with `ALTER TABLE … DROP
  COLUMN`, and the column defaults it removed are harmless. The rebuild moves to the MFT plan, which needs one anyway;
- in the second round:
  - the writes after the walk's loop (the pairing, `mark_missing`) check no lease, so F2 covers every write of a walk;
  - Phase 4's identities on Linux, its admission rule, its stop path, its lease renewal and its device counts;
  - `RETURNING` on the new INSERT would bring the statement journal back.

The evidence is in `docs/temp/scan-optimizations-validation.md` (**the report**): callgrind runs on 4,000 and 200,000
files, the variant probe, the build-option builds, and warm, flushed and cold wall-clock runs on Windows. Where a figure
here comes from the probe, or wasn't measured, it says so.

## Reevaluation, 2026-11-17 (at HEAD `b4515c7`)

Thirty-seven commits landed since `a8a7eac`. The design holds. The status, the numbering, the baseline and several
premises changed, and each is taken in below. This is one pass against the code, not an adversarial validation.

**What changed under the plan:**

- **Phase F is in, and the perf baseline was re-measured with it.** `5aa9961` has F1 and F2. `f036825` re-measured
  `docs/perf/baseline.json` at `0d44547`, which contains F. Its scan entries are `scan_walk.first` 2,032,603,262 and
  `scan_walk.again` 341,125,554 (they were 1,961,442,750 and 364,267,992). That commit also carries the pairing and
  import changes, so F's own cost is not isolated, and it is not measured.
- **Migrations 14 and 15 belong to the pairing work:** `TO_14_PAIRING_EPOCH` (`meta.pairing_epoch`, with triggers on
  `analysis` only) and `TO_15_HASHES_OWED`. `MIGRATIONS` ends at 15, so this plan's migration is 16. The MFT plan's
  "reaches version 13" was stale; its text is fixed.
- **Memory statistics are off, by another route.** `bed9346` sets `LIBSQLITE3_FLAGS = "SQLITE_DEFAULT_MEMSTATUS=0"` in
  `.cargo/config.toml`. Phase 2 had rejected that route. See Phase 2.
- **`rarray` is loaded on every catalog connection** (`core.rs:240`, `register`), and `rusqlite`'s `array` feature is on
  (`Cargo.toml:39`). The precondition of Phase 5's items 6 and 8 holds for the catalog. The walk's workers don't have it.
- **A read-only connection opener exists:** `read_only(file, cache_kib)` (`core.rs:247`), which `Catalog::reader`
  (`core.rs:215`) uses. `Reader` still borrows the `Catalog`, so Phase 4's reason for a new owned type stands.
- **`HeldLease` moved** to `lease.rs`. `ScanLease::held()` is at `core.rs:413`.
- **The pairing's cost figures are stale.** "13.97G of 28.15G" was measured before the pairing commits.

**Verdict by part:**

| Part | Verdict | Reason |
|---|---|---|
| F1, F2 | Done; keep | The code and tests match "As built". The baseline includes F |
| 1 | Valid; migration 16 | `files_added` (`schema.rs:859`) is still the only INSERT trigger on `files`, so the plain INSERT is single-write once it goes. Change 3's `placeholder` flag belongs to an unbuilt design (`OPEN_ITEMS.md`, S-17.7), so that part is conditional |
| 2 | Revised | Memory statistics landed (`bed9346`). Lookaside is open. The route is revised (see Phase 2) |
| 3 | Valid; larger than written | `folder_rows` returns only `(volume, file_ref)` with `(size, mtime)` (`roots.rs:242`). `last_scan` is still read by `pair.rs`, and `seen_volumes` is still a whole-table scan (`pair.rs:307`), as written |
| 4 | Valid | `read_only` is the worker connection's opener. The `Reader` borrowing argument stands |
| 5 | Valid; catalog precondition met | `rarray` is loaded on the catalog's connections; the workers need it for items 6 and 8. Nothing calls `set_prepared_statement_cache_capacity`, so item 4 stands |
| Allocator | Valid | `mimalloc` is not in `Cargo.lock`, so the trial adds the crate |
| Not doing; D1, D5, D6 | Hold | D5's MFT text is updated (see that plan) |
| D2 | Decided; route to confirm | Open item 1 |
| D3 | Superseded | The scan gate is live (±1%). See D3 |

**Anchors (2026-11-17).** The body's line numbers are as of `a8a7eac`. Recount by the named symbol before each phase.
The main anchors now:

| Anchor | Now |
|---|---|
| `files_added` trigger; `files` table; `files_fresh` index | `schema.rs:859`; `schema.rs:653`; `schema.rs:372` |
| `SETTINGS` (`temp_store = MEMORY`) | `schema.rs:529` |
| `ScanWriter::new` (`stamp_held_exts` at line 91) | `writers.rs:84` |
| `ScanWriter::upsert_file`; `seen_identity` | `writers.rs:169`; `writers.rs:228` |
| `remove_root`; `begin_scan`; `mark_missing` | `roots.rs:91`; `roots.rs:124`; `roots.rs:170` |
| `folder_rows`; `prior_states` | `roots.rs:242`; `roots.rs:273` |
| `HeldLease::hold` | `lease.rs:26` |
| `ScanLease`; `held`; `renew` | `core.rs:383`; `core.rs:413`; `core.rs:408` |
| `Catalog::reader`; `register`; `read_only` | `core.rs:215`; `core.rs:240`; `core.rs:247` |
| `pair_after_scan`; `in_batches`; `seen_volumes`; `fresh_rows` | `pair.rs:85`; `pair.rs:283`; `pair.rs:307`; `pair.rs:320` |
| The stamp in each pairing batch | `pair.rs:298` |
| `stamp_held_exts`; `mark_unheld_missing` | `lifecycle.rs:215`; `lifecycle.rs:197` |
| Walk: `Listing`; `RENEW_EVERY`; `walk_leased` | `scan/src/lib.rs:83`; `:175`; `:458` |
| Walk: `folder_rows` and `prior_states` calls | `scan/src/lib.rs:508`; `:605` |
| Walk: `seen_identity` and `upsert_file` calls | `scan/src/lib.rs:719`; `:733` |
| Walk: `pair_after_scan` and `mark_missing` calls | `scan/src/lib.rs:624`; `:627` |
| `LIBSQLITE3_FLAGS` | `.cargo/config.toml:7` |
| `rusqlite` `array` feature; fork rev | `Cargo.toml:39`; `Cargo.toml:137` |

**Open for you:**

1. **Resolved (2026-11-17): the fork route.** Both flags are the fork's `photo-gallery-defaults` feature (`8ec47c7`
   on the pinned branch `photo-gallery-0.41.1`, pushed). `bed9346`'s `.cargo/config.toml` is removed.
2. **A conflict with the bounded-memory plan.** `docs/plans/BOUNDED_MEMORY_PLAN.md:167` would set
   `LIBSQLITE3_FLAGS = "SQLITE_TEMP_STORE=0 SQLITE_DEFAULT_MEMSTATUS=1"`. That reverts `bed9346`'s `MEMSTATUS=0`.
   Whichever lands must keep one value.
3. **The Linux container.** Confirm that the pg-dev run sees `.cargo/config.toml`. The Linux pass was not run here.

## Summary

| Phase | Change | Evidence |
|---|---|---|
| F | **Bug fixes, first.** F1: read a folder's rows before listing it, and keep the identity read safe for hard links. F2: removing a library folder ends the scans under it | **Done** (2026-11-15). Each bug's test failed before its fix, as the reading said |
| 1 | New files: a plain INSERT that sets `pending`, `seq` and `changed`; `files_added` dropped. Changed files: an UPDATE by id | Measured in the app: first scan −42% Ir; at 200k, −47% flushed-warm and −37% cold wall time, −55% user CPU (−51% user and kernel) |
| 2 | SQLite built with memory statistics off (**landed** in the fork, `8ec47c7`, as its `photo-gallery-defaults` feature) and lookaside `1200,200` (**landed**, the same feature) | −10% Ir (statistics, measured in the app); −51% of the fixed write on Windows (lookaside, probe) |
| 3 | A rescan that changes nothing writes nothing: unchanged files are decided at listing time and skip the batch; "seen by this scan" moves into a temporary set; `last_scan` and `files_fresh` go; the root check folds into the known lookup; `mark_missing` and the pairing take the write lock only when they have something to write | The work it removes cost 30.9k of a rescan's 41k Ir per file (`seen`, `known`, `in_root`: measured costs). The saving is not measured |
| 4 | Parallel listing in the walker: workers read each folder's rows and list it ahead of the walk, in walk order; the walk consumes in its own order; the worker count measured per kind of device | Up to 28% of a cold rescan, measured on the lister alone, unordered, flat, on one NVMe drive. The ordered design must be measured itself, nested and on the HDD too |
| 5 | The small items: folder ids, statement reuse, name buffers, hash set, ASCII extensions; two gated by a probe run (per-folder known lookup, batched identity fallback); the allocator, gated | 0.2 to 2% each (measured costs; item 11 not measured) |

Each phase is written in full, then built and tested once (no build between its steps). After each one, the
**Measurement protocol** is repeated and its results added to the report.

## How this plan meets the others

- **The NTFS MFT plan** (`docs/plans/NTFS_MFT_SCAN_PLAN.md`) rewrites `ScanWriter`, splits `files` into `files` and
  `links`, and replaces the walk with an MFT read where it can. The walker stays: it runs without elevation, for small
  folders, and on Linux. This plan lands first (D5), and the MFT plan inherits its rules:
  - no INSERT trigger on `files` or `links`;
  - the writer sets the change columns;
  - "seen by this scan" is a set the pass holds, not a column it writes;
  - a folder's rows are read before it is listed.

  The table-rebuild step this plan had as its Phase 0 moves to the MFT plan, with the review's corrections, since its
  `files`/`links` split rebuilds `files` anyway. The MFT plan says all of this: its Catalog section, its `ScanWriter`
  rules and its steps' preconditions (updated 2026-11-15).
- **Migrations.** The pairing work took migrations 13 to 15 (`TO_13_PAIRS_OWED`, `TO_14_PAIRING_EPOCH`,
  `TO_15_HASHES_OWED`). This plan's schema changes are one migration, the next free number (16).
  - Phase 1 adds it, and Phase 3 extends it in place.
  - No real catalog runs the builds in between. A scratch catalog made by one of them differs from the final schema at
    its version, so `Catalog::prepare` rebuilds it with its rows, as it rebuilds any mismatched catalog.
- **The pairing plan's `held_exts` stamp** (its Part 3, item 3) runs in every scan batch's transaction
  (`writers.rs:84`) and every pairing batch's (`pair.rs:288`), since those are the transactions that can insert or
  revive `files` rows. Phase 1 leaves it as it is.
  - After Phase 3, an unchanged file is never written, so it is never revived either (see Phase 3, **One outcome
    changes**). The stamp's premise, that a scan's writes revive rows, then holds only for the files the batch writes.
    That is the case its sequence needs.

## Phase F: bug fixes, first

Both are bugs in the shipped code, and both are independent of the optimizations. Later phases build on F1's order.

### F1. Read a folder's rows before listing it

**The bug.**

- **The order today.** The walk lists a folder (`scan/src/lib.rs:486`), and then reads its rows' prior states (`:570`,
  `Catalog::prior_states`). The batch later writes a file only while its row still equals that prior state (the guard,
  `writers.rs:118-119`).
- **How a write-back commits.** It sets `file_writes.temp` before it rewrites the file, then records the row's new size
  and mtime and clears `temp` in one transaction (`sql.rs`, the doc of `not_being_written!`; `queue.rs`,
  `file_rewritten`).
- **The race.** Suppose a file's entry is listed, then a write-back rewrites the file and commits, all before the
  folder's prior states are read. The prior is then the write-back's new row, the guard passes, and the batch writes
  the listing's old size and mtime over it.
  - The window runs from the entry's read to the end of its folder's listing. That can be seconds on a cold cache.
  - The write-back's analysis carry-over then no longer matches the row, so the file is imported again and its thumbnail
    remade. The next scan puts the row right.
  - No user data is lost, but the walk's documented rule (`scan/src/lib.rs:16-19`: "a write-back between the listing
    and the write is kept") is broken.
- **Why the tests pass.** The existing test (`a_write_back_between_the_listing_and_the_batch_is_kept…`, `:1276`)
  writes back while *another* folder is listed, after `f.jpg`'s prior state was already read.

**The fix.**

- **Read the folder's rows first,** then list it. That is `prior_states`' folder statement (`roots.rs:224`), in its own
  short read transaction. No snapshot is held across the listing.
  - A write-back that commits after that read fails the guard, since the row no longer equals the prior. The file is
    only marked seen, and the next scan lists the new state.
  - It opens no new window. A write-back that commits between the row read and the entry's read makes the listing
    equal the row.
- **The identity read for files that aren't in the folder's rows stays after the listing**, since it needs the listed
  identities. It is not safe by order alone:
  - The write-back opens the row's stored path (`write_target`, `writeback/src/lib.rs:470`) and rewrites the file in
    place.
  - For a hard-linked file, the old path still reaches the same file. So does a file moved and moved back within the
    window.
  - The walk takes the first path it lists for an identity (`scan/src/lib.rs:552`). If that is the second link, the
    file's prior comes from the identity read after the listing, and the bug is back for that file.
- **So the identity read is guarded by a change mark.**
  - The folder-row read also reads `meta.seq`: the mark, taken before the listing.
  - The identity read returns each row's `changed`.
  - A row whose `changed` is past the mark was changed after the listing began, so it is only marked seen; the next
    scan records it. Every write-back bumps `changed`, through `files_source_changed` on its size or mtime change
    (`schema.rs:829-841`).
  - The cost: an import or a rating of such a file in the window also bumps `changed`, so recording its move waits
    for the next scan. Nothing is lost.
**Done (2026-11-15).** As built:
- **`Catalog::folder_rows(folder) -> FolderRows`**: the folder's rows and the mark (`meta.seq`), in one snapshot. The
  walk calls it after the folder's open and before its listing.
- **`Catalog::prior_states(&FolderRows, identities)`**: the folder's rows first, then the others by identity in one
  snapshot after the listing. A row whose `changed` is past the mark is `None`, which the writer's guard already treats
  as "don't overwrite an existing row": the file is only marked seen.
- The module docs, `ScannedFile::prior`'s doc and the catalog tests' helper (`test_support::listed`) follow.
- **Tests:**
  - `a_write_back_while_its_own_folder_is_listed_is_kept` (the `after_listing("")` hook);
  - `a_write_back_through_another_name_of_a_file_is_kept` (the hard link, the hook after the second name's folder);
  - in `roots.rs`, `a_walk_changes_a_row_only_as_it_listed_it` covers the mark directly.

  Both walk tests were run before the fix and failed as the reading said: the batch wrote the listing over the
  write-back (`changed` 1).

### F2. Removing a library folder ends the scans under it

**The bug.**

- The app removes a folder (`Catalog::remove_root`, which marks its rows missing, `roots.rs:98`), and only then stops
  the scans under it (`app/src/library.rs:326-333`).
- A walk in progress writes its batch anyway, since a stop writes the batch built (`scan/src/lib.rs:633-635`). Its
  files are written as present under the removed folder.
- `gallery remove` (`cli/src/lib.rs:248`) never stops another process's walk, which goes on reviving everything it
  lists.

- **The writes after the loop aren't guarded either.**
  - The walk ends with its last batch, the pairing and `mark_missing` (`scan/src/lib.rs:585-591`).
  - Only the batch renews the lease (`:678`). An empty batch returns before it renews (`:672-674`).
  - Neither the pairing's `in_batches` (`pair.rs:284-291`) nor `mark_missing` (`roots.rs:150-185`) checks the lease.
  - A take-over sets `missing = 0` (`pair.rs:356`). The fresh rows are chosen by folder path, not through `roots`
    (`:314`).
  - So a removal during the pairing (a renumbered volume: every file fresh, rebound in batches of 1,000) revives the
    old rows under the removed folder, and a stop doesn't prevent it: a stopped walk still runs tier 1.
  - Once F2 frees the lease, a new walk of the folder can begin while the old one is still running. The old walk's
    `mark_missing` would then mark the new walk's rows missing.

**The fix.**

- **`remove_root` deletes the scan leases under the folder in the same transaction as its marks** (`roots.rs:90-101`).
  - A batch that committed between the two would otherwise still revive rows.
  - A lease is matched by `rules.is_under`, as `begin_scan` matches them (`roots.rs:121`), not by a byte range.
- **Every write transaction of a walk requires its lease.** Its first statement is the heartbeat UPDATE, which must hit
  a row; on a miss the transaction fails as a lost lease, in any process. This covers:
  - `ScanWriter::new` (today it ignores the count, `writers.rs:83`);
  - each batch of the pairing's `in_batches`;
  - each of `mark_missing`'s transactions.

  So every write of a walk is atomic with the lease check, the ones after the loop included.
- **`pair_after_scan` and `mark_missing` move onto `ScanLease`** and lose their `scan` argument. Today they are
  `Catalog` methods that take any scan id. On the lease, the id can't disagree with the lease, or with Phase 3's seen
  set.
- **The lost-lease error** (`scan/src/lib.rs:654-657`, "it expired while the walk was held up") and the module docs
  gain the other cause: the folder was removed.
- **The catalog's test helpers hold a lease.**
  - `test_support::begin` gives the lease back at once (`test_support.rs:66-70`: "several of one folder without ending
    them"). Then `scanned` writes through it (`:79`).
  - The pattern recurs: about 70 calls in `pair.rs`, `roots.rs`, `writers.rs`, `lifecycle.rs` and `core.rs`. Some tests
    begin many scans up front, for example 9 at `writers.rs:626`.
  - Under the new rule, all of these fail as a lost lease.

**Done (2026-11-15).** As built:

- **Two catalog errors** say why a lease is gone: `Error::RootRemoved`, when no library folder holds the walked folder
  any more, and `Error::LeaseLost` otherwise (it expired).
- **`HeldLease::hold`** (`core.rs`) is the heartbeat that must hit a row. It is the first statement of:
  - `ScanWriter::new`;
  - each batch of `in_batches`;
  - each of `mark_missing`'s transactions.

  `ScanLease::renew` uses it too, and now returns the error rather than `false`.
- **`ScanLease` keeps its root.** `pair_after_scan(coverage)` and `mark_missing(skipped)` are `ScanLease` methods with
  no scan or root argument.
- **`remove_root`** deletes the leases under the folder, matched by `rules.is_under`, in the transaction of its marks.
- **A new outcome, `ScanError::Removed`.** Without it, removing a folder while it is scanned would show the app's
  "scan failed" notice.
  - The walk maps `RootRemoved` to it.
  - The app's runner ignores it, as it does a refusal.
  - `gallery scan` reports it and fails.
- **The test helpers.**
  - `test_support::begin` writes the scan's row and its lease the way `begin_scan` does, without the overlap refusal,
    and keeps the lease.
  - `test_support::lease(c, scan, root)` gives a `ScanLease` that isn't released when dropped, to run the walk's end
    through.
  - So the ~70 call sites keep their shape. The lease's own rules stay tested through `begin_scan`.
- **Tests:**
  - `removing_a_folder_ends_the_walks_under_it` (`roots.rs`): the removed folder's walk and its subfolder's walk fail
    in a batch, the marks and a renewal. A sibling root's walk goes on. An expired lease is `LeaseLost`.
  - `a_pairing_after_its_folder_was_removed_writes_nothing` (`pair.rs`): a renumbered volume's fresh row isn't
    rebound, and no row comes back as present.
  - `a_folder_removed_while_it_is_walked_is_not_written_again` (scan): a removal from another connection after a
    folder's listing. The walk ends as `Removed`, and the files stay missing. Run before the fix, it failed: the walk
    completed and wrote the files back.
  - `a_folder_outside_the_library_is_not_recorded` now expects `RootRemoved` from the batch after a removal.
  - `pending_follows_what_the_analysis_was_made_from` wrote through a scan after its `mark_missing` had ended it. The
    file's return is now a later scan.

## Phase 1: the scan's writes of new and changed files

**Built, not tested (2026-11-17).** The changes below are in, as written, with these differences:

- The batch's change number is one per batch, taken by its first new row (`change_number`, `ScanWriter::bump`).
  The plan's design says the same. The writer's `record_file` doesn't return the new row's id yet: Phase 3 is the
  first to need it, so it is left for then.
- The fixture's insert takes one number per file, as the trigger it replaces did.
- The probe's trigger variants install the old `files_added` text (`FILES_ADDED`). Its upsert variants are still the
  pre-Phase-1 statements; the plan's before numbers come from a worktree build at the parent commit.
- Not run: the test build of `pg-catalog`, `pg-scan` and `pg-library` (stopped by the user). The measurement protocol
  and the scan benchmark's re-baseline are not done.

### Phase 1: why

On a first scan, each new file pays for two things together (the report, **Mechanisms**):

- the `files_added` trigger (`schema.rs:808`): a `meta.seq` bump and an UPDATE of the row just inserted;
- a statement journal for every INSERT. The upsert is may-abort (NOT NULL columns), and multi-write for two separate
  reasons: the trigger, and its DO UPDATE clause.

Measured in a patched copy of the app (the report, **The fix in the app**, **The fix on Windows**):

| | Shipped | Patched |
|---|---:|---:|
| First scan, 4k, callgrind | 895.0M Ir | 506.9M Ir (−43.4%) |
| First scan, 200k, callgrind | 48.25G Ir | 28.15G Ir (−41.7%) |
| First scan, 200k, Windows, flushed warm | 13.09 s | 6.88 s (−47%) |
| First scan, 200k, Windows, cold | 15.26 s | 9.55 s (−37%) |
| First scan, 200k, user CPU | 10.81 s | 4.84 s (−55%) |
| First scan, 200k, user and kernel CPU, flushed warm | 11.77 s | 5.81 s (−51%) |

The cold row's medians pool two scripts' runs, 5 from `win_cold.ps1` and 5 from `win_cpu.ps1`, whose patched medians
differ by about 0.5 s (9.67 and 9.15 s). The −37% holds in each set alone.

### Phase 1: design

- **The writer says what it means.** The `known` lookup runs in the batch's own write transaction, so the writer
  already knows whether the row exists. An upsert's ON CONFLICT decides nothing it doesn't already know. So:
  - **a new file** (no hit, or the new row after a displace) gets a plain INSERT;
  - **a changed file** gets an UPDATE of the known row by its id, with the guard the DO UPDATE has today (size and mtime
    as listed, and `not_being_written!`, `writers.rs:118-119`), and the same columns, the `coalesce(created)` rule
    included. The guard returns 0 rows when it fails, as now.
  - After a displace, the old row's identity is NULL (`writers.rs:103`), so the INSERT can't conflict either.
  - Checked against the SQLite source: the plain INSERT has no INSERT trigger left, so nothing makes it multi-write,
    and it opens no statement journal (`sqlite3.c:140344`, `:90179`). Its foreign keys to `folders` and `scans` are
    checked at once (`:138281-138291`).
  - **`new_file` takes the new row's id from `last_insert_rowid()`, never `RETURNING`.** Phase 3 needs the id for the
    seen set.
    - SQLite makes a `RETURNING` clause a trigger in the table's trigger list (`sqlite3.c:158942-158958`). That sets
      `pTrigger`, which makes the INSERT multi-write (`:140344`), and with NOT NULL's may-abort that is a statement
      journal again (`:90179`).
    - The probe's plain INSERT never fetched an id (`stmt_journal_probe.rs:351-355`), so it didn't measure this.
    - The folder insert keeps its `RETURNING id` (`writers.rs:92`): it runs once per new folder.
- **The writer sets what the trigger set.**
  - `pending` = 1: the trigger's `stale!()` (`sql.rs:8`) is true for a new row, since an analysis can't outlive its file
    (`analysis.file_id … ON DELETE CASCADE`).
  - `seq` = `changed` = one change number per batch: `UPDATE meta SET value = value + 1 WHERE key = 'seq' RETURNING
    value`, run by the batch's first new file and kept in the writer (one `ScanWriter` per batch,
    `scan/src/lib.rs:682`).
- **One number per batch is safe, and the effect is small.**
  - A batch's rows become visible in one commit. The readers compare numbers, never count them: the thumbnail pass
    reads everything past its checkpoint (`thumbcache/src/reader.rs:30`), and the ledger conversion breaks ties by id
    (`convert.rs:276`).
  - A new row's number is replaced anyway when its analysis is stored: `analysis_added` stamps `seq` and `changed` again
    (`schema.rs:843-854`). The thumbnail sources list only analysed files. So the batch number never reaches the
    thumbnail pass.
  - It matters for `changed`, a client's resync, which can list a row before its analysis.
- **The column defaults stay.** `pending`'s default of 1 is right for any new row. A row inserted without `seq` and
  `changed` gets 0 until its analysis stamps both. Removing the defaults would need a rebuild of `files` with foreign
  keys off, the riskiest kind of migration, for a guard against a mistake whose worst effect is a late resync stamp.
  The MFT plan's rebuild of `files` can revisit this.
- **The UPDATE triggers stay.**
  - `files_changed`, `files_source_changed`, `files_failures_stale` and `files_failures_hidden` have many writers
    (write-back, rotation, `mark_missing`, the pairing's take-overs), so the schema's rule for them is right: "kept in
    the statements that change what they depend on, whoever writes them" (`schema.rs:806`).
  - Changed files are few, and an UPDATE of them still opens a journal through those triggers (variant j: nothing
    saved on Windows). That cost is accepted.

### Phase 1: changes

1. **The migration** (`schema.rs`, the next free version): `DROP TRIGGER files_added;`.
   - Its doc comment says the rule: whoever inserts a `files` row sets `pending`, `seq` and `changed`.
   - The baseline stays frozen (`schema.rs:43`).
2. **`ScanWriter`** (`writers.rs`):
   - `file` (the upsert, `:110-120`) becomes two statements, `new_file` (the INSERT) and `changed_file` (the guarded
     UPDATE by id).
   - `write_file` (`:31`) splits to match.
   - The `upsert_file` branches at `:189` and `:202` call `new_file`, and `:212` calls `changed_file`.
   - `upsert_file` is renamed `record_file`, since nothing upserts now. Its caller (`scan/src/lib.rs:701`) and the doc
     comments follow.
3. **The fixture** (`fixture.rs:101-119`): `insert` takes one `meta.seq` number per file and sets `pending` = 1, `seq`
   and `changed`, as the trigger did. Other crates' tests that use the fixture keep their behaviour.
4. **Raw inserts in tests at the current version** (`ledger.rs:1053`, `pair.rs:538`, `roots.rs:715`): set the three
   columns where the test reads them.
   - Inserts at older versions keep their layout: `lifecycle.rs:1276`, `:1493`, `schema.rs:1064`, `:1102`, and
     `lifecycle.rs:391` (a positional insert on a layout from before the baseline).
5. **Trigger tests** (`schema.rs`):
   - Remove `files_added` from `TRIGGER_TESTS` (`:928`).
   - Replace `a_new_file_is_pending_and_takes_the_next_seq` (`:1309`) with writer tests:
     - a scan's new files are pending and unpaired, and share one `seq` and `changed`, one past the `meta.seq` before;
     - a second batch takes the next number;
     - a displaced file's new row gets the same values. The displaced row takes a new number from
       `files_source_changed`, since its `missing` changes (`schema.rs:829`), as today;
     - a changed file's guarded UPDATE writes nothing when the row changed since the listing.
   - The every-version migration test (`lifecycle.rs:1422`, `every_older_version_reaches_the_current_one_on_both_paths`)
     covers the new version. Assert that it has no `files_added`.
6. **The variant probe** (`crates/catalog/examples/stmt_journal_probe.rs`):
   - The "before" numbers come from a worktree build at the phase's parent commit (see **Measurement protocol**), not
     from the probe imitating the old schema.
   - In the tree, the probe follows the new schema. Its variants that measure the trigger make it from a copy of its
     SQL; they drop it at `:262` and `:264` today.
7. **Other documents:**
   - `NOTIFICATIONS_PLAN.md`, row N-13: its open part is a test of `files_added`. It becomes the writer tests of
     step 5.
   - `OPEN_ITEMS.md`, the placeholder design: its `placeholder` flag is written by `new_file` and by `changed_file`, and
     is one of Phase 3's compared columns (conditional: the placeholder design is not built, `OPEN_ITEMS.md`, S-17.7).
     Otherwise a placeholder hydrated back with the same size and mtime would be
     classified unchanged, and its flag would never clear.
   - `NOTIFICATIONS_PLAN.md`, N-13 has a second open part, the guard in `carried_is_every_table_of_the_current_schema`.
     This plan doesn't close it.

## Phase 2: SQLite build flags

### Phase 2: status (2026-11-17)

- **Memory statistics: landed** in `bed9346`, through `.cargo/config.toml`. The commit measured the flag on the pairing
  load (cycles 2,122 to 1,308 M at 4 and 12 threads) and found it level on its own elsewhere. The scan's −10% was
  measured with the same flag, so that part is in.
- **Lookaside: landed** with memory statistics, through the fork's `photo-gallery-defaults` feature (`8ec47c7`). The
  probe's numbers stand (`1200,200`, −51% of the fixed write on Windows).
- **The route is the fork's, by the rule that dependency changes go there.** `bed9346` had put the memory-statistics
  flag in `.cargo/config.toml`, where Cargo finds it from the working directory only, so a build started elsewhere lost
  it silently. That file is removed. The compile-options test (change 2, in `crates/catalog/src/core.rs`) checks both
  flags in the built SQLite.

### Phase 2: why

- **Memory statistics.** With them on (the default), every `sqlite3_malloc` and `sqlite3_free` takes a global mutex
  (`sqlite3.c:32126`). Off, the scan costs 10.1% fewer instructions (the report, item 18).
  - Nothing reads them: no code in `crates/` or the fork calls `sqlite3_memory_used`, `sqlite3_status` or a heap limit.
  - `release_memory` (`app/src/reader.rs:182`) doesn't read them either.
- **Lookaside.** The default (`1200,40`: 30 slots of 1,200 bytes and 93 of 128) runs full.
  - In the probe's app-like write (variant a), there were 157,281 full-pool misses against 40,435 hits.
  - At `1200,200` the misses go to zero. That setting is a 240,000-byte budget, which SQLite splits into 151 slots of
    1,200 bytes and 459 of 128 (`sqlite3.c:189184-189190`).
  - There, the folded write (variant c) drops 12% in instructions and 51% on Windows (44.4 → 21.6 ms).
  - `1200,500` and `1200,1000` gave the same as `1200,200`.
- **Decided (D2): every connection.**
  - It costs 192 KB more per connection, small next to each catalog connection's 256 MB page cache (`schema.rs:479`).
  - It needs no `unsafe` call.

### Phase 2: design

- **The defines go in the fork's `libsqlite3-sys/build.rs`, behind a Cargo feature** (for example
  `photo-gallery-defaults`).
  - The workspace depends on `rusqlite`, which passes features on to `libsqlite3-sys` (the fork's `Cargo.toml:53`:
    `bundled = ["libsqlite3-sys?/bundled", …]`).
  - So the fork's `rusqlite` gets a forwarding feature too, `photo-gallery-defaults =
    ["libsqlite3-sys?/photo-gallery-defaults"]`. The workspace enables it where it names `rusqlite`.
- **Why not `LIBSQLITE3_FLAGS`.**
  - The bundled build already reads that variable (`build.rs:340-351`), and setting it in a `.cargo/config.toml`
    `[env]` would work.
  - But Cargo finds that config from the working directory of each invocation, not from the manifest. A build started
    elsewhere (`--manifest-path`, an IDE, a tool) would silently lose the flags.
  - A feature is part of the dependency graph, so it reaches every build.
- The fork already carries this workspace's patches (the statement cache that doesn't copy the SQL, `Cargo.toml:124`),
  so one more is in keeping.

### Phase 2: changes

1. **Done (2026-11-17):** the fork's `photo-gallery-defaults` feature (`libsqlite3-sys`, forwarded by `rusqlite`) sets
   both flags for the bundled build, before `LIBSQLITE3_FLAGS`, which can still override them. The workspace takes
   the rev `8ec47c7`. Built and checked: the fork's `libsqlite3.a` has both options.
   **The fork feature, as built:**
   - the feature in `libsqlite3-sys`, adding `-DSQLITE_DEFAULT_MEMSTATUS=0` and `-DSQLITE_DEFAULT_LOOKASIDE=1200,200`
     to the bundled build;
   - the forwarding feature in `rusqlite`;
   - a note in its README.

   Then bump the `rev` in the workspace `Cargo.toml` (`:137`) and enable the feature.
2. **A test** in `crates/catalog`: `PRAGMA compile_options` lists `DEFAULT_MEMSTATUS=0` and `DEFAULT_LOOKASIDE=1200,200`
   (`sqlite3.c:23438-23445`).
3. **The variant probe's lookaside and statistics variants** set their values explicitly, through
   `sqlite3_db_config(SQLITE_DBCONFIG_LOOKASIDE, …)` and `sqlite3_config(SQLITE_CONFIG_MEMSTATUS, …)`. Once the
   defaults change, they would otherwise no longer measure what they did.
4. **Delete the experiment's copy of the fork** (`target/scan-probe/fork`, with its `PG_SQLITE_FLAGS` hook), once
   Phase 2 is measured.

## Phase 3: a rescan that changes nothing writes nothing

**Built, not tested (2026-11-17).** The changes below are in, with these differences from the text:

- **The set's reads use `NOT EXISTS` and `EXISTS`, correlated on `files.id`, not `NOT IN (SELECT id FROM temp.scan_seen)`.**
  A `NOT IN` over the set is materialised again for each statement a batch runs, which is quadratic over the batches.
  The shared fragments are `not_seen!()` and the correlated lookup in `sql.rs`.
- **Fresh rows come from the set's unpaired partial index,** joined to `files` (`fresh_rows`), with no `INDEXED BY`.
  The probe with `LIMIT 1` runs on the catalog connection before the write transaction opens (`in_batches`).
- **`seen_volumes` reads the set joined to `files`,** not a collection from the walk. It reads the set's rows, not the table.
- **The folder read keeps every compared column** (`FolderRow`), and `FolderRows::unchanged` makes the listing-time
  decision. It compares the row as `record_file` does, including the creation-time rules, so an unchanged file is one
  `record_file` would only mark seen.
- **Walk:** unchanged files are marked seen in one deferred transaction per folder (`ScanLease::mark_seen`). Only the rest
  go to the batch, with their priors.
- **`known` returns `unpaired` and `in_root`,** the folder's membership, and `record_file` marks the row it writes or
  keeps. A new row's id comes from `last_insert_rowid()`, as the plan says.
- **Migration 16 is extended in place** (`DROP INDEX files_fresh`, `DROP COLUMN last_scan`), as the plan says for a
  migration no real catalog has run.
- **The set is emptied** when a walk begins (`begin_scan`), in `mark_missing`'s last transaction, and when the lease is
  released (`release_lease`, so on drop too).
- **`mark_missing`** reads the unseen present rows outside a write transaction, then marks them by id in batches of
  `WRITE_BATCH`, repeating the predicate in each write.
- **Tests:** `the_folder_read_recognises_a_listing_that_matches_its_row` and `a_lease_given_back_empties_the_walks_set`
  (roots), and the writer tests from Phase 1. Not written: the plan's test list in change 6 beyond these (the
  two-transaction count with `WriteHolds`, the temp-only transaction against a held catalog, the Busy begin, and the
  pairing and mark-missing cases with unpaired children and refused mounts). Existing tests were updated for `last_scan`'s
  removal and the set (raw inserts, `raw::file_row`, the pairing's batch test, which fills the set).
- **The probe** (`stmt_journal_probe.rs`) still names `last_scan` and `files_fresh` in its own statements. It no longer
  runs against the current schema, and the before numbers come from a worktree at the parent commit, as Phase 1 says.

### Phase 3: why

A 200k rescan costs 41k instructions per file. `write_batch` is 32k of that:

- the `seen` UPDATE, 14.8k;
- the `known` lookup, 11.5k;
- the `in_root` query, 4.6k.

(The report, **The fix in the app**.)

- **The `seen` UPDATE** (`writers.rs:100`) writes `last_scan` into every row of the root on every rescan.
  - So every leaf page of `files` under the root is rewritten into the WAL and checkpointed back, for a scan that found
    nothing.
  - Each write also checks a foreign key and opens a statement journal.
  - The schema's own rule is the opposite: "The WHEN clauses keep a rescan that changes nothing from writing anything"
    (`schema.rs:806-807`).
- **`last_scan` has two jobs, and both last only as long as the scan:**
  - `mark_missing`: `last_scan < ?scan` (`roots.rs:167`);
  - the identity pairing: fresh rows `last_scan = ?scan` (`pair.rs:314`), candidates `last_scan < ?scan` (`:100`,
    `:136`, `:198-199`), and the volumes seen (`:297-298`).

  Both run on the scan's own connection, after its batches. Nothing reads `last_scan` once the scan is over.
- **The `in_root` query** (`writers.rs:97-99`, `:195-199`) looks up the row's folder after the `known` lookup has
  already found the row.
- **Even with nothing to write, a rescan takes the write lock several times.**
  - `mark_missing` takes it for every batch of folders (`roots.rs:162-176`).
  - The pairing's `in_batches` opens its write transaction before it reads the fresh rows (`pair.rs:284-286`): once on a
    stopped scan, three times on a complete one.

### Phase 3: design

- **An unchanged file is decided at listing time and never enters the batch.**
  - **The double read.** The folder-row read (Phase F1, just before the listing) already reads every row of the folder
    in one statement. The write transaction then looks each file up again by identity (`known`, 11.5k Ir per file on a
    200k rescan, 2.3G in all).
  - **Unchanged** means the row was present, with the listing's folder, name, extension, kind, size, mtime and order
    key, and creation times that call for neither a displace nor a backfill.
  - **Today such a file ends as `Same` in every case but one** (below):
    - a write-back that changed the row fails the size and mtime guard, and the file is only marked seen;
    - a rotation or an import changes no compared column;
    - nothing outside the scan writer and the pairing's `take_identity` writes `name` or `folder_id`, and another
      root's pairing stays within its own root;
    - no other walk touches the row, since the lease keeps overlapping walks apart (and after F2, every write of a walk
      checks it).
  - **So the folder-row read decides.** It returns each row's id, `unpaired` and every compared column.
    - The walk records an unchanged file in the seen set, counts it `Same`, and doesn't add it to the batch.
    - Everything else goes through the batch as now: new files, changed ones, moved ones (found by the identity read),
      displaced ones, backfills, and rows of other roots.
    - Skipping changes no stats, ledger rows or progress, and the lease's renewal is time-based at checkpoints.
  - **The seen rows are written outside the catalog's write lock.**
    - The set is in the temporary database. The walk writes a folder's unchanged rows after its listing, in one
      explicitly **DEFERRED** transaction per folder that touches only `temp`.
    - It must not use `write_transaction` or `Catalog::tx`, which are `BEGIN IMMEDIATE` (`transaction.rs:14`):
      `BEGIN IMMEDIATE` starts a write transaction on every attached database, main included (`sqlite3.c:132121-132135`).
  - **With Phase 4**, workers do the folder-row read on their own connections. The set lives on the walk's connection,
    so the workers return the rows and the walk writes them.
- **One outcome changes: a row marked missing between the listing and the batch.**
  - Today the batch's write sets `missing = 0` again (`writers.rs:116`), since the guard passes. Now the row stays
    missing until this root's next scan, which finds it missing at listing time and writes it.
  - By `remove_root`, this no longer arises: its lease delete makes the walk's next write fail (Phase F2), so nothing
    of the walk is written after it.
  - **Phase F2 matters more after Phase 3.** Today the last batch is rarely empty, so its renewal is, in practice, a
    lease check before the pairing. After Phase 3, an unchanged rescan's last batch is empty, and that check is gone. F2's
    check in the pairing's and `mark_missing`'s own transactions is what holds then.
  - By `mark_unheld_missing` (`lifecycle.rs:197`, called from `prepare` at `core.rs:127`) in another build, it is a
    delay. A build that holds this extension stops reviving it until its next scan. That is accepted: it needs two
    builds with different extension sets running at once, and the pairing plan's `held_exts` stamp handles that case.
- **"Seen by this scan" is a set the scan holds:** `temp.scan_seen(id INTEGER PRIMARY KEY, unpaired INTEGER NOT NULL)`,
  with a temporary partial index on the unpaired ones (`CREATE INDEX temp.scan_seen_unpaired ON scan_seen(id) WHERE
  unpaired = 1`).
  - **What it holds:**
    - new rows, changed rows and unchanged ones, each with whether it is unpaired;
    - the rows a reparse file kept (`seen_identity`);
    - not a row of another root (`Recorded::Foreign`), and not a displaced row. Today neither is marked seen.
  - **Its lifetime.**
    - It is made by the first scan on a connection (`CREATE TEMP TABLE IF NOT EXISTS`), with its own statement in
      `begin_scan` before the lease's transaction. Not inside that transaction: when `begin_scan` is refused as Busy, the
      rollback would undo a schema change, which resets every schema and expires every statement on the connection
      (`sqlite3.c:189881-189897`), on each retry.
    - It is never dropped. A change to the temporary schema expires every prepared statement on the connection
      (`sqlite3.c:100701-100705`), so it happens once per connection.
    - It is emptied when a scan begins, and on both ways a lease ends: in the transaction where a complete walk deletes
      its lease (`roots.rs:183`), and on the lease's release when it is dropped.
    - The set belongs to the lease. The readers are `ScanLease` methods (Phase F2), so no caller can pass a scan id the
      set isn't about.
    - With `temp_store = MEMORY` (`schema.rs:479`), the pages an emptied set freed stay with the connection: a few MB
      after a 200k scan. That is accepted, since the next scan reuses them.
- **The set holds the rows `last_scan = ?scan` selects today, with two small differences,** and every reader's
  predicate maps one to one:
  - `last_scan = ?scan` becomes `id IN temp.scan_seen`;
  - `last_scan < ?scan` becomes `id NOT IN temp.scan_seen`.

  A stopped scan's pairing still runs on what was seen. A scan that fails never reads the set.
  - **The differences.**
    - Today `seen_identity` (`writers.rs:101`) stamps whichever row has the identity, in any root, and the guard-fail
      path (`:212-213`) can stamp a missing row of another root. The set records only what this plan lists under
      **What it holds**.
    - Today `last_scan < ?scan` leaves out a row stamped by a later scan of a folder that doesn't overlap. `NOT IN` the
      set keeps it.
  - **Neither changes an outcome.** Every reader is limited to the root's folders, and while the lease holds, no other
    scan writes the root's rows.
  - **Fresh rows** are the set's unpaired ones, through its partial index, joined to `files` with `unpaired = 1` checked
    again. That is exact and bounded by the scan.
    - A partial index on `files` would not be: unpaired rows with children are never paired or cleared (`fresh_rows`
      leaves them out, `pair.rs:316-317`), and it would hold every other root's unpaired rows too.
  - **`rebind` and `replace`** delete a fresh row (`pair.rs:344`, `:433`). They delete its id from the set in the same
    transaction. That is not strictly needed, since the remaining readers are limited to the root, but it keeps the set
    exact.
  - **`seen_volumes` comes from the walk**, as the volumes of the rows recorded as seen: not of `Foreign` hits, reparse
    files only when they have a row, and no volume that held only folders.
    - It is collected from `write_batch`'s results and from the unchanged rows as they are classified.
    - Today it is a scan of the whole table (`pair.rs:297-298`); through the set it would be 200,000 lookups.
- **`last_scan` goes, with `ALTER TABLE files DROP COLUMN last_scan`,** in the same migration as Phase 1, after
  `DROP INDEX files_fresh`.
  - SQLite's DROP COLUMN refuses PRIMARY KEY and UNIQUE columns outright (`sqlite3.c:123030`). Its schema check after
    the drop (`:123065`) also refuses a column that an index, a partial index's WHERE, a CHECK, a generated column, a
    table-level FOREIGN KEY, a trigger or a view still names. `last_scan` is none of these once `files_fresh` is gone.
  - It removes the column's whole definition, its `REFERENCES scans(id)` included (`:122953-122957`). It runs with
    foreign keys on, as TO_8, TO_12 and TO_13 already do (`schema.rs:358`, `:411`, `:447-448`).
  - Its foreign key to `scans` goes with it, which also lightens the changed-file UPDATE.
  - No trigger or view names `last_scan`; only `files_fresh` indexes it (`schema.rs:370`).
  - `scans` itself stays: the scans are recorded (`roots.rs:125` inserts them), and `roots.rs:196` reads them.
- **The root check folds into the known lookup.** `known` joins the row's folder by its primary key and returns whether
  that folder is under the root:

  ```sql
  SELECT f.id, …, f.unpaired, (d.path = ?3 OR (d.path >= ?4 AND d.path < ?5))
  FROM files f JOIN folders d ON d.id = f.folder_id
  WHERE f.volume = ?1 AND f.file_ref = ?2
  ```

  One lookup answers both questions, for the files the batch still writes. The `in_root` statement goes; today it runs
  for every present hit (`writers.rs:195-197`).
- **`mark_missing` and the pairing take the write lock only when they have something to write.**
  - **`mark_missing`** reads the root's unseen present rows outside a write transaction. They are rows whose folder is
    under the root and not under a skipped folder, as `folders_under` minus `skipped_folders` gives them today
    (`roots.rs:153-158`), with `missing = 0` and `id NOT IN temp.scan_seen`.
    - The skipped folders stay out: an unreadable folder, a refused mount or a reparse folder must keep its rows
      (`roots.rs:140-143`).
    - Only when there are some does it update them by id, in batches of up to 1,000 as now. The UPDATE repeats the
      whole predicate of the read: the folder in the same list, `missing = 0`, not in the set. A row can otherwise be
      removed and moved back as present by another root's batch in between.
    - Its last transaction, the folder-ledger sweep and the lease delete, stays a write. That is one per complete scan.
  - **The pairing's `in_batches`** (`pair.rs:284-286`) first runs its fresh-row read, `fresh_rows`, outside a write
    transaction, with its full predicate: an identity, under the root, no children, not in a skipped folder, `LIMIT 1`.
    - Only when that finds a row does it open its write transaction, and it reads the batch inside it as now.
    - A weaker check, "the set has an unpaired row", would not do. Unpaired rows with children are never cleared
      (`pair.rs:316-317`), so on a library that has any, every rescan would still take the write lock once per tier.

### Phase 3: changes

0. **`folder_rows` and `prior_states`** (Phase F1's split, `roots.rs`):
   - Today the statement returns only `volume`, `file_ref`, `size` and `mtime` (`roots.rs:242`). So this change is most
     of Phase 3's code, not a detail.
   - The folder statement returns `id`, `volume`, `file_ref`, `name`, `ext`, `kind`, `size`, `mtime`, `missing`,
     `order_key`, `created` and `unpaired`, with the change mark. `volume` and `file_ref` match the listed identities, as
     today (`roots.rs:225`).
   - It returns a classification per file: `Unchanged { id, unpaired }` or `Write(prior)`. The comparison is the one
     `upsert_file` makes today (`writers.rs:171-178`) and its creation-time rules (`:200`, `:205`), moved into one
     shared function that both use.
   - The walk (`scan/src/lib.rs:566-577`) records `Unchanged` rows in the set, in one DEFERRED temp-only transaction
     per folder, and adds only `Write` files to the batch.
1. **The migration** (Phase 1's, extended in place): `DROP INDEX files_fresh;` and then
   `ALTER TABLE files DROP COLUMN last_scan;`.
   - `last_scan` also leaves:
     - the fixture's insert (`fixture.rs:102`);
     - the raw inserts at the current version that Phase 1 lists (`ledger.rs:1053`, `pair.rs:538`, `roots.rs:715`);
     - the variant probe (`stmt_journal_probe.rs:35`, `:38`, `:48`, `:51`).
   - Some of those inserts set `last_scan = scan` to make rows seen. For example,
     `the_pairing_writes_in_short_batches` (`pair.rs:538`) makes 2,500 fresh rows this way. A fixture helper that fills
     `temp.scan_seen` on the lease's connection takes its place.
2. **`ScanWriter`** (`writers.rs`):
   - remove `seen`, `seen_identity` and `in_root`;
   - add `insert_seen` (`INSERT OR IGNORE INTO temp.scan_seen(id, unpaired) VALUES (?1, ?2)`);
   - `known` joins `folders` as above, and returns `unpaired`;
   - the INSERT and UPDATE of Phase 1 lose `last_scan`;
   - `seen_identity` becomes a lookup of the identity's id and `unpaired`, then `insert_seen`.
3. **`temp.scan_seen` and its index:** made, if not there yet, by its own statements in `begin_scan` before the lease's
   transaction. Emptied in `begin_scan`, in `mark_missing`'s last transaction next to its lease delete (`roots.rs:183`),
   and in the lease's release on `Drop`.
4. **`mark_missing`** (`roots.rs:150-185`, a `ScanLease` method after F2): the read of unseen rows outside a write
   transaction, with the skipped folders left out, then the UPDATE by id with the whole predicate repeated, only when
   there are some. The folder ledger takes the lease's scan id.
5. **The pairing:**
   - its queries (`pair.rs:100`, `:136`, `:198-199`, `:314`) follow the mapping above;
   - `seen_volumes` (`:297-298`) takes the walk's volumes instead of reading;
   - the take-over that sets `last_scan` (`:356`) records the row in the set instead;
   - `rebind` and `replace` (`:344`, `:433`) delete the fresh row's id from the set;
   - the fresh-row read uses the set's unpaired index. Check it with `EXPLAIN QUERY PLAN` in a test, as the other plan
     tests do (`raw::query_plan`);
   - `in_batches` runs `fresh_rows` with `LIMIT 1` before it opens its write transaction;
   - the docs that explain "seen" or `files_fresh` (`:14`, `:23-24`, `:142`, `:190`, `:296`, `:302`, `:796`) are
     rewritten.
6. **Tests:**
   - The existing rescan, `mark_missing` and pairing tests carry the behaviour. Each that reads `last_scan`
     (`fixture/raw.rs:95`) reads the set or the outcome instead.
   - New:
     - a rescan of an unchanged root changes no `files` row: a dump of the table is identical before and after. Its WAL
       grows by at most a small, fixed number of frames, independent of the number of files. The WAL can't be split by
       table without `dbstat`, and page 1 and freelist pages are written too;
     - a rescan of an unchanged root takes exactly two write transactions, `begin_scan`'s and the last one of
       `mark_missing`. Count them with `WriteHolds` (`transaction.rs:4`, through `crate::fixture::WriteHolds`).
       - It counts only `write_transaction`'s (`transaction.rs:13-16`). The lease's renewals, `finish_scan` and the
         release are autocommit writes and aren't in it.
     - the per-folder temporary transactions don't lock `main`: from a `FakeListing` hook during one of them, another
       connection's `BEGIN IMMEDIATE` succeeds at once;
     - a file whose row changed after the listing is still seen, and not marked missing;
     - an unchanged row that is still unpaired from an earlier stopped scan is a fresh row of this one, as today;
     - a stopped scan's set is empty after the lease drops, and a complete scan's after `mark_missing`;
     - a second scan starts with an empty set;
     - a `begin_scan` refused as Busy leaves the temporary schema unchanged;
     - a fresh row that the pairing rebinds or replaces leaves the set;
     - a rescan of a root that holds an unpaired row with children takes no pairing write transaction;
     - a complete rescan with an unreadable folder and a refused mount leaves their rows present.

## Phase 4: parallel listing in the walker

### Phase 4: why

Warm, listing is 0.3% of a first scan. Cold, listing 200,000 files takes 1.09 s on one thread: 7% of a cold first scan,
and a third of a cold rescan (3.29 s). Eight threads list the same folders cold in 0.16 s (the report, **The Windows
lister**).

- That figure comes from an unordered lister, `list_probe`, which splits the folders round-robin. The walk's ordered
  hand-off is a different shape, and must be measured itself.
- The walker keeps running where the MFT read doesn't: without elevation, for small folders, and on Linux.

### Phase 4: the constraints

- **Rows, then listing, then the identity read: one unit, in that order** (Phase F1). A worker must not list ahead and
  read the rows later.
- **The walk's state depends on its order:**
  - the identities seen (the second path to a file is the one skipped);
  - the loop chain;
  - the mount verdicts;
  - the skipped folders;
  - the folder ledger.
- **On Linux, the lister's volume identities depend on its order too.**
  - The Linux `Lister` (`platform/src/dir.rs:431-475`) keeps three things: a map from device to identity, using the
    first entry seen on that device as the probe; `claimed`, which stops two cloned disks with one UUID both taking
    it; and a count of fallbacks.
  - One lister per worker would let two clones each claim the UUID, so one clone's files would match the other's
    rows. A device could also get different identities in two workers, and the fallback count would be summed.
  - The Windows lister is stateless (`dir.rs:316-342`).

### Phase 4: design

- **The walk keeps the volume identities, and gives each folder its own.**
  - Rows are keyed by the mapped identity: the folder-row read matches on `(volume, file_ref)` (`roots.rs:225-237`), and
    the walk builds identities from the mapped volume (`scan/src/lib.rs:215`, `:501`). A worker with only raw device
    numbers could match no row. Every file on a Linux volume with a UUID would get no prior, fail the guard, and be
    recorded as `Same`, so no change there would ever be written.
  - So each folder is handed out with its device's identity. The walk always has it: it mapped the folder's own entry
    when it consumed the parent, and the root's comes from `folder_volume`.
  - The worker uses that identity for the entries on the folder's device, which is nearly all of them.
  - An entry on another device (a mount-point folder, or a bind-mounted file) is reported with its raw device number and
    its probe path. The walk maps it with its own lister's resolver as it consumes the folder, in walk order, with the
    same probe as today: the first entry of any kind seen on the device (`dir.rs:503`), before the walk's filtering.
  - A file among those gets its identity read on the walk's connection. The F1 change mark makes that safe from any
    thread.
  - So identities, claims and the fallback count are exactly a single-threaded walk's.
- **Workers take folders in walk order, and results are kept by walk order.**
  - **Hand-out.** A folder's subfolders can be handed out as soon as a worker has listed the parent, before the walk
    consumes it.
    - The skip names, the loop chain and the reparse rule are functions of the listing and the folder's ancestors, so
      the worker that lists the parent applies them. It carries the ancestor chain.
    - A subfolder on another device waits for the walk's mount verdict. So no worker lists a refused mount or a reparse
      folder, and a chain of single subfolders still gets listed ahead.
  - Workers take the next folder in walk order that nobody holds or is listing.
  - **The admission rule:** the folder the walk will consume next is always admitted, whatever the bounds.
    - Over the bound, in entries or folders, the held results farthest in walk order are dropped first. They are listed
      again when they come near.
    - Without this rule, held results for a folder's siblings could fill the bound, and the walk would wait forever for
      the child it needs next. `pipeline.rs` already grants in list order (`:420-422`).
  - Results stay held while their folder is on the walk's stack. They are not dropped because newer subfolders were
    pushed above them, so a huge folder's listing and row read are not repeated for that reason.
- **The walk consumes each folder's result in its own depth-first order,** and does everything else on its own thread,
  as now: the seen identities, chain, mounts, skip rules, batches, lease and the seen set. Its results equal a
  single-threaded walk's.
- **Worker connections.** Each worker has its own `Lister` and its own read-only connection with a small page cache.
  `Catalog::open` would carry the 256 MB `cache_size` of `SETTINGS` on each.
  - **Not `Catalog::reader`** (`core.rs:214`), though it opens the same kind of connection. Its `Reader` borrows the
    `Catalog`, which the walk holds mutably for its batches, so it can't be on another thread. And for an in-memory
    catalog it reuses the catalog's own connection.
  - **A new owned type instead**, for example `CatalogReadOnly::open(path, cache_kib)`. `read_only` (`core.rs:247`,
    which `Catalog::reader` already calls) opens its connection with the same flags as
    `reader` (`SQLITE_OPEN_READ_ONLY`, `NO_MUTEX`, a busy timeout and the given cache), is `Send`, borrows nothing, and
    has the folder-row and identity reads. `ScanLease` makes one per worker from the catalog's path. The reads need no
    collations or SQL functions. If Phase 5's items 6 or 8 are adopted, they need `rarray`, and so
    `vtab::array::load_module` on every connection that runs them, these included.
  - **It holds the file itself.** `Catalog::reader` relies on the catalog's shared hold on the file to keep a rebuild
    out (`core.rs:209-213`). The new type takes its own `pg_dbfile::Live::shared` (`dbfile/src/lock.rs:55`), so it does
    not depend on how long the catalog lives.
  - **An in-memory catalog has no path,** so another connection can't reach it. The walk then runs with no workers, on
    its own connection, as now.
  - **Workers start lazily.** Opening a connection and parsing the schema is pure overhead for a small walk, so a
    worker starts only once the walk has more than one folder waiting.
  - **They run at `BelowNormal`,** as the background pools do (`library/src/runner.rs:581`, `:596`, `:611`).
- **The lease is renewed only while the listing makes progress.**
  - Today a hung readdir stops `beat`, and the lease expires after `SCAN_LEASE_SECS` (`scan/src/lib.rs:39-44`).
  - Linux accepts FUSE mounts: only network, FAT and pseudo file systems are refused (`platform/src/refusal.rs:119-141`).
    A dead sshfs or rclone server hangs readdir forever.
  - If the free walk thread renewed while it waited, every overlapping walk would be refused as Busy for as long as the
    worker hangs, and `gallery scan --wait` would never end.
  - So each worker bumps a progress counter per entry, where `beat` runs today. The walk renews every `RENEW_EVERY`
    only if the folder it waits on made progress since the last renewal.
  - With no workers (an in-memory catalog), `beat` stays as it is.
- **Pause and stop.**
  - The walk stops handing out folders, and drops results it didn't use.
  - A worker checks the control between folders, with `ctl.checkpoint_until(&stop)`, as `pipeline.rs` does (`:133`).
  - A guard on the walk sets `stop` and calls `ctl.wake()` when it is dropped (`pipeline.rs:408-418`). So a walk that
    returns early (a lost lease during a pause, a busy batch) never waits to join a worker parked in a pause or on the
    window.
  - A pause holds no catalog lock, since workers only read, outside transactions.
- **Panics.**
  - The desktop's panic hook shows the fatal-error box unless `pg_jobs::panics::caught()` is true
    (`desktop/src/panic_log.rs:21-33`). That flag is per thread (`jobs/src/panics.rs:11-14`), and the scan job's catch
    covers only the job's thread (`library/src/runner.rs:413`).
  - So each worker's body runs inside `panics::catch`, sends the payload back, and the walk raises it again with
    `resume_unwind`. That keeps the payload, which `thread::scope`'s own re-panic loses.
- **The `Listing` trait** (`scan/src/lib.rs:74`) gains a way to make one lister per thread, and becomes `Send`.
  - The Windows `Dir` wraps a raw `HANDLE`, so it is not `Send`. Each worker opens its own folders, and the root's `Dir`,
    opened on the walk's thread (`scan/src/lib.rs:411`, `:440`), is used only there.
  - `FakeListing` follows, its hooks included (`on_listing`, `after_listing`): a hook runs on the worker that lists the
    folder, at the same point of the unit.
  - So does `synthetic::SyntheticTree`.
- **The worker count is a measured value per kind of device,** applied as a concurrency cap per device, since a Linux
  walk can cross devices.
  - The cold runs were on one drive, an NVMe SSD (`D:`, KINGSTON SNV2S2000G, checked with `Get-PhysicalDisk` on
    2026-11-15; the report doesn't name the drive).
  - **The HDD is not assumed to want one lister.** The repo's own measurement on the library's HDD says the opposite for
    reads (`IO.md:5-29`): 8 reads in flight let the drive's NCQ reorder them, reaching sequential speed, and for sparse
    reads "only queue depth helps". Listings may behave the same, so they are measured.
  - The exception known in advance: a USB HDD behind a bridge without UAS has no NCQ (`STORAGE.md:34`).
  - **The device's kind:**
    - on Windows, `IOCTL_STORAGE_QUERY_PROPERTY` with `StorageDeviceSeekPenaltyProperty` and the bus type;
    - on Linux, `queue/rotational` of the partition's parent disk.
    - An answer that can't be had falls back to the default count with a log line. Examples: btrfs, overlay or ZFS
      (major 0, no `/sys/dev/block` entry), or a spanned Windows volume.
  - The query belongs with the platform's volume facts (`crates/platform/src/volume.rs`).
  - **Built (2026-11-18):** the device query is `crates/platform/src/drive.rs`, not `volume.rs`, as the
    multi-drive lanes' Phase 1 (`docs/plans/MULTI_DRIVE_LANES_PLAN.md`). Its answer tunes the job's lanes; the listing
    workers of this plan's Phase 4 still take a fixed count (`WORKERS = 4`), which the lanes' Phase 3 would set per kind.

### Phase 4: changes and tests

1. The pool, the ordered window and the hand-off in `pg_scan`; the volume mapping on the walk's thread; the trait
   change; `FakeListing`; the read-only connection type; the device query.
2. **The worker count is a parameter of the walk**, and the whole `FakeListing` suite runs with 0 workers and with N.
   - The walker tests' `catalog()` is in memory (`scan/src/lib.rs:896`), and so would run with no workers. The suite
     therefore runs on on-disk catalogs (`pg_testkit::run_folder`) for N.
3. Tests whose meaning changes with prefetch are rewritten for it:
   - `a_stop_mid_walk…`: its hook on `b` stops the walk, and with prefetch the stop can land before `b` is consumed.
     The test asserts what a stop guarantees, not which folder it lands on.
   - `a_listing_that_outlasts_the_lease_keeps_it`: it relies on the per-record renewal with `renew_every` at zero. It
     becomes a test of the walk renewing while the listing it waits on makes progress. A second test covers a listing
     that hangs: the lease expires.
4. New tests:
   - a walk with 0 and with 8 workers gives the same stats and rows, on a fake tree with unreadable folders, a reparse
     folder, a refused mount, a loop, a second path, a folder with more subfolders than the window, and two volumes;
   - a write-back between a worker's listing and the batch is kept;
   - a stop and a pause with listings in flight;
   - a walk that fails during a pause, with workers parked, returns at once;
   - a worker's panic surfaces on the walk with its payload, and counts as caught;
   - when the held results fill the bound, the walk still gets its next folder;
   - on Linux, two cloned devices with one UUID resolve as they do single-threaded, and a fake tree whose entries have a
     mapped identity different from their raw device number gets its rows matched.
5. **Tuning, by measurement:** the window, the bounds, and the count per kind of device.
   - Measure the ordered design itself, cold, at 200k: the scan's own cold rescans and first scans.
   - Two library shapes:
     - today's flat one, 400 leaf folders under one root;
     - a nested one, year/month/event, where the hand-out's order matters.

     `list_probe` lists a flat set, with the root outside its timing, which is the best case for this design.
   - On the NVMe drive and on the HDD: `Get-PhysicalDisk` shows three here, among them an ST24000NM001H. Run
     `list_probe` cold at 1, 2, 4 and 8 listers on each first.
   - `just perf` never runs this path: its scan bench is in memory (`bench/benches/ir/catalog.rs:256`), so it has no
     workers.

**Built, not tested (2026-11-17).** The walk lists ahead with workers, as the design says, with these differences:

- **No admission rule and no eviction.** The walk never waits on a folder no worker has taken: it lists that folder
  itself (`take` returns `Inline`). That keeps the walk from waiting on a folder nobody lists, so the rule that grants
  the next folder isn't needed. Held results are not dropped: `AHEAD` (entries listed and not consumed) only stops the
  workers taking more folders.
- **The identity reads are the walk's.** `prior_states` runs on the walk's connection for every changed file, not only
  for files on another device. The worker reads only the folder's rows (`FolderRowReader::folder_rows`, before the
  listing), so the seen set, mounts, loops and the batch stay on the walk's thread.
- **Devices are mapped by the walk, in walk order.** The platform's `Entry` carries the raw device (`Entry::device`).
  `Lister::map_device` is called by the walk for each entry on another device, in the order a single thread would
  meet them. The Linux lister no longer maps while it lists, so workers hold no identity state. A worker's lister is
  `Listing::fresh`.
- **The lease is renewed while the walk waits** on a worker's listing, only when that listing has moved since the last
  renewal (`take`, every `RENEW_EVERY`). A hung listing lets the lease expire.
- **Workers:** `WORKERS = 4` on every device. The device-kind query (`volume.rs`) and the count per kind are not built;
  they wait for the measurement this phase asks for.
- **Panics** are caught on the worker, delivered with the payload, and raised again on the walk
  (`Outcome::Panicked`, `resume_unwind`).
- **Tests added:** a walk with 4 workers gives the same judgments and rows as one with none (`judged_tree`); a panic
  in a listing reaches the walk with its payload; a stop with workers listing marks nothing missing. Not written yet:
  the write-back between a worker's listing and the batch, a pause with workers parked, a walk that fails during a
  pause, and the two cloned devices on Linux. The rewrites of the prefetch-sensitive tests (change 3) are not done; the
  tests on an in-memory catalog have no workers and run as before.

## Phase 5: the small items

Their costs were measured in the app, except item 11.

**Two are bets, not sure wins: items 6 and 8.**

- Batching them saves the per-statement cost of bind, step and reset. It keeps every B-tree probe, and adds the cost of
  passing the list.
- The probe showed that a list passed as JSON can cost more than it saves: the set-based insert, k2, ran 340.4M against
  c's 197.9M.
- So:
  - the list goes in as a bound array, with rusqlite's `array` feature (`rarray`), not JSON. It needs
    `vtab::array::load_module` on each connection that runs it: the catalog's (loaded by `register()` since the pairing
    work, `core.rs:240`), and Phase 4's workers';
  - each is added to the variant probe first, against today's per-file statement;
  - each is adopted only if it is faster in both instruction counts and Windows time. The rule is fixed now, and the
    measurement only applies it.

The rest are the right code regardless of their size.

| Item | Change | Cost now |
|---|---|---|
| 6 | The known lookup per folder: one statement over the folder's identities (`rarray`), into a map, in place of one per file. Phase 3 has folded the root check into it, and leaves it only the files that need a write. Gated as above | 1.9% of a first scan; on a rescan only the changed files remain |
| 8 | The identity read after the listing (Phase F1's `prior_states`) as one statement per folder (`rarray`). Gated as above | 4,000 empty lookups, 0.9% of a first scan |
| 9 | Folder ids resolved once per batch | 0.3% |
| 4 | Statements reused across batches: raise the connection's statement cache (`set_prepared_statement_cache_capacity`) to hold the scan's and the import's statements, and prepare them with `prepare_cached`. The comment at `writers.rs:24-26` chose `prepare` because the cache held 16 | 1.6M Ir per batch |
| 11 | The listing's names moved into the batch, not cloned (`scan/src/lib.rs:558`) | Not measured |
| 13 | The walk's identity set as a hash set (`FileIdentity` derives `Hash`; it is only queried) | 0.2% |
| 10 | An ASCII fast path in `Ext::of_name_bytes` | 0.2% |

Item 17 (skipping `mark_missing` on folders with nothing missing) is covered by Phase 3: `mark_missing` reads first,
and takes the write lock only when there are rows to mark.

**Built, not tested (2026-11-17).** Items 4, 9, 10, 11 and 13 are in:

- **4:** the scan writer's statements are `prepare_cached`, and `core::register` sets the connection's cache to 64
  (`STATEMENT_CACHE`). The import's writers still `prepare` per transaction: their change waits with the measurement.
- **9:** `ScanWriter::folder` memoises each folder's id for the writer (one batch), so a folder is looked up once per batch.
- **10:** `Ext::of_name_bytes` compares an ASCII extension byte by byte, without decoding (`eq_ignore_ascii_case` against the
  lower-case lists).
- **11:** a listed file's name moves into its batch entry (`ListedFile::name`); the walk owns the listing.
- **13:** the walk's seen set is a `HashSet`; it is only queried for membership.

**Not built, and why:**

- **6 and 8 (the `rarray` lookups).** Their rule is to add each to the variant probe against today's per-file statement
  first, then adopt it only if the measurement says so. The probe variants are the measurement's work, so they wait for
  it. Item 8's lookup would go in `prior_states` (`roots.rs`), where the list is a bound array on the walk's connection;
  item 6 changes `record_file`'s control flow, which is the larger edit.
- **The allocator (item 20).** The trial adds `mimalloc` to the workspace and needs the crate fetched, so it is a
  measurement step, not a code change to make before it.

**The allocator (item 20), gated.** (Reevaluated 2026-11-17: `mimalloc` is not in `Cargo.lock`, so the trial adds the
crate. Nothing else here changed.)

- glibc's malloc and free are 14% of the process. Phases 1 and 2 remove most of the SQLite allocations behind that.
- A Rust `#[global_allocator]` doesn't reach SQLite, which calls the system `malloc`. So the trial is mimalloc for both:
  - the global allocator for Rust;
  - for SQLite, `sqlite3_config(SQLITE_CONFIG_MALLOC, …)` with mimalloc's functions, once, before the first
    connection.
- The trial runs after Phases 1 and 2, on the first scan and the rescan at 200k.
- **The rule:** adopt it if it is faster in both instruction counts and Windows wall time on both runs, with no loss
  elsewhere in `just perf`.

## Not doing

From the report's item verdicts, with the reason:

- **Set-based inserts (item 3):** the JSON costs more than the journal saves, and Phase 1 with the lookaside beats it.
- **Merging `prior_states` into the known lookup (item 7):** wrong. It would read the prior state inside the write
  transaction, and so overwrite a write-back committed between the listing and the batch.
  - Phase 3 goes the other way, and that direction is safe: it decides before the batch only the files whose outcome
    can't change.
  - Every file that needs a write still gets the guarded lookup inside the transaction.
- **Larger batches (item 15):** the batch size bounds how long the write lock is held (W2). The commits are 25.8M, 0.3%
  of a 200k rescan; with the writers' making, 0.7%.
- **Rust-only LTO (item 21):** −0.7%.
  - SQLite is C, so by default it is outside Rust's LTO.
  - Cross-language LTO would bring it in. It hasn't been tried, and is tracked in `OPEN_ITEMS.md` ("Cross-language LTO
    with the bundled SQLite"), with four builds to compare and the rule for adopting one.
- **`SQLITE_OMIT_*` (item 22):** no gain, and JSON1 is used.
- **mmap (item 23):** two thirds of the `memcpy` is the statement journal's copies, which `mmap_size` doesn't touch
  (Phase 1 removes most of them).
- **Folding the UPDATE triggers into the writer:** they have many writers (see Phase 1).
- **Removing the `files` column defaults:** harmless as they are (see Phase 1). It needs a foreign-keys-off rebuild of
  `files`, which the MFT plan does anyway.

## Decisions

- **D1. The `files_changed` index. Decided (2026-11-15): keep it.** It costs 16.4M per 4,000 files and has no
  production reader yet. It is for a client's resync, which is a planned reader, so it stays. (`files_fresh` goes in
  Phase 3 for another reason: its only column goes.)
- **D2. Lookaside for every connection. Decided (2026-11-15)** (Phase 2).
- **D3. The perf baseline. Superseded (2026-11-17).**
  - The user re-measured the baseline at `0d44547` (`f036825`), which contains Phase F. The scan gate is live (±1%,
    `tools/perf-compare.py`).
  - Each phase now re-baselines its own scan entries in its commit, from its callgrind archive, as `f036825` did.
  - The decision as first written, kept for its reasons:
  - Phases 1 to 3 move `scan_walk.first` and `scan_walk.again` (`docs/perf/baseline.json`) far outside ±1%. Phase 4
    doesn't move them, since the bench's catalog is in memory and runs with no workers. Phase 5's small items may not
    either.
  - Re-baselining waits with `NOTIFICATIONS_PLAN.md` (N-43, D10) until `tree.all` is explained.
  - Until then, each phase records the scan benches' new counts here, and `just perf` misses on them are expected, not
    failures to fix.
- **D5. Order with the MFT plan. Decided (2026-11-15): this plan first.**
  - Its gains are measured and near, and the MFT plan is large.
  - The MFT plan is updated to match:
    - its migrations follow this plan's;
    - it owns the table-rebuild step, with the review's corrections;
    - its `ScanWriter` keeps Phases 1 and 3's rules;
    - `links` has no `last_scan`, and its sweep reads the seen sets;
    - it keeps Phase F1's order;
    - its step 1 starts from what this plan leaves.
- **D6. No table rebuild in this plan. Taken on the review's evidence (2026-11-15); revisit if you disagree.**
  - The rebuild this plan had (a Phase 0) was for two changes. `last_scan` drops without it (`DROP COLUMN`), and the
    other, removing the column defaults, is a weak guard (Phase 1).
  - A foreign-keys-off rebuild of the table every user-data table hangs off is the riskiest migration there is. So it is
    done once, where it is needed anyway: the MFT plan's `files`/`links` split.

## Measurement protocol

**Validated, 2026-11-17 (tests and lint, not the measurements).** Phases 1 to 5 build and pass the workspace's tests
(1,241 passed, none failed), `cargo clippy --workspace --all-targets -- -D warnings`, and `tools/lint.py`. The fixes
that validation needed: the temp index's `CREATE INDEX temp.…` (Phase 3's `ensure_seen` was not valid SQL), the
test helper that starts a walk with an empty seen set (as `begin_scan` does), the `mark_missing` expectation (the
write lock is taken only by the last transaction when nothing is left to mark), and the lint points (`BTreeMap` and
`BTreeSet` in place of the hash types the project bans, one doc link). The measurements below are not run yet.

After each phase, repeat what the report measured:

- **Callgrind**, in the `pg-dev` container (the justfile's perf recipe runs the benches there):
  - the scan probe on `target/scan-probe/lib` (4k) and on the `pg-scan-large` volume (200k), first scan and rescan
    (`target/scan-probe/app_level.sh`);
  - then `just perf`.
- **Windows**, elevated, from `target/scan-probe/`:
  - `win_cpu.ps1`: flushed-warm runs, `purge.ps1 -FlushOnly` before each, with CPU time;
  - `win_cold.ps1`: cold first scans, cold rescans and cold listing, `purge.ps1` before each;
  - medians of 5 per script, or 10 for the 200k cold first scan, from `win_cold.ps1` alone. The report's 10 pooled two
    scripts;
  - not back-to-back warm runs: they are noisy (one took 36.8 s), because the last run's catalog is still being written
    back.
- **Phase 3 also:** the WAL bytes a 200k rescan writes, before and after. That is the I/O its instruction counts don't
  show.
- **Phase 4 also:** a nested library as well as the flat one, and the HDD as well as the NVMe drive (Phase 4, step 5).
- **Before and after binaries.**
  - "Before" is the phase's parent commit. Build it in a detached worktree (`git worktree add --detach <dir> <sha>`)
    with its own `CARGO_TARGET_DIR`. Build from a commit, never the moving working tree.
  - Never build two copies of the workspace into one target dir. Cargo names a path package's artifacts by its place in
    its workspace, so one copy's `pg-catalog` replaces the other's. That invalidated a round of the report's runs (the
    report, **Corrections**).
  - Check each binary before timing it:
    - Phase 1: the catalog's `user_version` after a run, and `files_added` absent from `sqlite_schema`;
    - Phase 3: also no `last_scan` in `files`, and no `files_fresh`;
    - Phase 2: `PRAGMA compile_options`.

## After this plan

- **The identity pairing** (`pair_after_scan`) is half of a 200k first scan once Phase 1 is in (13.97G of 28.15G, measured before the pairing commits: re-measure). It
  needs a probe and a report of its own.
- **Cleanup**, once the measurements are done:
  - the probe data: `target/scan-probe/large` (about 0.3 GB) and the docker volume `pg-scan-large`;
  - the experiment's target dirs in `pg-cargo-target` (`patched`, `lto`, `fork`);
  - the copies `target/scan-probe/tree` and `target/scan-probe/fork`.

  Keep the scripts until Phase 4 is measured.
- **The report** stays in `docs/temp` until the plan is done. Then its figures fold into this plan's phases, and it
  goes.
