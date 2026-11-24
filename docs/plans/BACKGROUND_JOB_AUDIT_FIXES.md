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
4. Scans run when a folder is added or the user asks for a rescan. There are no background rescans; change detection
   is the NTFS_MFT_SCAN_PLAN's.
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
  `drive_removals` and sweeps orphan thumbnails (`coordinator.rs:834-848`, `removals.rs:18-36`).
- The removals thread starts with the library, and its first pass runs at once; the lanes start at the same time
  (`coordinator.rs:768-800`).
- Walk leases: `scan_leases` with a wall-clock heartbeat (`unixepoch()`, expiry after `SCAN_LEASE_SECS`, `roots.rs:124`).
  Each write transaction of a walk calls `hold`, which fails if the lease row is gone (`lease.rs`).

## 1. Removal

**Invariant.** From the moment a removal begins, no lane starts work on a file of the removed folder, and no write lands
on one. A removal may leave work to finish; it must not leave a window in which new work starts.

**Current design (verified).**

- `begin_removal` is one transaction: it deletes the roots row, releases the folder's leases, and inserts the marker
  (`removal.rs:82-102`). The folder's files stay `missing = 0`.
- `hide_pages` marks them missing afterwards, in batches of `WRITE_BATCH` (`removal.rs:104-119`).
- The removals service runs `wake_all(true)` before `hide_pages` (`coordinator.rs:834-842`, `removals.rs:24-26`).
- The pass work queries select by volume and pending state with no root filter (`work.rs:448`, `pending_files!`).
- Writes are guarded by `missing = 0` in SQL: analysis, borders and failures through `IS_CURRENT`
  (`writers.rs:450,780`), thumbnails through `currents` (`lookup.rs:35`, `current.rs`), hashes and placeholders through
  `IS_CURRENT` (`writers.rs:506-508`, `writers.rs:700`).
- A lane that is parked (`coordinator.rs:589`, the `park` used by crops and cuts) is held, asked to stop, and waited for
  until its round has ended (`inner.rs:121`, `wait_idle`). A round's writes are committed by then.

**Problems.**

- **P1 (verified ordering; predicted effect).** Between `begin_removal` and the last hide batch, the folder's files are
  still `missing = 0`. A lane restarted by `wake_all(true)` selects them as work, and its writes are accepted. The result
  is wasted decode and thumbnails, removed later by the delete and the sweep. The end state is right; the work is not.
  Check: log the import count of a removed folder's files during a removal.
- **P2 (verified).** `wake_all(true)` stops every lane, including lanes on drives the folder is not on. That is churn on
  other drives, not a correctness problem.
- **P4 (verified ordering; predicted effect).** A removal left by an earlier run (a marker with its folder not yet
  hidden) is finished by the removals thread, which starts with the library. The lanes start at the same time
  (`coordinator.rs:768-800`), so a lane's first round can select the folder's files before the thread hides them. That
  is the same window as P1, after a crash or a quit.
- **P3 (refuted).** A pair write does not fail on a partner the delete has removed. The delete removes the partner's
  analysis through the file's foreign-key cascade. SQLite runs a cascade as a trigger program whose delete step compiles
  through the normal delete path, which fires the child table's triggers: `fkActionTrigger`, `codeTriggerProgram` and
  `sqlite3DeleteFrom` in the bundled `sqlite3.c` of SQLite 3.53.2 (the newest copy in the local cargo registry). The
  project's rusqlite 0.43 is not in the registry, so the same path is assumed for it; the regression test below covers
  that. The `analysis_pairing_removed` trigger bumps the epoch, and the guarded write skips every picture whose analysis
  is gone.

**Measurements (2026-11-23, the real catalog code, E: hard disk, 10,000 files from the synthetic tree walked by the real
scanner; temporary probes, since removed).**

- `begin_removal` (roots row, leases, marker): 0.18 ms.
- `hide_pages`, batches of `WRITE_BATCH`: 350–410 ms, medians over 5 runs.
- One transaction for the hide: 237–274 ms. Dropping `files_source_changed` saves some of it, to 187–253 ms. Dropping
  `file_failures_hidden` makes no clear difference. The variants are within the disk's noise, about ±20%.
- The cost is the row write itself, about 25 µs per file on E:.
- The bench `ir::removal::folder_removal::ten_thousand` (hide and delete, in memory, callgrind): 1,696,398,183
  instructions. The hide alone, `ir::removal::folder_hide::ten_thousand`: 796,777,538. The hide is about 47% of the
  removal.
- A Python prototype of a bulk hide (a flag, the trigger skipping its body, one set-based update) looked twice as fast.
  On the real code it was not faster, so no result from the Python harness is used here.
- The `changed` column is load-bearing for scans (`prior_states`, `roots.rs:287-295`: a file's old row is a move source
  only while `changed` is at or below the scan's mark). A hide during a scan of another folder must bump it, so the per-row
  trigger body stays. `seq` is not load-bearing for a hide: the thumbnail view `thumb_sources` filters `missing = 0`.

**Decisions.**

- **D1. The hide stays in the removal job, in batches, under a park of the folder's lanes.** The sequence:
  1. `begin_removal`, as now: the roots row, the leases and the marker in one transaction. It returns the volumes of the
     folder's present files: `Begun::Started { path, volumes }`. Nothing is hidden yet, and the UI does not wait for
     anything.
  2. The removals job parks the lanes on those volumes (`park`, `coordinator.rs:589`): it holds them, asks them to stop,
     and waits until each has finished its round. Lanes on other drives keep running (fixes P2).
  3. `hide_pages`, in batches of `WRITE_BATCH`, as now.
  4. Release the holds.
  5. `drive_removals`, as now. The delete needs no park: the hidden files are invisible to every pass.
  
  Why there is no window: while the lanes are parked, no round runs on the volume, so no pass selects the folder's files
  or writes to them. The rounds that were running had committed their writes before `wait_idle` returned. Once the holds
  are released, every pass sees the hidden rows.
- **D2. Only the folder's volumes are parked.** `wake_all(true)` leaves `finish_removals`. If the folder has no present
  file, or a present file with no volume, every lane is parked, as `wake_files` does for a file with no volume
  (`coordinator.rs:330-331`).
- **D3. No change to the pair write, unless the regression test fails.** The epoch guard already skips a deleted partner.
  The rule stays as the rest of the audit states it: every write that names a file row checks that row in its own write
  transaction. If the regression test fails, the fallback is `INSERT INTO pdq_pairs SELECT ?1, ?2, ?3 WHERE EXISTS
  (files a) AND EXISTS (files b)` with the same `ON CONFLICT`. It checks existence, not presence, so a pair with a missing
  but existing file is still kept (Q3, `present_pdq_pairs`).
- **D4. The grid is told when the hide is done.** The removals job publishes `Change::Scanned` after the hide and again
  after the delete (`coordinator.rs:847` is the second). The desktop's immediate reload when `remove_folder` returns
  `Started` (`desktop/library.rs:287`) runs before the hide, so it shows the folder; it is removed, and the roots list is
  still reloaded there. With no library running, `app/library.rs:401-412` finishes the removal synchronously, so the
  reload sees the final state.
- **D5. Removals finish before the lanes start working.** A root-level hold does not reach the lanes: `wait_while_held`
  checks only the lane's own holds (`jobs/control.rs:513-530`), and each lane has its own control (`spawn_lane`,
  `library/src/coordinator.rs:557-565`). So the barrier is a hold per lane. `Lane` stores the hold it was given at spawn,
  and `Lanes` has a barrier flag that makes `spawn_lane` take one. The lanes are spawned by the coordinator's first route,
  which runs beside the removals thread (`coordinator.rs:768-800`). The removals thread's first pass, which finishes any
  marker left by an earlier run, clears the barrier and releases the holds of each volume once its folders are hidden. A
  volume with no marker is released at once. If that pass fails, the volumes of its markers stay held, the failure is
  posted, and the pass runs again at the recheck (`REMOVAL_RECHECK`). This closes P4.
