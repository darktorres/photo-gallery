# Plan: proper fixes for the audited background-job defects

## Context

The user removed folders while the background job was running and it kept processing. An audit of the whole job
(runner, scan, import, borders, thumbs, catalog writers, leases, pipeline) found that case plus related defects. The user
wants the most proper fix for each, including small gaps; churn is not a concern; and the fixes must **use the designs
already recorded in BOUNDED_MEMORY_PLAN (BM) and DECODE_ONCE_PLAN (DO)** instead of parallel mechanisms, amending those
plans. This version is the result of: 3 code explorers, an adversarial design review, a fact check, a scope/constraint
review and a reconciliation against the recorded plans. Line numbers drift; confirm each against the code as it is edited.

Discarded (user request): the earlier uncommitted `Library::folder_removed` change in `crates/library/src/runner.rs` and
`crates/app/src/library.rs` is reverted to HEAD, so the round stop on folder removal is again to be written (fix B8/A3).
Not mine and to be left alone: modified `crates/catalog/src/writers.rs` and
`crates/import/src/lib.rs` (temporary `TRACE` timing, `writers.rs:247-262`, `import/lib.rs:100`) plus docs/tools files:
edit around them, never revert or commit them.

User decisions: rescan on start (plus idle/slow rescan); use the recorded designs. **Decided by me, redirect if wrong:**
a BASELINE edit, in place, since no release has been made (`schema.rs:28-31`: the baseline is edited in place before the first release and frozen after it; the BM §1.3 "one BASELINE edit" is that rule);
`analysis` stays `INSERT OR REPLACE` for now (DO 3.5's upsert is unsafe against today's triggers; settle before DO
Phase 4); repair re-pairs the whole library once after the upgrade (resumable, low priority); slow rescan default 6 h
while idle (a parameter); round order unchanged (D17 vs DO 2.9 is DO Phase 2's); batch 100 files / 250 ms for import and
borders (DO 5.5; BM D18's frame budget supersedes later); synchronous removal; `Q3` pairing; platform-level `Unreadable`.

Standing rules: one cargo build at the end (no cargo between steps); no new branch; commit only when asked; workspace
lints (no indexing/unwrap/expect/panic/bare casts, `pg_model::num`, `pub(crate)`, `split_at_checked`/`chunks` for
bisect, `checked_mul`/`saturating_mul` for backoff); `just lint` at the end; native PowerShell build, Linux CI only in the
pg-dev container.

## Facts that shape the design

- Baseline frozen; `MIGRATIONS` = TO_2..TO_9; **TO_10 is the audit slice**, one migration `[Sql(a), Rust(dups_repair)]`
  (SQL first so the column exists). DO's TO_10 content is absorbed; DO's TO_11 stays TO_11 (no renumbering).
- `content_gen` is also recorded in `docs/plans/NTFS_MFT_SCAN_PLAN.md:279-291,345`: that plan gets amended too.
- Two recorded plans conflict on `INSERT OR REPLACE` vs upsert for `analysis` (BM 2.1/2.4 vs DO 3.5/4.4); BM Q3 already
  decides "the missing flag is ignored when pairs form" (D25 wording ambiguous).
- Real guard tests for a migration: `carried_is_every_table_of_the_current_schema` (in `catalog/src/schema.rs`),
  `the_baseline_and_the_migration_make_the_current_schema` (`:956`),
  `every_older_version_reaches_the_current_one_on_both_paths` (`lifecycle.rs:1229`). SQL steps never reach rebuilt
  catalogs, so a data repair must be a Rust `convert` step (`convert.rs:1-12`).

## TO_10 contents

**Sql:**

- `files.content_gen INTEGER NOT NULL DEFAULT 0` + trigger `files_content_gen AFTER UPDATE OF size, mtime, volume,
  file_ref` (not `created`: its backfill would bump it); `DROP VIEW thumb_sources` + recreate (TO_6 shape) with
  `f.content_gen`.
- `work_leases(id INTEGER PRIMARY KEY, kind, target BLOB, owner, token INTEGER NOT NULL, beat INTEGER NOT NULL DEFAULT 0)`
  replacing `scan_leases`; named unique partial indexes `(target) WHERE kind='scan'` and `(kind) WHERE kind='pair'`
  (nested-root overlap stays a Rust check); `meta.lease_next_id`. These six columns are TO_10's. The full table in
  `BOUNDED_MEMORY_PLAN.md` (Shared tables) has six more (`inputs, seq_lo, seq_hi, dead, progress, reserved`) and is not
  in TO_10.
- `root_removals(id, path, hidden, report, finished, cursor_path, cursor_id)` + unique index on `path WHERE finished IS NULL`
  (`key_lo/key_hi` come with the reader mask).
- `meta.roots_gen` + triggers `AFTER INSERT/DELETE ON roots` (bumped by CLI, app and tests alike); `meta.dups_version`
  seeded (DO 3.3; unused until DO Phase 3).
- `analysis.dups_state TEXT NOT NULL DEFAULT 'done' CHECK (…)` + `analysis_dups_due` (DO 3.3). `store_analysis` and
  `store_failure` name it in their REPLACE (`done` after inline pairing, `due` if pairing was skipped).
- `io_attempts(file_id REFERENCES files ON DELETE CASCADE, pass, attempts, next_try_at INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(file_id, pass))` + the reset trigger of DO 3.3.
- `file_writes.parked_folder`, `file_writes_ready` replacing `file_writes_next`, the unpark trigger on `files`, and a
  trigger on `file_writes` (`temp` cleared or row deleted) that bumps `files.seq` so Thumbs relists a file it excluded.
**Rust step:** `dups_repair`, shape-guarded and idempotent: `Source::Main` always; `Source::Carried` only if the carried
`analysis` lacks `dups_state`. Marks `pdq_dihedral IS NOT NULL AND pdq_quality >= MIN_QUALITY` rows `due`.
**Bookkeeping:** `rebuild.rs` `TRANSIENT` (`work_leases` replaces `scan_leases`), `CARRIED` (`root_removals` after `roots`,
`io_attempts` after `files`); bench catalog `LAYOUT` (`crates/bench/benches/ir/catalog.rs:40`); fixtures need only the lease
tests; tests that scan without `add_root` get one (`library/tests/borders.rs:36`, `library/tests/writeback.rs:84`,
`thumbs/src/lib.rs:406`, pair.rs `begin()` tests, bench, `a_folder_outside_the_library_is_not_recorded`).

## Fixes

### A. Removal, scans, leases

1. **`IS_CURRENT`** (`catalog/src/current.rs:13-16`): add `AND missing = 0` and `AND content_gen = ?5`; `query_current`
   binds it. Every user inherits it (`AnalysisWriter` writers.rs:294, `BordersWriter` :450, `Catalog::is_current`
   lookup.rs:25, thumbs keep-check). Plumb `content_gen` through `Listed`, `work_columns!()`/`work_list`
   (`work.rs:46-53`), `ThumbWork` (`thumbcache/reader.rs:48-55`, `pending_sql`), and the two literal `Listed{..}` sites
   (`thumbs/lib.rs:230`, `scan/lib.rs:1210`). Not the scan upsert (it sets `missing = 0`). Exact; replaces the heuristic
   identity compare.
2. **Leases (BM D16)**: helper in `pg_catalog`: `claim`, `fence(tx)` (`UPDATE … SET beat=beat+1 WHERE id=? AND token=?`
   must hit exactly one row, else `Error::LeaseLost`), `release`, adoption. Staleness is a **counter** unchanged for the
   adopter's own monotonic 60 s (remembered in a field on `Catalog`), not `unixepoch()`; `SCAN_LEASE_SECS` stays as that
   window. A heartbeat **thread** with its own plain connection beats every 5 s (not built: the code renews the lease every 20 s, `crates/scan/src/lib.rs:205`, and writes its heartbeat in each write transaction, `crates/catalog/src/lease.rs:21`) (covers FFmpeg stalls, pauses, 500-video
   batches without touching `pipeline::run`; `:memory:` renews from the owner). BUSY on renew = retry, not `Lost`.
   `Error::ScanBusy` → `LeaseBusy{kind,target,holder}`; `ScanError::Lost` (+ the exhaustive match at `cli/lib.rs:168-183`);
   `scan/lib.rs:600-601` `ensure!` becomes typed `Lost`.
3. **Removal (BM D34/3.4/D10, slice)**: `remove_root` = one transaction that deletes the root row, deletes overlapping
   `work_leases` kind `scan` (the in-flight walk fails its next `fence`), and inserts the `root_removals` marker; then it
   drives the pager **synchronously** to completion and returns the count as today (`cli/lib.rs:218`,
   `app/library.rs:312-321`). Pager: folders in `folders` path order over `under_params`; page
   `UPDATE files SET missing=1 WHERE id IN (SELECT id … AND missing=0 AND id>? ORDER BY id LIMIT 1000)`; each page
   transaction re-checks that its marker row still exists and skips any folder now under a live root (this one rule covers
   re-add of the same path, covering root, nested root; no invariant sweep needed). A runner `Removals` slot resumes a
   leftover marker at job start. `begin_scan` refuses a path no root covers; `ScanWriter::new` returns `Lost` when
   `covering` is `None` (`writers.rs:84`); `pair_after_scan` and `mark_missing` become `ScanLease` methods whose batches fence
   inside their own transaction (`pair.rs:266 in_batches`, `roots.rs:132`; `take_identity` sets `missing = 0`, `pair.rs:357`).
   Out until the reader mask lands: hidden-at-once removal, `scope_keys`, CLI "drive removals first". Interim guard for D10:
   a source-pattern test (`UPDATE\s+files\s+SET[^;]*?\bmissing\s*=`) with an allowlist of statement sites.
4. **Runner on `Lost`**: re-read `roots()`; root present (a lease taken over after a suspend > 60 s) → requeue; gone → drop
   silently, no `ScanFailed`. A `Lost`/`Failed` mid-pairing must not drop the post-commit `Pairing`
   (`scan/lib.rs:536`, `?`): carry the partial `Pairing` in the error so `replaced_ids` (stale thumbnails after a same-size
   replacement) and `temps` (leaked files; `replace` already nulled `temp`, `pair.rs:420`) are still handled; call `scanned()`
   on `Lost` as on `Failed`, and only when something changed.
5. **`roots_gen` + import lease (BM D16 kind `pair`, D25/5.7)**: every pass compares `roots_gen` at each flush and returns
   `Preempted` when it moved (read baseline before the worklist; add vs remove generation separate so adds don't
   preempt; Import needs a "stopped early" signal, it returns `Done` today, `import/lib.rs:141`). The Import takes the
   `pair` lease after an `EXISTS` probe on `files_to_import` and before `pending_import` and the hash load; the pairs-only
   pass holds it too; busy → `Step::Busy{until}`; the CLI waits or fails with a message. The Phase 5 pair worker later
   holds it instead.

### B. Round, retry, schedule (BM D17, DO 2.9/3.10)

1. `Step::Busy{until: Instant}` (`jobs/schedule.rs:9-14`). `schedule::round` runs every pass; on a pass `Err` **or panic**
   it records the first error, continues with the later passes and returns it last; stop/`Preempted` logic unchanged;
   rewrite `an_error_ends_the_round_with_it`. `Cx.catalog` becomes `Option<Catalog>` with reopen after a panic
   (`runner.rs:383-392`).
2. **One deadline table** in the runner (replaces `BUSY_RETRY`, `runner.rs:42,355`): busy retry, failure backoff
   5/10/20/40/60 s (forever; reset on a successful `Done` round; per-pass deadlines so a foreign lease doesn't re-run the
   failing pass every 5 s), `next_try_at`, the idle check, the slow rescan, scan retries. A user wake runs at once without
   resetting the count; an error never sets `again`. `Waker` gets a "take the wake flag" method (start of a round) so
   `again = true` doesn't leave `woken` set and cost an empty round + `Change::Round`.
3. **`wake()` must not `request_stop`** (`runner.rs:265`; the sticky `woken` guarantees another round). Stop stays for
   `folder_removed`, crops, scan requests. Today a turned photo or Retry (`desktop/library.rs:140-144`,
   `notices.rs:484`) restarts a non-resumable walk from the root, so a startup rescan would starve.
4. **Clear `round-failed` only on a `Done` round** (not `Preempted`, `runner.rs:370-372`).
5. **Notices**: `RoundFailed` stays a `State`, static data (no per-retry countdown; a state re-posted with unchanged data
    keeps marks and counts, `catalog/src/inbox.rs:304-315`); no counts embedded in text that changes each round.
    `RoundPanicked` → `State` with `NextSuccess(Op::Round)`; a deterministic panic retries at the cap interval only.
    `ScanFailed` → keyed `State` per root with explicit `Key::all("scan-failed", Some(root))` cleared when a scan of that
    root succeeds. Regenerate the resolver table in `crates/model/src/notice/resolver_table.golden` (`the_resolver_table_is_the_golden_one`),
    `golden.jsonl`, the `ordinal` test, `crates/client/locales/en/text.ftl`.
6. **Scans**: read `roots()` per popped path (`runner.rs:450-456`; today a folder added mid-scan is dropped by `continue`),
    drop the stale `library.contains(&path)` at `:491` (replace with fresh roots, or a sub-folder scan replaces a root's
    scan-notes); per-root retry deadline, dropped on `folder_removed`; a renamed/moved root gets a distinct "folder not
    found" state with "remove then add" advice, not endless silent retry.
7. **Rescans (BM 3.9/DO 3.10)**: a scan of every root once at job start (after the catalog opened and `roots()` succeeded);
    a **cheap idle check** (separate path: no `Change::Round`, no inbox poke): `Catalog::idle_work()` = `EXISTS` probes on
    `files_to_import`, the borders-due and dups-due indexes, `roots_gen`, earliest `next_try_at`, actionable work only
    (DO's `feature_counts` is a UI count; amend DO 2.8/3.10); a **slow full rescan** timed from completion (default 6 h).
    All are requests to `scan(root)`, so they stay valid when a journal pass later sits behind it. Prune `scans` only rows
    referenced by no `files.last_scan` and no lease, never the max id (no AUTOINCREMENT; `folder_refused` uses `max(id)`).
    Refuse mark-missing when a root lists zero entries or its volume changed (an unmounted mount point on Linux/macOS), and
    report it as a state. Make the streaming video hash a chunked loop that checks the stop flag (stop latency today is the
    slowest worker; `JOB_STOP_WAIT` 5 s at quit leaves the lease up to 60 s).

### C. Pairing and import

1. **Q3 pairing**: `pdq_hashes()` drops `f.missing = 0` (`work.rs:106,113,123`; readers filter via `present_pdq_pairs`).
    A restored file needs nothing. No trigger on `missing` 1→0 (`files_missing_dups`, DO 4.4, stays a separate stamp trigger).
2. **`dups_state` + pairs-only module in `pg_import`** (DO 3.3/3.5/3.7, slice): `pending_pairs` paged by keyset on
    `file_id`, ≤100 rows per transaction; the write tx re-verifies the hash and `pending = 0` before `pair_and_insert`,
    `store_pdq_pair`, `store_pairs_done` (guarded, `temp` guard); holds the `pair` lease. The `dups_repair` step makes the
    whole library `due` once; the pass rebuilds pairs from stored `pdq_dihedral` with no file reads (ON CONFLICT keeps
    the best distance). Out: toggles/`Features`, `DUPS_VERSION` bump code, `store_hashes`, `skipped`, `FilePass::Duplicates`.
3. **`pdq_hashes` error** (`work.rs:109-119`): at hash load, mark rows with `pdq_dihedral IS NULL AND pdq_quality >= MIN`
    `pending = 1` (`mark_pending` pattern, `work.rs:141`) **before** the worklist read in the same pass, skip them in the
    index, log once; skip `dups_state <> 'done'` rows too. The hash load stays in RAM (D13 deletes it in Phase 5) but lazy:
    built at the first PDQ-eligible result, never cached across rounds (`PdqIndex` has no delete, a re-analysed file's old
    entries would pair later files wrongly). `IndexFull` is unreachable (2^32): no item.
4. **Pairer vs commit**: a failed/retried flush rolls the Pairer back to its length at flush start (truncate if append-only,
    else rebuild lazily). Intra-batch pairs stay found because inserts happen during the flush, before commit.
5. **Flush errors (new; becomes the `Sink` body later)**: a `Flusher` in `jobs/pipeline.rs` beside `batched` wrapping
    `flush`, with a classifier closure (`pg_jobs` can't name `pg_catalog::Error`). Classify by SQLite primary code:
    CONSTRAINT/Value attributable to an item → terminal; BUSY, FULL, IOERR, READONLY, CORRUPT → systemic: keep the batch,
    retry with stop-aware backoff, never bisect. Closures consume the batch only after commit and apply counters after
    commit (`import/lib.rs:85` drains before commit; borders/thumbs likewise). Bisect only for import and borders (the
    thumbs flush writes the cache). Before bisecting to one item, probe with a trivial write transaction; an FK failure from
    a stale pair row rebuilds/skips the pair instead of failing the item; the terminal item gets a `retry:false` ledger row.
    Announce committed work despite `Err`: passes take `&mut Stats`, Import adopts Borders' `done`-copy pattern, so
    `imported()` and `files_read(…, complete=false)` still run (`runner.rs:516,530,556`).
6. **Bytes vs listed size**: a length mismatch between bytes read and `item.size` is `changed`, store nothing
    (mid-copy files, camera card, cloud sync; `import/lib.rs:105-113`).

### D. Read errors

1. **`Unreadable(io::Error)` in `pg_platform`** (jobs, video already depend on it; add `analysis → platform` to
    `tools/crate-layers.txt:30`, `Cargo.lock`). Created in `pipeline::read_whole`/`read_error` (`pipeline.rs:273,282`),
    `pg_video::Unopened`, and `analysis/analyze.rs:40-41,196` (`file_hash_of`, a bare `io::Error` today).
    Verified: codecs map to `ImageError`, none returns a bare `io::Error`. Passes test `downcast_ref::<Unreadable>()`.
    Tests: a held file (`pg_testkit::held`) for an image and a video classify `Unreadable`; a truncated JPEG does not.
2. **`io_attempts` (DO 5.6)**: counted only while the file's folder is present (`folder_present` moves from writeback's
    private `Inner` to `Catalog`, memoised per folder per flush, checked **outside** write transactions: `is_dir` on a dead
    share blocks); spaced by `next_try_at` (wall clock is fine; only leases need monotonic); `NotFound` is not an attempt
    (`next_try_at = now + 60 s`, wait for the rescan); the third counted attempt writes the terminal row (`store_failure`
    for import, `retry:false` ledger row for borders). Worklists skip `next_try_at > unixepoch()`. Import gets
    `ImportStats.unreadable` and `files_read(Import, …)`; Borders replaces its endless retry (`borders/lib.rs:79-83`).
    Amend DO 5.6's `downcast_ref::<std::io::Error>()` text (it also misses `Unopened`).

### E. Thumbnails and borders

1. `store_borders -> Result<bool>`; gate `done.stored += 1` and the `found_known` push (`borders/lib.rs:71-73,99-100`, the
    push clears the file's ledger row) on `true` (DO 3.5 last bullet).
2. Thumbs: `still_wanted = || heights.get() == Some(height)` replaces the height listener's `request_stop`
    (`runner.rs:182-189`; DO 2.9, race-free, amend line 272/R-85); `is_current` `Err` per item → "not current"
    (`thumbs/lib.rs:231`); `put_batch` `Err` records `cache_error` and ends production without aborting the ledger flush
    (DO 5.8). Record `Other` failures as `retry:true` ledger rows for visibility (optional). **No retry-list query**: the
    `fold`/`first_failed` mechanism is deleted by D35/4.7; the earlier plan cited D31 for it wrongly.
3. **Orphans**: no eager prune (it regresses "remove, then re-add keeps analyses", `app/library.rs:311-314`). When
    `put_batch` reports full, the pass asks the reader (which attaches the catalog, `reader.rs:28`) for entries joined to
    `files.missing = 1` through **its own present-files view** (never `thumb_sources`, which hides unanalysed/failed
    files), calls `forget` per id (`thumbcache/lib.rs:217`), retries once, then reports `cache_full`. Never prune on
    a scan-detected `missing`. `forget_many` (BM 3.5/4.5) replaces the loop later. Eviction itself (`writer.rs:406`) unchanged.

### F. Stuck copy and writeback (BM 4.3 slice)

1. Exclude `file_writes.temp IS NOT NULL` files from `pending_import_sql`, `pending_borders`, `pending_sql` (alias variant
    of `not_being_written!`); keep the store guard as second defence; count them as "waiting for a write", not `changed`.
2. Parking: `NEXT_READY` (`queue.rs:316`) uses `file_writes_ready`; writeback parks the job instead of `return Ok(())` at the
    removed-file-with-temp case and the offline case (`writeback/lib.rs:395`, ~:439; offline is not an attempt, so the
    same temp row came first every drain and starved later jobs); periodic unpark plus `retry_write_copies`
    (`queue.rs:306`) on the same timer, folder check outside write transactions. Document that the status line may flap.

### G. Tests (every item), plus

- Race tests: remove the root between two scan batches, between two pairing batches, and before `begin_scan`; lease takeover
  and `Lost` with the root still present; removal interrupted mid-pager then resumed; re-add during removal.
- `folder_removed`: deterministic test with a pass that blocks on a channel (start a round, call `folder_removed`, release the
  pass, assert queue/busy entries gone and the round stopped before the next pass).
- Retry: `a_failed_round_is_said_and_the_next_round_clears_it` (`runner.rs:709`) asserts the automatic retry without `wake()`
  (base delay an `Inner` field, ~10 ms); a later pass runs after a failing/panicking one; a stop after the failing pass still
  preempts; a preempting wake does not clear `round-failed`.
- Flusher: a poisoned row in a batch of 500 loses only that row; a busy error does not bisect; an FK-failing pair rebuilds.
- Pairing: restored file pairs with files imported while it was missing; `store_pdq_pair`/`pdq_hashes` coverage
  (`raw::set_missing`, `raw::add_pdq_pair`, `raw::insert_analysis` with quality ≥ `MIN_QUALITY`); repair step on a carried
  catalog.
- `IS_CURRENT`: extend `what_was_read_from_a_file_is_kept_only_while_the_file_is_as_listed` (`writers.rs:689`) with
  `missing`, a replace-with-same-size case, and a writeback rewrite (must not livelock: it changes size/mtime itself).

## Docs to amend (same change)

- **BM**: §1.2 rule 4, §1.3 bullet 2, §7 "one BASELINE edit" → migrations (baseline frozen) with TO_10 as the first; D1
  (stale `schema.rs:18-24` claim; index-guard exemption, TO_9 `files_fresh` precedent); D10/3.8 (interim source-pattern
  guard, pager statement allowlisted); D13 (`pdq_hashes` stays until Phase 5); D16/1.9/2.1 (`work_leases` subset now,
  heartbeat thread, adopter-side staleness, `LeaseLost`, `lease_next_id`, columns added later with `ADD COLUMN`); D17
  (`Step::{Done,Preempted,Busy{until}}`, error isolation + backoff, startup/idle/slow rescans, `wake()` doesn't stop);
  D25/Q3 (reword "missing files are ignored"); D34/3.4/2.1 (`root_removals` columns now; interim synchronous pager;
  `add_root` doesn't cancel, the live-root rule replaces it); D35/4.5/4.6 (orphans via `forget`; who owns the one
  cache-layout edit vs the NTFS plan); 2.1 `file_writes`/4.3 (parking slice); Q-D (new deviations: per-folder
  `mark_missing` until the authorizer, synchronous removal, wall-clock `next_try_at`, Q3 pairs, full re-pair after upgrade).
- **DO**: line 11 (batch 100/250 ms); 2.8/3.10 (`idle_work()`); 2.9 (round order vs D17, per-pass isolation replaces
  "an Err propagates", line 272, `cache_error`); 3.3 (TO_10 is the audit superset, `next_try_at`, README note: the code
  refuses an older build, `core.rs:75-80`); 3.5 (REPLACE with `dups_state` named until TO_11, why an upsert would leave
  `pending = 1`: `analysis_changed` doesn't fire on unchanged values); 3.6 (Pairer from `done` rows incl. missing, lazy,
  under the `pair` lease); 3.7 (pairs-only lands now, keyset paging, re-verify); 4.3 (`files_log` `what` as BM);
  5.5 (unify `Ended` on BM 1.8's; `tick()` renews the lease; `Flusher`); 5.6 (typed `Unreadable`, `folder_present`,
  `NotFound`, `next_try_at`); 5.8 (`is_current` `Err`); migrations table 1261-1268.
- **NTFS_MFT_SCAN_PLAN** 279-291/345: `content_gen` in `files` only now (trigger); `analysis.content_gen`, `stale!` and the
  cache `Source` follow later.
- **NOTIFICATIONS.md** (removed; its content is in the crates' module docs): resolver table, scan/lease paragraphs (`Lost`, `work_leases`, per-batch fence), borders ledger
  paragraph (isolation), lines 236-241 (thumbs `Other` now recorded).

## Order of work (one build at the end)

1. TO_10 consts, the Rust repair step, `rebuild.rs` lists, bench `LAYOUT`.
2. Catalog core: lease helper and error variants; `ScanLease` fences, `ScanWriter` fence, `begin_scan` guard; `remove_root`
   pager + `root_removals`; `roots_gen`, `idle_work`; `IS_CURRENT` + `content_gen` plumbing; `store_borders -> bool`;
   `pdq_hashes` (Q3, lazy mark-pending); worklist `temp` exclusion; `dups_state` writers + `pending_pairs`; `io_attempts`
   API; `Catalog::folder_present`; write-parking queries.
3. `pg_platform::Unreadable`, analysis/video mappings, `crate-layers.txt`; jobs: `Step::Busy`, `schedule::round`
   isolation incl. panics, `Flusher`.
4. Scan crate (typed `Lost`, partial `Pairing`) and the CLI match.
5. Passes: import (lease, lazy Pairer with rollback, io attempts, counters after commit, stats on `Err`, pairs-only module,
   size check), borders, thumbs (level check, `Err` handling, orphans), writeback parking.
6. Runner: deadline table, startup/idle/slow rescans, scan retry list, `Removals`, wake-flag take, `Lost` handling, notices.
7. Docs amendments, tests for every item, one build, tests, `just lint`.

## Verification

- `cargo test` over catalog, scan, jobs, import, borders, thumbs, thumbcache, dedup, writeback, library, model, cli
  (native PowerShell, `QT_DIR` on PATH), then `just lint`; Linux CI pass in the pg-dev container only.
- Manual in the app: start a large scan, remove the folder mid-scan (job stops, nothing reappears, grid empties); remove,
  then re-add (analyses and ratings return); add a folder during a round (scanned); remove a root from the CLI while the app
  runs (the app's round stops); rename the catalog file and restore it (recovers without a manual wake); unplug a drive for
  several rounds (files don't become permanent failures); add files while the app is closed (appear on next start); rotate
  photos during a startup rescan (the walk completes).
