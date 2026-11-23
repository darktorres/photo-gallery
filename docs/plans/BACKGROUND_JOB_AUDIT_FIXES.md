# Plan: background job, reevaluated against the current code

> **Status (2026-11-23).** This replaces the 2026-11-21 version, whose designs were written for a single-thread runner,
> a migration slice and a lease table that the code no longer has. Nothing here is built; no tests were run for this
> revision. Claims are marked **verified** (read in the code, as of this date) or **predicted** (reasoned; the check that
> settles it is named).
>
> Owned by other plans, not repeated here: read-error retry, the attempt counter and the failure ledger
> (`IMPORT_FAILURES_PLAN.md` item 5); zero-listing and volume-change refusals, and the waiting-root state
> (`MOUNT_POINTS_PLAN.md`); lanes, the busy requeue, unplugged roots and the pairing cooldown (`MULTI_DRIVE_LANES_PLAN.md`);
> the notification ledger (`NOTIFICATIONS_PLAN.md`).

## Goals

1. A removed folder's files are not touched again, and do not come back.
2. A folder added or scanned while a round runs is scanned.
3. A failing pass or round is retried with spacing and said once. It is cleared by the next round that completes. A
   failure never leaves the job idle until the user does something.
4. Every folder is rescanned at start, and again every 6 hours while the app runs.
5. Stop returns promptly, including during a long video hash.
6. No schema change.

## Current design, as verified

- Each lane (one per physical drive, `lanes.rs`) runs a round of passes in order: Scans, Import, Thumbs, Placeholders,
  Hashes, Pairing, Borders (`lane.rs`, `round`).
- A round stops at the first pass that returns `Err`: `schedule::round` uses `pass.run(cx)?` (`jobs/schedule.rs:24-31`).
- After a failed round a lane waits with no timer. The only timer is the busy retry, and it runs only while some scan is
  busy (`lane.rs:43`).
- A round that ends `Ok` clears `RoundFailed`, including a round that was preempted: the lane calls `tally.round(n, None)`
  for every `Ok(step)` (`lane.rs`, `tally.rs:89-103`).
- `Runner::wake()` calls `wake_all(true)`, which stops every lane's round (`runner.rs:129-133`).
- Removal: `begin_removal` takes the folder's leases and writes the marker (`removal.rs`). The removals service then runs
  `finish_removals`: `stop_scans_under`, `wake_all(true)`, then `removals::finish`, which calls `hide_pages` and
  `drive_removals` and sweeps orphan thumbnails (`coordinator.rs:834-848`, `removals.rs:24-36`).
- Walk leases: `scan_leases` with a wall-clock heartbeat (`unixepoch()`, expiry after `SCAN_LEASE_SECS`, `roots.rs:124`).
  Each write transaction of a walk calls `hold`, which fails if the lease row is gone (`lease.rs`).

## 1. Removal

**Problem (predicted from the order of calls).** `finish_removals` restarts the lanes (`wake_all(true)`,
`coordinator.rs:838`) before `hide_pages` runs (`removals.rs:24-26`). Until `hide_pages` has marked the folder's files
missing, every pass still sees them: the pending queries select by volume and pending state with no root filter
(`work.rs:448`, `pending_files!` in `sql.rs`). A round started in that window can analyse, thumbnail and pair files of a
folder being removed. The delete later removes the rows and the sweep removes the thumbnails, so the end state is right,
but the work is wasted and visible.

**Decision.** Hide before any lane restarts. In `finish_removals`, for each marker, park the lanes of the folder's
volumes, then run `hide_pages`, then release them. `park` already exists for crops and cuts (`coordinator.rs`, the
`park` that waits for the lanes to go idle, then holds them). The delete stays asynchronous in the removals service:
hidden files are invisible to every pass, so nothing else waits for it. Lanes on other volumes keep running.

**Predicted cost.** A removal waits for the current round on that volume to reach its next checkpoint. `hide_pages`
is batched updates, so the wait is short.