- **D6. A lane spawned during a removal starts held.** `spawn_lane` makes a new lane with its own control and no hold
  (`coordinator.rs:557-565`), and `assign_lanes` spawns lanes when a folder group has no lane (`coordinator.rs:440-480`).
  A drive plugged in while a hide runs can therefore get a lane that is not parked, and that lane can work on the folder's
  files before they are hidden. The fix is the same mechanism as D5: `Lanes` records the volumes a removal has parked, and
  `spawn_lane` takes a hold on a new lane whose volume is in that set. The removal releases its volumes at the end of the
  hide. Confirmed by reading; no test yet.
- **Trace notes (2026-11-23, by reading).** A park returns no holds when a stop arrives while it waits
  (`coordinator.rs:603-607`), and the holds taken so far are dropped. The removal job must treat that as "try again at the
  recheck", and the marker stays. The crop and cut jobs park the same lanes with their own stop controls. A lane never
  waits on a job, so overlapping holds cannot form a cycle; the holds count, and a stop set by one park is cleared only
  when the lane runs again. A test with two overlapping parks is still wanted. Writeback is not a lane and is not parked.
  Its drain checks `file_gone` and `under_marker` when it picks a job (`writeback/lib.rs`, around line 396), and a copy that
  has started finishes while the folder is still there. A hide that lands after the pick therefore leaves the copy to
  finish, which the removal rule keeps the temp for. Confirmed by reading; not tested.

**Unchanged.** The delete (`drive_removals`), the sweep of orphan thumbnails, the rule that a file with a recorded write
is kept, and the resume after a crash. A marker whose folder is not hidden yet is the normal state between `begin_removal`
and the hide, so `Begun::Pending` and the resume are unchanged.

**Park latency (measured, 2026-11-23, a bound, not a test of a park).** The park waits for each lane's current item to
  stop, and the video hash cannot stop inside its one read (section 7). On E:, a 2 GB file read unbuffered and
  sequentially takes 12.6–12.9 s (160 MB/s, three runs). So the worst case today is the time to read the largest video
  in the library: about 6 s per GB. With the chunked hash of section 7 the bound becomes one chunk, a few milliseconds
  here. The park test in the checks below measures the real wait once D1 is built.

**Cost (predicted).** The removal parks its volume's lanes for the length of the batched hide: about 3.5–4 s per 100,000
files on E:, by linear extrapolation from the measurement above. The begin itself is 0.18 ms. The park also waits for each
lane's current item to reach a stop checkpoint. Stop checks are per item in the passes (`IMPORT_FAILURES_PLAN` row B), so
the wait is short except for a long video hash, which section 7 fixes.

**Checks (predicted; each names the test that settles it).**

- D1 park: a pass blocked on a channel on the folder's volume. Start a removal: the pass is stopped and no pass runs on the
  folder's files until the hold is released. A lane on another volume keeps running.
- D1 wait: time the park on a library with a long import in progress; the wait is the stop latency of that import's
  current item, and should be well under a second.
- D5: a marker left by an earlier run. Launch: no lane round starts on the folder's volume before its folders are hidden.
- P3 regression test, without timing: load the pairing work, delete a partner's file through `drive_removals` (so the
  delete cascades), then write a pair with it. Expected: no error and no pair written. Add it to the suite as a guard. If
  it fails, D3's fallback applies.
- D2: a blocked pass on the folder's volume stops, and a lane on another volume does not.
- D6: a lane spawned for the folder's volume while the hide is parked starts held and runs no pass until the release.
- Overlapping parks (a crop or cut and a removal on the same volume): both holds stand, and the lanes run only after both
  are released.
- `begin_removal` returns the volumes: the call sites change with the `Begun::Started` variant. They are
  `app/library.rs:401`, `bench/benches/ir/catalog.rs:278`, `catalog/src/roots.rs:101`, `cli/src/lib.rs:470-487` and its
  test at `:1007`, and `desktop/src/library.rs:285`.
- The existing removal tests (`removal.rs`) stay as they are: the hide is still a separate step after `begin_removal`, so
  `a_removal_begun_and_not_hidden_keeps_its_rows_until_the_next_run` and
  `a_removal_left_part_hidden_is_finished_by_the_next_run` describe real states.
- `remove_root` (`roots.rs:98-108`) still runs begin, hide and drive in sequence. It is used by tests and by
  `inbox.rs:622`, not by the app. It needs no change.

## 2. Scans

**Problem (verified by reading; not reproduced).** A request for a new folder can be dropped in two places, and both
come from a root list read too early. The app adds the folder and then asks for its scan (`app/library.rs:389`).

- `route_scans` gets its root list at the start of `route` (`coordinator.rs:372`), and takes the pending requests later
  (`coordinator.rs:502-505`). A request for a folder added in between is matched against the old list, and dropped
  (`continue`).
- The `Scans` pass reads the roots when it starts (`passes.rs:65`). A path popped later in the same pass is skipped if its
  root is not in that snapshot (`passes.rs:70-73`). Each routed request calls `request_stop` on its lane
  (`coordinator.rs:512`), but the pass does not check for a stop between paths (its loop pops the next path at once).
  So when the walk in progress completes after the stop, the pass pops the new folder, drops it, and returns `Done`.
  Nothing retries it. The window is small folders finishing while the request arrives; a large walk stops at a checkpoint
  first and the next round reads the roots again.

**Decisions.**

- `route_scans` takes the pending requests first, and reads the roots after. The app adds a root before it requests its
  scan, so every request taken is for a root already in the catalog, and the roots read afterwards include it.
- The `Scans` pass reads the roots for each popped path, not once. Roots are few, so this is one indexed read per walk.
  A path under no root at its pop is still dropped: that is a removed folder, and its scans are stopped on removal.
- The notes that a complete walk replaces (`passes.rs:111`) use the same fresh roots.

**Checks (predicted).**

- A root added while a `Scans` pass is walking a small folder, its request arriving as that walk completes: the new root is
  walked, not dropped. The walk's batch hook used by the scan tests (`after_listing`) can add the root and the request at
  the right moment.
- `route_scans`: a request taken after its root's add is routed to the lane, not dropped (a test that calls it with the
  roots read after the take).

## 3. Rounds and retries

**Decisions.**

- **Isolation.** A pass's `Err` is recorded, the later passes still run, and the round returns the first error at its
  end. This changes the contract that `an_error_ends_the_round_with_it` (`jobs/schedule.rs`) pins.
- **One deadline per lane.** The lane waits until the earliest of: the busy retry (5 s, as now), the failure backoff,
  and the next retry-due work from `IMPORT_FAILURES_PLAN` item 5. That last term is not built, so until it is the deadline
  has the busy retry and the backoff only.
- **Failure backoff.** 5, 10, 20, 40 and then 60 s, and 60 s from then on, with no give-up. A `Done` round resets it. A
  `Preempted` round is not a failure: no backoff, and the lane runs again at once.
- **Clearing.** `RoundFailed` is cleared only by a `Done` round, not a preempted one.
- **Panics.** Unchanged: `round_caught` reports the panic and reopens the catalog.
- **Wake: no change.** `Runner::wake()` keeps stopping the rounds (`runner.rs:129-133`). The earlier reason to stop
  nothing was that a wake restarted a non-resumable walk from its root, and a startup rescan would starve. Rescans are
  manual now (section 4), and a walk resumes where it stopped (`passes.rs`, the `Scans` pass pushes the folder back to
  the front). So a stop costs the batch in flight, and each pass resumes from its last committed batch (predicted per
  pass; check each one's batch commit when the stop is tested).
- **A user wake runs at once.** A wake interrupts the failure backoff and runs the round at once. It does not reset the
  failure count, which resets only on a `Done` round. The wake's callers are the Retry of a pass's failed files
  (`desktop/notices.rs:650`) and the thumbnail-cache setting (`desktop/settings.rs:88`).
- `wake_files` is unchanged.

**Verified.** A wake that arrives during a round is kept: `Waker::wait` takes the flag, so the next wait returns at once
(`jobs/service.rs`, the `woken` flag), and the lane runs one more round. No wake is lost, and no take-the-flag method is
needed. The cost of a wake during a round is one round that may find nothing to do.

## 4. Rescans: manual only

**Decision.** No background rescans: not at start, not on a timer, not when idle. A scan runs when a folder is added, or
when the user asks for a rescan (the existing "Rescan all" in `LibraryPanel.qml`, and the per-add scan in
`app/library.rs`). Showing when each root was last scanned is a UI follow-up, not part of this plan. Its data is already
stored (`scans.finished`).

**Why.** A full walk of a large HDD volume pays a seek per folder (`NTFS_MFT_SCAN_PLAN.md`, Context: E: has about 3.2M
folders), so periodic full walks are the cost that plan exists to remove. Detecting changes without a walk belongs to its
USN journal passes, which the rescan button then falls back from.

**Consequence.** Files changed outside the app are seen at the next add or rescan, not before. That is the accepted gap
until the NTFS plan lands.

**No idle probe.** A round that ends `Done` means no stop was requested during it, and every pass either finished or is
switched off by a setting. A stop makes the round `Preempted` (`schedule::round` checks after each pass). A pass that is
switched off has work owed: the thumbnails while the cache is off or no height is set, the duplicate hashes while
duplicates are off. Turning the setting on wakes the lanes (the height and feature listeners, `runner.rs:75-84`, and the
cache setting, `desktop/settings.rs:88`). So nothing waits on a timer for that work.

The timers the job runs are the busy retry (5 s, `lane.rs:26`), the failure backoff (section 3), the removal recheck
(30 s, `coordinator.rs:150`) and the drive probes (5 s and 15 s, `coordinator.rs:44-46`). Nothing else polls. So work that
a command line run leaves in the catalog is seen at the next wake or add, not before. Retry-due work from
`IMPORT_FAILURES_PLAN` item 5 will need a timer of its own; it is not built. Those are the reasons no idle probe is needed.

## 5. Failed walks and leases

**Problems (verified).**

- A lease lost during a walk (`LeaseLost`) becomes `ScanError::Failed`. It posts `ScanFailed`, and the root is not asked
  for again (`passes.rs`, the `Failed` arm).
- The pairing runs after the walk's batches with `?` (`scan/lib.rs:1263`). An error there drops the `Pairing` value that
  earlier batches already committed. The thumbnails of the replaced files are then not forgotten, and their temp files
  are not discarded. Only the `Ok` arm does either (`passes.rs`, around the `forget_replaced` and `discard_temps` calls).
  The replacement that a committed batch ran has already cleared the temp path in the catalog, in the same transaction
  (`replace`, `pair.rs:530-552`). So after the walk fails, that path survives only in `Pairing.temps`, and the file it
  names leaks on disk.
- **Verified: the hold is the first statement of every lease-guarded write.** `ScanWriter::new` (`writers.rs:139`), the
  pair batches (`pair.rs:400`), the two removal writes (`roots.rs:203,219`), and the renewal (`core.rs:582`). A walk whose
  lease is gone therefore fails before it writes anything in that transaction.

**Decisions.**

- **Leases stay as they are.** The wall-clock heartbeat and the per-transaction `hold` stay. The `work_leases` table, the
  fence token, the monotonic counter and a heartbeat thread are not built. Every write checks the lease inside its own
  transaction, so an expired walk writes nothing more and the data stays right. Expiry costs one restarted walk; a
  monotonic counter would only avoid that restart after a suspend.
- **Requeue on loss.** `LeaseLost` puts the root back on the busy list, so the lane walks it again after the busy retry
  (`walk_busy`, `inner.rs`; 5 s, `lane.rs:26`). No per-root backoff is needed. A lost lease always means the root is still
  covered: a removal releases the lease and the walk gets `RootRemoved` instead (`lease.rs:34-35`, `scan/lib.rs:926-940`).
  The walk starts again from its root, since its progress was not committed as a resumable state. The first loss posts no
  notice. Three losses of the same root in a row post a keyed `ScanFailed`, and a complete scan of the root clears the
  count.
- **Carry the partial pairing.** A pairing error returns the `Pairing` done so far, inside `ScanError::Failed`. The `Failed`
  arm then does the same forgetting and discarding as the `Ok` arm.
- **`ScanWriter` coverage: no change.** `ScanWriter::new` takes `hold` first, so a writer for a removed root never gets
  past it. The `covering: None` branch in `folder_failed` is then unreachable while the lease is held (predicted from
  reading; confirm with the existing removal-during-walk test,
  `scan/lib.rs` `a_folder_removed_while_it_is_walked_is_not_written_again`).

**Checks (predicted).**

- A lease that expires during a walk (the fixture's `age_leases`, `fixture/state.rs:55`, called from the walk's batch
  hook): the walk fails with `LeaseLost`, writes nothing after it, and the root is walked again after the busy retry.
- The partial pairing: a walk whose pairing commits one replacement batch and then loses its lease. Before the fix, the
  temp path of the committed batch is in no list the library sees: the file leaks. After the fix, the library forgets the
  thumbnails and discards the temp.

## 6. Pairing load

**Problem (verified).** A stored hash that `from_blob` rejects (its length is not `BLOB_BYTES`, `model/src/pdq.rs:134-139`)
fails the whole load (`work.rs:300-301`, through the `?` at `work.rs:333` and `work.rs:391`). The pairing pass returns
that error (`passes.rs`, the `Pairing` arm, `Err(e.into())`), so the round fails until the row changes.

The stored hashes are not only this build's. `PAIRING_VERSION` exists so that another build can prepare the catalog. The
load reads the rows (after the marks, `work.rs:321`), and only then does `pair_with` check the version
(`import/pairs.rs`, after `pairing_work_split`). So a row another build wrote in another layout fails the load first.

**Decisions.**

- **Skip and count, do not fail.** A row `from_blob` rejects is left out of the load, and counted in `PairStats`. It is
  logged once per pass.
- **Re-analyse the picture.** Skipping alone is not enough. The picture stays owed (`pairs_owed`), and the pairing pass
  runs whenever anything is owed (`import/pairs.rs:142`). Each round would load every picture again and log the same row.
  So the load keeps the malformed ids in `PairingWork`, and the pairs write transaction clears their `pairs_owed` rows
  and sets `files.pending = 1`. The import then re-analyses them, and the analysis write replaces the row
  (`INSERT OR REPLACE`, `writers.rs:406`). The clearing and the marking are in the same write transaction as the pair
  writes.
- **Check the version first.** The version is checked before the rows are read, so another build's layout is never
  read. The marks are read first, in the same snapshot (`pairing_marks`, `work.rs:321`).

**Checks (predicted).**

- A raw row with a truncated blob: `pair` fails today. After the fix it completes, counts one malformed row, the picture
  is pending, and its `pairs_owed` row is gone.
- Another build's marks (a different `PAIRING_VERSION`) with a row in another layout: the load reads no hashes.

**Not built.** The old design's lazy `PdqIndex`, rollback and mark-pending. The pass builds its join from the stored
hashes on each run (`import/pairs.rs`, `Joiner::new_on`), and a failed write fails the pass, so nothing persists across a
retry to roll back.

## 7. Stop latency, and the import size check

**Stop latency (verified).** The video hash streams the whole file in one call (`analysis/analyze.rs:37-41`, `update_reader`)
with no stop check. The job is abandoned after `JOB_STOP_WAIT` of 5 s (`app/library.rs:36,191-193`). `IMPORT_FAILURES_PLAN`
row B lists stop latency as open and gives other causes.

The hash pass calls it from its process step (`import/hashes.rs:72`). An error from that step is recorded as a failure
(`flush`, `import/hashes.rs`, from line 90), so a stop must not come back as an error: the chunk loop would then mark the
video as failed.

**Decision.** Hash a video in chunks of 1 MiB, checking the stop request between chunks. A stop returns "not hashed yet"
(`Ok(None)`), which leaves the file unhashed and owed: it is not put in the batch, and the pass ends on its own stop. The
next round hashes it. Predicted cost of a stop: at most one chunk, a few milliseconds at 160 MB/s.

**Bytes against the listed size: not that check.** Comparing the bytes read with the listed size would refuse every large
JPEG and PNG, because the import gathers only a header or the chunks it needs (`pipeline.rs:104-109`, the `Gather` read;
`container/src/lib.rs:199,303`). Those lengths differ on purpose. A video's bytes are not read at all (`Streamed`).
So the check applies only to whole-file reads (`Read::File`).

**Decision.** For a whole-file read, compare the bytes with the file's length at open, which `sized` takes from the file's
metadata (`pipeline.rs:621`). A mismatch means the file changed while it was read, so it counts as `changed`:
nothing is stored, and the file stays pending. The scanner corrects a file that changed after listing anyway: its next
scan sees the new size and mtime and sets it pending again. So the check's only gain is not storing an analysis of
partial bytes. Small, but it keeps a stored analysis tied to the bytes it was made from.

**Predicted.** A file that is still being copied is then not stored with a partial analysis. Check with a test that copies
a file in two halves.

## 8. Writes in progress

**Verified.** The pending queries do not exclude files with a write in progress (`file_writes.temp`), and the import
stores nothing for such a file (its `IS_CURRENT` check fails). The file stays pending.

**Verified, by reading the write path.**

- Queuing a turn changes only `file_writes` (`queue.rs:106-140`). The file's turns and bytes are unchanged until the write
  completes, so no analysis is made against turns the file does not have yet.
- Completing a turn adds the turns to the catalog (`WriteEffect::KeptInCatalog`, `queue.rs:258-282`) and records it
  (`turned_once`, `writeback/lib.rs:190-193`). That publishes `Change::Turned`, and the desktop calls `wake_turned` on it
  (`desktop/library.rs:190-198`, `app/library.rs:204-206`). The lanes of the file's volume then re-analyse it.
- A rating rewrites the file (`FileRating`, `file_rewritten`). Its analysis is kept only when the file's bytes hash to the
  same value as before, so a metadata-only change does not re-analyse (`queue.rs`, `file_rewritten`, `metadata_only`).
  Otherwise the analysis is left stale, and the import redoes it.
- A border cut is not a write kind (`model/src/write.rs` has ratings and rotations only). It rewrites pixels in place, and
  the cut job parks the lanes of the file's volumes while it works (`coordinator.rs`, the `Job::Save` and `Job::Cut` arms
  call `park`). So it is protected by the park of section 1, not by the temp check.