**Check.** A test with a pass that blocks on a channel: start a removal while the pass is held, release it, and assert
that no pass touched the removed folder's files after the hide.

## 2. Scans

**Problem (verified in the code, not reproduced).** The `Scans` pass reads the roots once when it starts
(`passes.rs:65`). A popped path that is not under a root in that snapshot is skipped (`passes.rs:70-73`). The app asks
for a scan right after it adds a folder (`app/library.rs:389`). If that folder's scan is popped later in the same pass,
it is dropped with no notice and no retry.

**Decision.** Check membership against the roots for each popped path. Roots are few, so this is one indexed read per
walk.

**Check.** A test that adds a root while a `Scans` pass is walking another folder, and asserts the new root is walked.

## 3. Rounds and retries

**Decisions.**

- **Isolation.** A pass's `Err` is recorded, the later passes still run, and the round returns the first error at its
  end. This changes the contract that `an_error_ends_the_round_with_it` (`jobs/schedule.rs`) pins.
- **One deadline per lane.** The lane waits until the earliest of: the busy retry (5 s, as now), the failure backoff,
  the next retry-due work from `IMPORT_FAILURES_PLAN` item 5, and the next rescan of its roots (section 4).
- **Failure backoff.** 5, 10, 20, 40 and then 60 s, and 60 s from then on, with no give-up. A `Done` round resets it. A
  `Preempted` round is not a failure: no backoff, and the lane runs again at once.
- **Clearing.** `RoundFailed` is cleared only by a `Done` round, not a preempted one.
- **Panics.** Unchanged: `round_caught` reports the panic and reopens the catalog.
- **Wake.** `Runner::wake()` wakes the lanes without stopping their rounds. Scan requests keep stopping their lane's
  round, as `route_scans` does today (`coordinator.rs:512`): the user waits for those. `wake_files` is unchanged.

**Predicted.** No current caller of `wake()` depends on the stop. Check each call site when it is changed:
`app/library.rs:211` (`wake_job`) is the one known caller.

## 4. Rescans