So the skip corrects itself for turns: the file is pending while its copy is in progress, and the completion wakes the
lanes.

**Predicted.** Check by turning a photo while a round is running that imports it.

## 9. Notices

**Verified.**

- `RoundFailed` is a state, resolved by `NextSuccess(Op::Round)` (`model/src/notice.rs:1362`).
- `RoundPanicked` and `ScanFailed` are events: the lifecycle arm that lists them returns `Lifecycle::Event`
  (`notice.rs:1385-1408`). Each post is a new event.
- A panic is posted straight to the notices (`lane.rs:70`, `self.notify(...)`), not through the tally that keeps
  `RoundFailed` per lane (`tally.rs:89-103`).
- The resolver for a condition of the library folder already exists: `Resolver::RootWalk`, which a complete walk of the
  whole folder clears (`notice.rs:906-908`). The scan notes use it, keyed `(root, code)` (`notice.rs:1347`).
- `ScanFailed` is posted from four places. One is a walk that fails (`passes.rs:125-128`). The others are the thumbnails
  of replaced files and the temp files that could not be deleted (`passes.rs:89-101`). Those are one-off failures of the
  scan's aftermath, not conditions of the root.

**Decisions.**

- **`RoundPanicked` becomes a state.** A panic goes through the tally too: `Fault::Panicked` reaches `tally.round`, which
  posts `RoundPanicked` as a state for the lane instead of an event. The lifecycle arm gives it
  `NextSuccess(Op::Round)`, as `RoundFailed` has now.
- **`ScanFailed` of a failed walk becomes a condition.** It is keyed `(root, code)` and resolved by `Resolver::RootWalk`,
  so a complete walk of the root clears it, and a repeat replaces it. No new mechanism: the scan notes already work this
  way. The other three posts stay events.
- When the resolver changes, regenerate the golden table and keep `the_resolver_table_is_the_golden_one` passing.
- A root whose folder cannot be opened is shown by the waiting-root state in `MOUNT_POINTS_PLAN`. This plan adds no second
  state for it.

## 10. Thumbnails and pass errors

**Verified (read in the code, 2026-11-24).**

- Eviction is at store time, in the store's own transaction (`thumbcache/src/writer.rs:355-404`,
  `thumbcache/src/lib.rs:225-236`). A batch that cannot make room is not stored, and `put_batch` returns `false`.
  Eviction spares the current height (`keep_height`, `writer.rs:397`).
- `sweep_orphans` drops the cache's entries whose file id the catalog no longer has (`thumbcache/src/lib.rs:266-271`,
  `reader.rs:23-43`). It reads the catalog through `ATTACH`. It runs once when the removals thread starts, with the
  `swept` flag set before the attempt (`library/src/coordinator.rs:773-794`), after each `removals::finish`
  (`library/src/removals.rs:36`), after `discard_removal` (`app/src/library.rs:435`), and in the CLI (`cli/src/lib.rs:451`).
- A failed startup sweep posts `FolderNotRemoved` on every failure, with no repeat rule (`coordinator.rs:789-793`). That
  is a plain event (`model/src/notice.rs:1400`, `:1408`). A failed sweep in `finish` is propagated by `?`
  (`removals.rs:36`) and reported as a removal failure (`coordinator.rs:800-806`), although the folder is already removed.
- `Thumbs` catches every error of `run_while`, posts `CacheNotStored`, and returns `Done` (`library/src/passes.rs:276-291`).
  The round goes on, and no backoff applies.
- `Placeholders` does not catch: `placeholders::run_while(...)?` (`passes.rs:317-324`) returns any error to the round.
  Both passes use `thumbs::Error`, whose variants are `Catalog`, `Cache`, `Make`, `Render`, `Read`, `Pass` and `Invalid`
  (`thumbs/src/lib.rs:31-60`).
- `pipeline::batched` returns `Result<(), Failed>` (`jobs/src/pipeline.rs:441-449`). The flush's error is converted with
  `From<Error> for JobFailed` (`thumbs/src/lib.rs:65-78`), which maps `Catalog`, `Cache`, `Render` and `Invalid` to
  `Class::Other`; `ran?` (`:455`) then returns `Error::Pass`. So a catalog error from `currents` (`:334`) and a cache
  error from `put_batch` (`:386`) leave `run_while` with their kinds lost.
- Init errors (`Maker::new`, `:421`), and panics and setup failures in the pipeline, fail the whole pass through `run`
  (`pipeline.rs:215-217`, `:325-328`) and reach `Error::Pass` the same way.
- A worker's `process` error is an item's result, not a stop: it reaches the flush as `Result<R, Failed>`
  (`pipeline.rs:448`, `:510-525`). Only the flush stops a pass (`:481-495`).
- Items are looked up by index in four places: `thumbs/src/lib.rs:337` and `:423`, `placeholders.rs:110` and `:191`.
- `pending_sql` joins `cat.thumb_sources` and `cat.thumb_failures` with the cache's `entries` table in one statement
  (`thumbcache/src/reader.rs:202-215`). `pending_for` and `orphans_in` read the catalog through `ATTACH`
  (`reader.rs:35-36`, `:66-68`), so a catalog read failure surfaces as `thumbcache::Error::Sqlite`, then `Error::Cache`.
- `run_while` returns through `ran?` (`:455`) before its `stats` are returned. On an error the ledger outcome
  (`stats.ledger`), the unreadable count and the failures are dropped, and `files_read` and `ledger_written`
  (`passes.rs:297-298`) are skipped.
- A per-file failure of class `Other` is not recorded in the ledger and is tried at every pass (`thumbs/src/lib.rs:161-177`).
- `get_nearest_into` reads the JPEG in a subquery over `data` (`thumbcache/src/lib.rs:179-181`) and treats a missing JPEG
  as a miss (`:184`). A stored value a column can't hold is `thumbcache::Error::Value` (`thumbcache/src/error.rs:23-25`).
- `CacheNotStored` is a state, cleared by `StoreRecovered` (`model/src/notice.rs:1366`). Its words are the title "A
  thumbnail couldn't be kept" and the body "{error}. It was shown all the same." (`wording/locales/en/text.ftl:269-270`).
  `RoundFailed` is a state cleared by the next `Done` round (`notice.rs:1362`).
- `rebind` and `replace` delete a fresh row with `DELETE FROM files` (`catalog/src/pair.rs:476`, `:560`). No sweep
  follows them.
- Passes on `pipeline::batched` that map a non-file error to a per-file `Other` today: borders
  (`borders/src/lib.rs:53`), import (`import/src/lib.rs:58`), crop (`crop/src/lib.rs:72`, with its own pipeline pass at
  `:59-61`), and the CLI borders analyzer (`cli/src/borders.rs:88`, `:93`). Writeback does not use `pipeline::`
  (`writeback/src/error.rs:73-77` is a job's failure).

**Decisions.**

Pipeline errors:

- **D1. Typed errors through the pipeline.** `pipeline::batched` is generic over the flush's error `E` and returns
  `Halt<E>`: `Pipeline(Failed)` for a panic, a lost item or a setup failure, or `Pass(E)` for the pass's own error. The
  flush returns `Err(E)` to stop the pass. Each caller defines its own error type; `Failed` is not a default. `Error::Pass`
  and the `From<Error> for JobFailed` conversion for pass errors are removed.
- **D2. Items carried, and per-item results typed.** The pipeline passes each item with its result, so no index is looked
  up. A per-item result is `ItemError<E>`: `File(Failed)`, a file's failure with its class, or `Pass(E)`, which halts the
  pass through the flush. The `Invalid` lookups go.
- **D3. Init returns the pass's typed error.** `Maker::new` failing is `Render`. Panics and setup failures are
  `Halt::Pipeline`. Both are round errors.
- **D4. One contract for every pass on `pipeline::batched`.** No per-file error is `Other`. A file's failure is `Make`'s
  `Undecodable` or `Unreadable`, or the pipeline's `Read` or `Panic`; every other error is a pass error. Borders, import,
  crop and the CLI borders analyzer change in their own passes to meet it, and their pass errors become round errors
  (D7). Writeback is not a pipeline pass and is outside this contract.
- **D5. The maker source is an argument.** `run_while` in `thumbs` and `placeholders` takes
  `&(dyn Fn() -> Result<Maker, Error> + Sync)`. Production passes `|| Maker::new().map_err(Error::Render)`, so the `Render`
  mapping sits at the call site, and a test can pass a failing source. Callers: `library/src/passes.rs:276`, `:317`;
  `cli/src/lib.rs:541`, `:544`; `library/tests/writeback.rs:143`, `:230`; `thumbs/src/lib.rs:245`, `:613`, `:621`, `:839`.

Pass results:

- **D6. `run_while` returns one type.** It returns `Result<ThumbStats, Stopped>`, where `Stopped { error, stats }`. Before
  setup the stats are empty. `Thumbs` and `Placeholders` report the ledger outcome, the unreadable count and the failures
  in every case, then classify the error. The calls after the pipeline, `checked` and `shrink`
  (`thumbs/src/lib.rs:460-462`), return `Stopped` with the stats so far, not a bare error. Callers that use `?` convert
  `Stopped` into `Error`.
- **D7. One classification for `Thumbs` and `Placeholders`.**
  - **Cache store** (`put_batch`, `checked`, `shrink`): a round error, `CacheStore`. The round posts `CacheNotStored`, not
    `RoundFailed`.
  - **Cache read** (`pending`, `totals`, in `Thumbs`): a round error, `CacheRead`. The round posts `CacheNotRead` (D13).
  - **Placeholders' cache reads** (`cached`, `placeholders.rs:118`): a cache-level error is a `Pass` result of
    `CacheRead`, mapped explicitly at `cached` (D8). A row-level value error is a miss (D8). The read in `read`
    (`placeholders.rs:101`) keeps its fall-back: if the cache read fails there, the file is read, and the worker's read
    decides the pass.
  - **Catalog** (the catalog open, `recorded_failures`, `currents`, and the candidate reads of D10), **Render**, and the
    pipeline's own failures: round errors, with `RoundFailed`.
  - **Make** and **Read**: per-file. `Undecodable` is recorded in the ledger; `Unreadable` is retried by the next pass.
- **D8. A row's value error is a miss; a cache-level error is the pass's.** `thumbcache::Error::Value` belongs to one row:
  that file's read is a miss, its entry is forgotten (`forget_height`, `thumbcache/src/lib.rs:276`, best effort, as the
  touch is), and the next thumbs pass remakes it. Every other cache error (a missing table, a statement that won't prepare,
  I/O, a damaged file) is cache-level, as D7. The split is made at the cache's call sites, not in the pass.
- **D9. Retry, and said once.** A round error is retried with section 3's backoff, with a 60 s cap and no give-up, as goal
  3 requires. A permanent error is retried every 60 seconds and shown once, as `RoundFailed` or as its cache state (D7).

The pending query and the sweep:

- **D10. The pending query is split by owner and joined as a streamed merge, in a fixed order.**
  1. The catalog's volumes, when the set is `All` (`catalog_volumes`, `reader.rs:124`). `checkpoint_since` needs them.
  2. The cache's checkpoints for the height (`checkpoint_since`, `reader.rs:107`) and its `evictions` (`checked`), on the
     cache's own connection. They give the `since` change number.
  3. `Catalog::thumb_candidates(height, volumes, since)` in `pg_catalog` streams the candidate files in file-id order,
     with their volumes. It returns the catalog's change number and `since_checked` from the same read. It applies the
     current rules: `since` is used only when `since ≤ seq` (`reader.rs:59-60`, `:77`), and files that failed and have
     not changed are left out by `thumb_failures` (`reader.rs:210-211`).
  The cache then streams its entries at the height in file-id order. The thumbs pass merges the two streams, keeps the
  candidates that need work, and sorts those by volume and file reference, as the query's `ORDER BY` does now.
  - **Ordering.** The checkpoint written after the pass uses the change number from step 3, never a later one. No shared
    snapshot is needed for correctness: each made thumbnail is re-checked against the catalog before it is stored
    (`thumbs/src/lib.rs:334-349`), so a stale candidate costs only work.
  - **Memory (measured 2026-11-24, 100,000 files, a temporary probe since removed).** `pending` raised the working set by
    5.8 MB with no work items (every entry current), and by 10.9 MB with 100,000 work items (about 50 bytes each above the
    5.8 MB). The current query is therefore not bounded by its work alone: SQLite's page cache grows with the pages read,
    up to the 128 MiB set at `reader.rs:65`. The streamed merge drops that component, and its cost is bounded by the work.
    The saving over the current query is at most 128 MiB at the cap. The 4.6-million-row case is not measured.
- **D11. The sweep streams the same way, bounded by the id high-water.** `thumb_high_water` is read first. The catalog's
  file ids are then streamed in id order up to it, and the cache's entries are merged against them
  (`reader.rs:19-20`, `:36`). A file added during the merge is outside the bound. `orphans_in` loses its `ATTACH`.