**Decision (the user's, from the earlier round).** Every root is scanned once at start. After that a root is due when
its last finished scan is 6 hours old. `scans.finished` already records the finish time, so no schema change is needed.
The coordinator checks for due roots at start and every 10 minutes, and requests a scan for each one that is due.

**No idle probe.** A round that ends `Done` means every pass had nothing left to do. The only work that appears without a
wake is retry-due work (`IMPORT_FAILURES_PLAN` item 5) and the rescans, and both feed the lane deadline in section 3. So
the old `idle_work()` probe of `EXISTS` queries is not needed.

**A root that keeps failing.** Its last finished scan stays old, so it is requested again at each check. Its failure is
a keyed notice (section 9), so the repeat replaces the notice rather than adding one.

**Predicted cost.** A full rescan of a large library every 6 hours. The cost is the scan's own; the cadence is a
parameter.

## 5. Failed walks and leases

**Problems (verified).**

- A lease lost during a walk (`LeaseLost`) becomes `ScanError::Failed`. It posts `ScanFailed`, and the root is not asked
  for again (`passes.rs`, the `Failed` arm).
- The pairing runs after the walk's batches with `?` (`scan/lib.rs:1263`). An error there drops the `Pairing` value that
  earlier batches already committed. The thumbnails of the replaced files are then not forgotten, and their temp files
  are not discarded. Only the `Ok` arm does either (`passes.rs`, around the `forget_replaced` and `discard_temps` calls).

**Decisions.**

- **Leases stay as they are.** The wall-clock heartbeat and the per-transaction `hold` stay. The `work_leases` table, the
  fence token, the monotonic counter and a heartbeat thread are not built. Every write checks the lease inside its own
  transaction, so an expired walk writes nothing more and the data stays right. Expiry costs one restarted walk; a
  monotonic counter would only avoid that restart after a suspend.
- **Requeue on loss.** `LeaseLost` requeues the root through the lane backoff, if the root is still a library folder. The
  first loss posts no notice. Three losses in a row post a keyed `ScanFailed`.
- **Carry the partial pairing.** A pairing error returns the `Pairing` done so far, inside `ScanError::Failed`. The `Failed`
  arm then does the same forgetting and discarding as the `Ok` arm.
- **`ScanWriter` coverage: no change.** `ScanWriter::new` takes `hold` first, so a writer for a removed root never gets
  past it. The `covering: None` branch in `folder_failed` is then unreachable while the lease is held (predicted from
  reading; confirm with the existing removal-during-walk test,
  `scan/lib.rs` `a_folder_removed_while_it_is_walked_is_not_written_again`).

## 6. Pairing load

**Problem (verified).** A stored hash that `from_blob` rejects fails the whole load (`work.rs:300-301`). The `?` at
`work.rs:333` and `work.rs:391` makes the pairing pass fail on every round.

**Decision.** Skip that row, count it in `PairStats`, and log it once per pass. Only this build's encoder writes stored
hashes, so a malformed one means damage. A re-analysis path for it is not worth building.

**Not built.** The old design's lazy `PdqIndex`, rollback and mark-pending. The pass builds its join from the stored
hashes on each run (`import/pairs.rs`, `Joiner::new_on`), and a failed write fails the pass, so nothing persists across a
retry to roll back.

## 7. Stop latency, and the import size check

**Stop latency (verified).** The video hash streams the whole file in one call (`analysis/analyze.rs:37-41`, `update_reader`)
with no stop check. The job is abandoned after `JOB_STOP_WAIT` of 5 s (`app/library.rs:36,191-193`). `IMPORT_FAILURES_PLAN`
row B lists stop latency as open and gives other causes.

**Decision.** Hash a video in chunks of 1 MiB, with a stop checkpoint between chunks. A stop leaves the file unhashed, so it
stays owed and the next round hashes it.

**Bytes against the listed size (verified: no comparison).** The import reads the file's bytes and never compares their
length with `item.size` (`import/lib.rs`, the read path). A file that changes while it is being copied is stored with the
size it had when listed.

**Decision.** A length mismatch counts as `changed`. Nothing is stored, and the file stays pending.

**Predicted.** A file that is still being copied is then not stored with a partial analysis. Check with a test that copies
a file in two halves.

## 8. Writes in progress

**Verified.** The pending queries do not exclude files with a write in progress (`file_writes.temp`), and the import
stores nothing for such a file (its `IS_CURRENT` check fails). The file stays pending.

**Decision: no change to the queries.** A turned file is re-analysed by the app's `wake_turned`, which runs once the
writeback has recorded the turn (`writeback/lib.rs:277-279`, `app/library.rs:204-206`). Ratings do not change the
analysis. So the skip corrects itself for turns.

**Predicted.** Check by turning a photo while a round is running that imports it.

## 9. Notices

**Verified.** `RoundFailed` is a state, resolved by `NextSuccess(Op::Round)` (`model/src/notice.rs:1362`). `RoundPanicked`
and `ScanFailed` are transient events (`notice.rs`, around line 1386). A root that keeps failing posts a new `ScanFailed`
each time.

**Decisions.**

- `RoundPanicked` becomes a state resolved by `NextSuccess(Op::Round)`, like `RoundFailed`.
- `ScanFailed` becomes keyed per root, with `Key::all("scan-failed", root)`. It is cleared when a complete scan of that root
  finishes, and a repeated failure replaces the entry rather than adding one.
- When the resolver changes, regenerate the golden table and keep `the_resolver_table_is_the_golden_one` passing.
- A root whose folder cannot be opened is shown by the waiting-root state in `MOUNT_POINTS_PLAN`. This plan adds no second
  state for it.

## 10. Thumbnails and pass errors

**Verified.** The thumbnail cache evicts at store time to make room (`thumbcache/lib.rs:225-228`, the writer). The
removal sweep drops orphan thumbnails (`removals.rs:36`). So an orphan prune has nothing to do. `cache_full` means eviction
could not make room while sparing the current height.

**Decision.** No change. The errors of `put_batch` and of the `currents` check fail the pass, and the round backoff retries
it.

## 11. Batch sizes

**Verified.** Two constants share the name `WRITE_BATCH`: 500, public, in `catalog/src/lib.rs:23` (import, borders and
pairing use it), and 1000, `pub(crate)`, in `types.rs:122` (removal uses it).

**Decision.** One value, 500, for all of them. Removal makes twice as many transactions as now. Measure in the removal
bench, and change the value only if it regresses.

## Not built, and why

- **Flusher** (bisecting a failed write batch to find the bad item): no per-item write failure has been reproduced. Each
  item is checked against the catalog inside the batch's transaction, so a stale item is skipped rather than failing the
  batch. A batch failure is a round error.
- **Work-lease table, fence token, heartbeat thread**: section 5.
- **Idle-work probe**: section 4.
- **Pending-query exclusion for writes in progress**: section 8.
- **Orphan prune in the thumbnail cache**: section 10.

## Schema

No change. `scans.finished` already holds the rescan time, and the leases, roots and failure tables stay as they are.
The migration question in the earlier version does not arise.

## Order of work (one build at the end)

1. Removal: park, hide, release (`coordinator.rs`, `removals.rs`). Test.
2. Scans: roots per popped path (`passes.rs`). Test.
3. Rounds: isolation (`jobs/schedule.rs`); the lane deadline and backoff (`lane.rs`); the clear rule; `wake()` without a stop
   (`runner.rs`, `coordinator.rs`). Change the tests named in section 3.
4. Rescans: the due check at start and every 10 minutes (`coordinator.rs`), with a query on `scans.finished` (catalog).
5. Leases: `LeaseLost` requeue; the partial pairing carried in the error (`scan/lib.rs`, `passes.rs`).
6. Pairing load: skip and count a malformed hash (`catalog/src/work.rs`).
7. Import size check (`import/lib.rs`); chunked video hash with a stop checkpoint (`analysis/analyze.rs`).
8. `WRITE_BATCH`: one constant (`catalog/src/lib.rs`, `types.rs`).
9. Notices: `RoundPanicked` as a state, `ScanFailed` keyed per root; the golden table.
10. Tests (below), one `cargo build`, `cargo test` for the crates touched, then `just lint`.

## Tests

**Changed.** `an_error_ends_the_round_with_it` (new isolation contract). `a_failed_round_is_said_and_the_next_round_clears_it`
(retries without a wake; cleared by a `Done` round, not a preempted one).

**New.**

- A root added while a `Scans` pass walks another folder is walked.
- A removed folder's files are not touched while a blocked pass holds the removal before the hide.
- A `LeaseLost` requeues the root, and the replaced thumbnails of the partial pairing are still forgotten.
- A malformed stored hash is skipped and counted, and the pairing pass still completes.
- A size mismatch between the bytes read and the listed size stores nothing.
- A video hash stops within one chunk.
- Rescan at start, and again once the 6-hour age has passed (the age is a parameter, so the test does not wait).

## Verification

- Native PowerShell build with `PG_QT_DIR` on `PATH`; `cargo test` for `pg_jobs`, `pg_library`, `pg_catalog`, `pg_scan`,
  `pg_import`, `pg_analysis`, `pg_thumbs`, `pg_writeback`; `just lint`. Linux CI runs in the pg-dev container only.
- Manual, in the app: remove a folder during a large scan (the job stops, nothing reappears, the grid empties); remove,
  then re-add (analyses and ratings return); add a folder during a round (it is scanned); quit during a long video hash
  (the quit is prompt); rotate photos during the startup rescan (the walk completes).

## Standing rules

One build at the end, not between steps. No new branch. No commit unless asked. The workspace lints apply: no indexing,
`unwrap`, `expect`, `panic` or bare casts; `pg_model::num` for conversions; `pub(crate)` for internal items;
`checked_mul`/`saturating_mul` for backoff arithmetic.