Notices:

- **D12. A failed sweep is a keyed state at every call site.** A new `Notice::CacheNotSwept { error }` (code
  `cache-not-swept`) is a `State` with key `cache-not-swept` and resolver `NextSuccess(Op::Sweep)`, a new `Op` variant
  with key `sweep`. It is `Warning` and source `Cache`, as `CacheNotStored` (`notice.rs:1243-1260`, `:1315-1320`). A
  repeat failure replaces the state. Each sweep call site posts it on failure and resolves it on success with
  `notices.resolve(Trigger::Op(Op::Sweep))`, as `library/src/tally.rs:100` does for a round.
  - The startup sweep (`coordinator.rs:787-794`) sets `swept` only on success, and is retried at each removal recheck
    (`REMOVAL_RECHECK`, 30 s, `coordinator.rs:150`).
  - The sweep in `finish` (`removals.rs:36`) no longer fails the removal. `finish` returns the sweep's outcome, and its
    caller posts it. The folder is already hidden and deleted, and `finish`'s own doc (`removals.rs:14-17`) says a
    thumbnail failure is not a reason to fail the removal.
  - The app's `discard_removal` (`app/src/library.rs:435`) posts through the inbox. The CLI (`cli/src/lib.rs:451`) prints
    the outcome, as its report resolves nothing (`cli/src/report.rs:71`).
- **D13. A failed cache read is its own keyed state.** A new `Notice::CacheNotRead { error }` (code `cache-not-read`) is
  posted when `Thumbs` cannot read the cache (`pending` or `totals`). It is a `State` with key `cache-not-read` and
  resolver `NextSuccess(Op::CacheRead)`, a new `Op` variant with key `cache-read`, cleared by the next successful cache
  read of `Thumbs`. It is `Warning` and source `Cache`. `CacheNotStored` is not reused: its title says a thumbnail couldn't
  be kept, which is false for a read, and its resolver (`StoreRecovered`) is a store's recovery. Wording:
  `msg-notice-cache-not-read-title` ("The thumbnail cache couldn't be read") and `-body` ("{error}. Its thumbnails wait
  until it can be read.").
  - Both new kinds follow the procedure in `notice.rs:50-61`: the variant and its ids entry, the level, source and
    lifecycle arms, a sample in `Notice::samples()`, a `golden.jsonl` line, a `resolver_table.golden` row (for example
    `| \`cache-not-swept\` | — | \`next-success(sweep)\` |`), an arm in the test `ordinal`, the `ftl` entries, and the error
    arm at `crates/wording/src/notice.rs:79`. On a golden mismatch the test prints the table, and that output is the new file.

Pair writes and orphans:

- **D14. Pair writes forget their thumbnails.** `rebind` (`catalog/src/pair.rs:476`) and `replace` (`:560`) return the ids
  of the fresh rows they delete. The pass forgets those thumbnails after the commit, in the same `forget_replaced` call
  that handles replaced thumbnails (`passes.rs:88`, over `pairing.replaced_ids`). No sweep is needed for them.
- **D15. The sweep is the orphan prune; nothing more is built.** Orphans from a removal are swept (`removals.rs:36`, and
  the startup sweep). Orphans from the pair writes are forgotten by D14. A rebuild leaves none: it carries every row or
  none (a refused row is an error, not a drop), keeps the catalog id and the file ids when it carries rows, and gets a
  new catalog id, so the cache starts over, when it carries none (`rebuild.rs:199-253`, `:284-301`). The removal and
  rebuild cases are checked below. Eviction is not the reason: it is least-recently-used and spares the current height.

Unchanged:

- **D16. No change** to eviction, `cache_full`, or the wording of `CacheNotStored`.

**Dependencies.** D7's round errors need section 3's isolation: without it, a round error stops Placeholders, Hashes,
Pairing and Borders for the round. Build after item 3 of the order of work. D10 adds a query to `pg_catalog`. D12 and D13
need the golden tables.

**Checks (each names the fault that makes it happen, so no test needs an injected error).**

- Catalog error: a fixture drops the view the candidate query reads (`thumb_sources` today) before a pass. The round
  returns an error, `RoundFailed` is posted, no cache state is posted, the later passes run, and the round is retried
  after the backoff.
- Cache store: `pg_thumbcache::fault::refuse_stores` (as `library/src/runner.rs:357` does). `CacheNotStored` is posted with
  the cache's text, `RoundFailed` is not, and the lane retries after the backoff, with no wake.
- Cache read: the `checked` table is dropped in the cache file before a pass, so `pending` fails. `CacheNotRead` is
  posted, and the lane retries after the backoff. The fixture recreates the table with the exact definition at
  `thumbcache/src/schema.rs:52-54`; a reopen is not assumed to restore it (not checked). The next successful read
  resolves the state.
- Locks: no check uses a write lock, since WAL readers are not blocked by writers.
- Placeholders: a dropped `placeholders` table (`work.rs:150-159`) is a round error. Dropping `data` fails the statement
  in `get_nearest_into` (`thumbcache/src/lib.rs:179-181`), so the pass posts `CacheNotRead`. A row whose `jpeg` is set
  to a non-blob value is a miss: that entry is forgotten, the pass completes, and the next thumbs pass remakes it. A
  refused cache confirms only that the pass posts nothing.
- Per-file: a failure is `Make` or `Read`, recorded or retried by its class, and never `Other`. The Undecodable and
  Unreadable cases in `thumbs/src/lib.rs:720-736` cover this.
- Row value in the entries stream: an `entries` row whose `mtime` is text is a miss once D10 exists. The pass completes,
  and the entry is forgotten.
- Ledger: a trigger in the fixture aborts inserts into `file_failures` for a file with no existing row. The ledger
  upserts (`writers.rs:892-896`), and an upsert onto an existing row may not fire an insert trigger. `thumb_failures` is
  a view over `file_failures` (`baseline.sql:281`), so `pending` still succeeds. With `refuse_stores` also set, the flush
  and the final ledger write both fail; both are reported and `LedgerNotWritten` is posted. The trigger's upsert
  behaviour is confirmed when the test is written.
- Maker startup: a source that returns `Error::Render` (D5) makes the pass a round error: `RoundFailed` is posted, no cache
  state is posted, and the lane retries after the backoff. `a_failed_init_fails_the_pass`
  (`crates/jobs/src/pipeline.rs:1231`) asserts the error's text ("worker setup", "no state"); it is extended to assert the
  typed error (D1).
- Startup sweep: the view `thumb_file_ids` is dropped before the startup sweep. The sweep fails, and `CacheNotSwept` is
  posted, not `FolderNotRemoved`. Restoring the view and the next recheck sweeps the orphans and resolves the state.
- Removal finish: the same drop during a removal's `finish`. The folder is hidden and deleted, `finish` returns the
  sweep's outcome, no `FolderNotRemoved` is posted, and `CacheNotSwept` is posted.
- Pair writes: a thumbnail made for a fresh row is gone after `rebind` and after `replace` commit.
- Orphans, removal (measured 2026-11-24 with a temporary probe in `crates/library/tests/writeback.rs`, since removed): an
  imported folder of six files had six thumbnails. `begin_removal`, `hide_pages` and `drive_removals` deleted the six
  files without a forget, leaving six orphaned entries. `sweep_orphans` dropped six, and a second sweep dropped none. The
  real path (`removals::finish`) forgets each batch first and then sweeps, so it leaves none either way.
- Orphans, rebuild (checked by reading and by existing tests, 2026-11-24): `a_queued_write_the_layout_refuses_is_an_error_not_a_dropped_row`
  (`rebuild.rs`) and `a_cache_of_another_catalog_starts_over` (`thumbcache/src/lib.rs:1400`) pass.
- Callers of `pending` and `orphans_in` are rewritten against the new API: `thumbcache/src/lib.rs:1208-1384` and `:1470`,
  `reader.rs:147-195` and `:339-344`, `thumbs/src/lib.rs:617` and `:624`, and the bench at
  `bench/benches/ir/catalog.rs:165`. The checkpoint and `since` tests (`lib.rs:1239-1384`) keep their expectations. The
  streamed merge returns the same work list, in the same order, on each fixture.
- `the_resolver_table_is_the_golden_one` passes after the golden table is regenerated for both new kinds.

## 11. Batch sizes and checkpoints

**Status (2026-11-24).** Implemented, and every check below is run. The gap, the close behaviour and the long round are
measured (see Cost and Gap in D8). Not measured: how many writes outside the three threads happen between rounds, which
depends on use, and read latency while a large WAL is held. D8 is in place: `wal_autocheckpoint = 0` in `SETTINGS`
(`catalog/src/schema.rs`); `Catalog::checkpoint_passive` (`catalog/src/core.rs`), which counts its calls
(`passive_checkpoints`), run at the end of each lane's round (`library/src/lane.rs`), after the removal's deletes
(`library/src/removals.rs`), and when the writeback drain ends (`writeback/src/lib.rs`). D7 is unchanged. Tests that pass:
the catalog's `a_catalog_connection_does_not_checkpoint_on_a_commit`, `a_passive_checkpoint_moves_the_wal_and_a_reader_in_the_way_is_no_error`
and `a_passive_checkpoint_leaves_the_frames_a_reader_holds`; the library's `a_lane_round_checkpoints_the_wal_at_its_end` and
`a_removal_checkpoints_the_wal_after_its_deletes`; and the writeback's `a_drain_checkpoints_the_wal_at_its_end`. The catalog,
library and writeback suites pass in full. The delete-commit latency was re-measured on the built code (below), on E:.

**Verified.**

- Two constants are named `WRITE_BATCH`, with different purposes.
  - 500, public (`catalog/src/lib.rs:23`): the rows a batch holds in the import, hashes and borders passes
    (`import/src/lib.rs:26`, `import/src/hashes.rs:15`, `borders/src/lib.rs:22`).
  - 1000, `pub(crate)` (`catalog/src/types.rs:122`): the most rows a pass over a walk's results writes in one
    transaction, so another writer waits for one batch at most. Its users are the walk's mark-missing (`roots.rs:201`),
    the walk's pairing (`pair.rs:39,403`) and the removal's hide and delete (`removal.rs:112,288`).
- The import's own pairing uses `WRITE` = 4000 (`import/src/pairs.rs:39`).
- Every catalog connection runs `SETTINGS` (`schema.rs:122`): WAL, `synchronous = NORMAL`, and no
  `wal_autocheckpoint`. So SQLite's automatic checkpoint runs in the commit that crosses 1000 WAL pages.
- Explicit checkpoints are at `removal.rs:269-271`, `work.rs:563`, `core.rs:60` and `rebuild.rs`. None is on a writer's
  path during a batch.
- The thumbnail cache has its own WAL and checkpoints (`thumbcache/src/writer.rs`). This section does not cover it.

**Measured (2026-11-23, the real catalog code; temporary probes, since removed).** E: is a hard disk; D: is NVMe.

- Batch work is small. The delete's batches of 1000 files commit in about 0.5 ms each, with 10–20 ms of statements per
  batch (median 15 ms over 3 runs). The whole delete of 10,000 files has a median of 324 ms.
- Removal bench, in memory (callgrind, `ir::removal`): the hide is 796,777,538 instructions and the delete 895,915,093.
  The hide is about 47% of the removal's 1,696,398,183.
- The stall is a checkpoint, not a batch. In each run one commit of 182–190 ms was the tenth batch's; the other commits
  took about 0.5 ms.
- Delete of 10,000 files on E:, alternating runs, 3 each:

  | Mode | Drive, median | Slowest commit, median |
  |---|---|---|
  | Automatic checkpoint (today) | 435 ms | 219 ms |
  | `wal_autocheckpoint = 0`, PASSIVE checkpoint after the loop | 217 ms | 25 ms |

  The PASSIVE checkpoint took 207 ms (1,200 pages, busy 0, 4.8 MB of WAL before it). The total is the same (217 + 207 =
  424 ms, against 435 ms). The work moves from the writer's commit to the thread that wrote the WAL.
- Writer latency during checkpoints (E:, 10 runs, 278,478 writes from a second connection): median 22 µs, p99 85 µs,
  p99.9 175 µs. The slowest write was 1–35 ms in most runs. One run had 330 ms, while its checkpoint took 3.75 s under
  sustained writing. Checkpoints took 151 ms to 3.75 s, longer as the writer added WAL during them.
- On D:, checkpoints took 6–9 ms and no write was slow. On E:, checkpoints took 234 ms to 1.2 s, and each run had one
  write of 2.7–49 ms that overlapped one.
- A round of realistic size does not reach the threshold. An import of 300 JPEGs (640 × 480) wrote 527 KiB of WAL, far
  under the automatic threshold, so no automatic checkpoint ran in either mode. Medians were 55 ms each way (3 runs,
  alternating).
- Where the wait is (E:, a delegating VFS timing every file call, 3 runs): the checkpointer's time is one `xSync` of the
  database file, 276 ms to 1.01 s. The writer's slow calls are its `xWrite`s to the WAL, 33–196 ms, and they fall inside
  that sync. Its other calls take microseconds, and it had no gap of 2 ms or more between calls. What the kernel does
  inside the wait is not shown.
- Not established: a flush of another file on the same volume. In the two tests that ran one, the writer saw at most two
  commits of 2 ms or more (worst 2.9 ms). The VFS trace is the strongest evidence, and it points at the database flush.

**Rejected.**

- Small checkpoints (one every 200 commits, 21 in all) against one checkpoint of the whole WAL: 13 slow commits against
  2, and 1,044 ms of checkpoint time against 313 ms.
- `synchronous = OFF` for the checkpoint. It removes the flush, but a power loss could then leave the database file
  behind the WAL reset. That is a durability decision, not a tuning one. A control run gave a 46 µs checkpoint, which
  cannot be a copy of 15 MB of pages; it is unexplained, so it is not used as evidence.
- Moving the catalog to an SSD. The NVMe drive showed no stall. That is a hardware choice, outside this plan.

**Decisions.**

- **D7. Batch sizes: no change.** Keep both constants, since they bound different waits. Batches of 1000 cost 10–20 ms
  of statements each (measured), so the removal keeps 1000. Unifying on 500 would double the removal's transactions
  with no measured gain, so it is dropped.
- **D8. Checkpoint policy.**
  - `wal_autocheckpoint = 0` in `SETTINGS` (`schema.rs:122`), for every catalog connection.
  - A PASSIVE checkpoint, run by the thread that wrote the WAL, at the end of its work: each lane at the end of its
    round, the removal job after its deletes, and the writeback when its queue drains.
  - Effect (measured): the writer's commit no longer pays for the checkpoint. The slowest commit of the delete falls
    from 236–269 ms to 53–55 ms (re-measured 2026-11-24 on the built code, E:, 30,000 files; see the checks). The total work
    is not higher: 620–650 ms against 667–996 ms.
  - Not a bound. A write can still wait for a checkpoint: typically microseconds, rarely tens of milliseconds, and under
    heavy write load occasionally a few hundred. "Never blocks a writer" is not the claim.
  - Cost (measured 2026-11-24): a round's WAL is held until the round ends. A scan and import of 20,000 small JPEGs wrote
    12.8 MB and 14.8 MB of WAL, about 1.4 KB a file, and the PASSIVE checkpoint at the round's end moved all of it in
    0.125 s (on D:). Under the automatic setting that round would have crossed the 4 MB threshold about seven times, each
    crossing stalling a commit. Reads of a large WAL are slower until it is checkpointed; that is not measured.
  - Gap (measured 2026-11-24): a writer outside the three threads (for example the window's session saves, or any catalog
    write that is not a round or a drain) adds one page per small commit: 4,120 bytes each, over 2,000 commits. The WAL
    grows by that much per write until the next round or drain checkpoints it, and the automatic threshold of 4 MB is about
    1,000 such writes. Closing the last connection removes the WAL with no checkpoint point reached (test
    `the_last_connection_to_close_leaves_no_wal`). How many such
    writes happen between rounds is not measured; it depends on the user.

**Checks.**

- Measured 2026-11-24 on the built code, on E:, with a file-backed catalog and a temporary probe (since removed). Each
  run clears the WAL, then removes the folder; the batch intervals are read from the removal's callback. At 30,000 files,
  where the automatic checkpoint fires during the delete: the automatic setting's slowest batch was 236 and 269 ms, and
  the delete 667–996 ms in all. With PASSIVE, the slowest batch was 53 and 55 ms, the delete 355–384 ms, and the checkpoint
  after it 267–269 ms (620–650 ms in all). So the stall leaves the writer's commit, and the total is not higher. The
  slowest batch is about 54 ms, not the 25 ms predicted. At 10,000 files the WAL does not reach the threshold, and neither
  mode stalls (slowest batch 36–67 ms in both). The stall is therefore specific to a delete that crosses the threshold.
- A PASSIVE checkpoint while a reader holds a snapshot is not an error, and it leaves the frames after that snapshot in
  the WAL: `checkpointed` is below `log`, and every frame moves once the reader ends. (The first form of this check said
  `busy` would be above zero; a PASSIVE checkpoint reports `busy` as 0, so the test asserts the frames instead.) Test:
  `a_passive_checkpoint_leaves_the_frames_a_reader_holds`.
- After writes that no round or drain follows, the next PASSIVE checkpoint moves them: `checkpointed` equals `log` once the
  reader is gone. This is the gap above. Test: `a_passive_checkpoint_moves_the_wal_and_a_reader_in_the_way_is_no_error`.
- Each call site runs its checkpoint: the lane round, the removal's deletes and the writeback drain each leave the catalog's
  passive count raised. Tests: `a_lane_round_checkpoints_the_wal_at_its_end`, `a_removal_checkpoints_the_wal_after_its_deletes`
  and `a_drain_checkpoints_the_wal_at_its_end`.

## Not built, and why

- **Flusher** (bisecting a failed write batch to find the bad item): no per-item write failure has been reproduced. Each
  item is checked against the catalog inside the batch's transaction, so a stale item is skipped rather than failing the
  batch. A batch failure is a round error.
- **Work-lease table, fence token, heartbeat thread**: section 5.
- **Idle-work probe**: section 4.
- **Pending-query exclusion for writes in progress**: section 8.
- **Orphan prune in the thumbnail cache**: section 10.
- **A bulk hide that skips the trigger's per-row body**: measured on the real code in section 1, and not faster.

## Schema

No change. The leases, roots, scans and failure tables stay as they are.
The migration question in the earlier version does not arise.

## Order of work (one build at the end)

1. Removal: the removals job parks the folder's lanes around the batched hide (D1, D2); the startup barrier (D5); the grid
   publish points (D4); `Begun::Started` carries the volumes (`removal.rs`, `coordinator.rs`, `app/library.rs`,
   `desktop/library.rs`, `roots.rs`, `cli/src/lib.rs`). Test (section 1, Checks).
2. Scans: roots per popped path (`passes.rs`); `route_scans` takes the requests before it reads the roots
   (`coordinator.rs`). Test (section 2, Checks).
3. Rounds: isolation (`jobs/schedule.rs`); the lane deadline and backoff (`lane.rs`); the clear rule (`lane.rs`). Change
   the tests named in section 3. No change to `wake()`.
4. Leases: `LeaseLost` requeue; the partial pairing carried in the error (`scan/lib.rs`, `passes.rs`).
5. Pairing load: skip and count a malformed hash, re-analyse its picture, and check the version before reading rows
   (`catalog/src/work.rs`, `catalog/src/pair.rs`, `import/src/pairs.rs`).
6. Whole-file length check (`jobs/pipeline.rs`, `import/lib.rs`); video hash in chunks that return "not hashed yet" on a
   stop (`analysis/analyze.rs`, `import/hashes.rs`).
7. `WRITE_BATCH`: no change. Checkpoint policy: `wal_autocheckpoint = 0` in `catalog/src/schema.rs`, and a PASSIVE
   checkpoint at the end of each lane's round, the removal job's deletes and the writeback's drain (section 11).
8. Notices: `RoundPanicked` as a state through the tally; a failed walk's `ScanFailed` as a `RootWalk` condition; the
   golden table (`model/src/notice.rs`, `library/src/tally.rs`, `library/src/lane.rs`, `library/src/passes.rs`).
9. Thumbnails and pass errors (section 10): typed errors through `pipeline::batched`, `init` and per-item results (D1–D3);
   items carried, not indexed (D2, `thumbs/src/lib.rs`, `thumbs/src/placeholders.rs`); `run_while` returns `Stopped` (D6);
   the contract of D4 in borders, import, crop and the CLI borders analyzer, each checked when its pass changes
   (`borders/src/lib.rs:53`, `import/src/lib.rs:58`, `crop/src/lib.rs:72`, `cli/src/borders.rs:88`); the maker source (D5);
   the candidate query in `pg_catalog` and the entries read in `thumbcache`, joined as a streamed merge with no `ATTACH`
   (D10, D11); the keyed sweep and cache-read states and their golden tables (D12, D13); the pair writes forget their
   thumbnails (D14). Needs item 3. Test (section 10, Checks).
10. Tests (below), one `cargo build`, `cargo test` for the crates touched, then `just lint`.

## Tests

**Changed.** `an_error_ends_the_round_with_it` (new isolation contract). `a_failed_round_is_said_and_the_next_round_clears_it`
(retries without a wake; cleared by a `Done` round, not a preempted one).

**New.**

- A root added while a `Scans` pass walks another folder is walked.
- Section 1's checks: the park test (D1), the park's wait (D1), the startup barrier (D5), the P3 regression test, and the
  lanes on the other volumes (D2).
- A `LeaseLost` requeues the root, and the replaced thumbnails of the partial pairing are still forgotten.
- A malformed stored hash is skipped and counted, and the pairing pass still completes.
- A whole-file read whose length changes during the read stores nothing, and a gathered JPEG still stores.
- A video hash stops within one chunk.
- Section 10's checks: the catalog, cache-store, cache-read, placeholder, ledger, maker-startup, sweep and removal cases,
  and the pair-write forget (D1–D14).

## Verification

- Native PowerShell build with `PG_QT_DIR` on `PATH`; `cargo test` for `pg_jobs`, `pg_library`, `pg_catalog`, `pg_scan`,
  `pg_import`, `pg_analysis`, `pg_thumbs`, `pg_writeback`; `just lint`. Linux CI runs in the pg-dev container only.
- Benches of the removal (`ir::removal::folder_removal`, `ir::removal::folder_hide`) run in the Linux container with
  `just linux-bench`, against the committed HEAD. A bench run of uncommitted code needs the working tree mounted, as the
  2026-11-23 runs did.
- Manual, in the app: remove a folder during a large scan (the job stops, nothing reappears, the grid empties); remove,
  then re-add (analyses and ratings return); add a folder during a round (it is scanned); quit during a long video hash
  (the quit is prompt); rotate photos during a manual rescan (the walk completes); quit during a removal and relaunch (the
  removal finishes before any lane works on the folder).

## Standing rules

One build at the end, not between steps. No new branch. No commit unless asked. The workspace lints apply: no indexing,
`unwrap`, `expect`, `panic` or bare casts; `pg_model::num` for conversions; `pub(crate)` for internal items;
`checked_mul`/`saturating_mul` for backoff arithmetic.
