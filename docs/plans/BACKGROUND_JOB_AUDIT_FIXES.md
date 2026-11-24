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

**Verified.**

- The thumbnail cache evicts at store time, in the store's own transaction (`thumbcache/lib.rs:225-228`, and the writer's
  store). `cache_full` means eviction could not make room while sparing the current height. So an orphan prune has nothing
  to do.
- `sweep_orphans` lists the cache's entries the catalog no longer has, and forgets them (`thumbcache/lib.rs:266-271`). It
  runs once, when the removals thread starts (`coordinator.rs`, the `swept` flag), and after each removal
  (`removals.rs:36`).
- A thumbnail error does not reach the round. `Thumbs` catches the error of `run_while`, posts `CacheNotStored`, and
  returns `Done` (`passes.rs:280-292`). The round goes on, and the backoff of section 3 does not apply.
- `CacheNotStored` is a state, cleared by `StoreRecovered` (`notice.rs:1366`), not an event. But the notice's text is
  the whole error chain under the words "cache not stored", so a catalog error is shown as a cache failure.
- The error type of `run_while` is typed: `Catalog`, `Cache`, `Make` and `Render` (`thumbs/lib.rs:32-44`). So the pass can
  tell a cache write from a catalog failure.

**Decisions.**

- **Catalog errors become round errors.** `Thumbs` returns them, so the round records them, the backoff of section 3
  retries them, and the other passes still run. The `currents` check's error (`thumbs/lib.rs:334`) takes the same path.
- **Cache errors stay `CacheNotStored`**, with the cache's own text, not the catalog's.
- **No change to eviction, `cache_full`, or the sweep.**

## 11. Batch sizes

**Verified.** Two constants share the name `WRITE_BATCH`, with different documented purposes.

- 500, public, `catalog/src/lib.rs:21-23`: the rows a write batch holds in the import and borders passes. Its users are
  the import (`import/src/lib.rs:26`, `import/src/hashes.rs:15`), the borders (`borders/src/lib.rs:22`) and the import
  bench.
- 1000, `pub(crate)`, `catalog/src/types.rs:120-122`: the most rows a pass over a walk's results writes in one
  transaction, so another writer waits for one batch at most, never for the pass. Its users are the walk's mark-missing
  (`roots.rs:201`), the walk's pairing (`pair.rs:39,403`), and the removal's hide and delete (`removal.rs:112,288`).
- The import's own pairing uses a third value, `WRITE` = 4000 (`import/src/pairs.rs`). The earlier text of this section
  said pairing used the 500 constant; it does not.

**Decision.** No change: keep both constants, since they bound different waits. The removal's batches are of the
walk-results kind, because another writer waits on them, so they keep 1000. The earlier decision to unify on 500 is
dropped: it would double the removal's transactions, with no measured gain.

**Measured (2026-11-23).** The delete alone: `ir::removal::folder_delete::ten_thousand`, the new bench (callgrind, in
memory): 895,915,093 instructions; with the hide (796,777,538) that is the full removal's 1,696,398,183. On the real
catalog on E:, the delete per batch (a temporary timing example, since removed; 10,000 files, 3 runs): median 15 ms,
and the whole delete a median 324 ms.

**The slow window, traced (2026-11-23, temporary per-statement timers, since removed).** In each run one window of about
200 ms is the tenth batch's commit: its commit takes 182–190 ms, where the other nine commits take about 0.5 ms each.
The final step's commit takes about 30 ms, and its statements under 1 ms. The batch's own delete is 10–20 ms, like the
others. So the stall is one commit. In WAL mode with `synchronous = NORMAL`, a commit waits for nothing, and a checkpoint
fsyncs the database, so this is consistent with the automatic checkpoint that runs once the WAL reaches its threshold
(the checkpoint itself is not observed; the stall in the commit is).

The consequence for section 11: the stall belongs to the checkpoint, not to the batch. Its length does not depend on the
batch size, and any writer's commit can absorb it, not only the removal's. So "keep 1000" is a decision about typical
batch work (10–20 ms, measured), and the checkpoint stall (about 0.2 s on E: in this run) is a separate bound that applies
to every writer whatever the batch size.

**Decision: the checkpoint bound.** A writer's commit should never run a checkpoint, and the job's own threads should
pay for checkpoints when they have nothing to write.

- **Today (verified).** The catalog's connections use SQLite's automatic checkpoint, every 1000 WAL pages, and the
  commit that crosses the threshold runs it (`schema.rs:122`, `SETTINGS`, no `wal_autocheckpoint` set). Explicit
  checkpoints exist in a few places (`removal.rs:269-271`, `work.rs:563`, `core.rs:60`, `rebuild.rs`), none on a writer's
  path during a batch.
- **Prior measurement (`docs/reference/PROFILING.md`, the pairing write).** One automatic checkpoint over 4.9 MB of WAL
  cost about 22 ms, with `wal_autocheckpoint = 0` giving 23 commits against 45. That was a different disk; the stall here
  is about 185 ms, so the cost depends on the disk and the WAL's size.
- **Decision.** Set `wal_autocheckpoint = 0` in `SETTINGS` (every catalog connection). A PASSIVE checkpoint is run by the
  thread that wrote the WAL, at the end of its work: each lane at the end of its round, the removal job after its deletes,
  and the writeback when its queue drains. A PASSIVE checkpoint does not wait for readers, and it does not hold up writers
  in general (measured below), so a UI write's commit no longer pays for the checkpoint's work. A writer can still wait
  for one occasionally (measured below).
- **Cost.** The WAL grows while a round writes, up to that round's work. That is bounded by the round, not the library,
  and a long import round is the case to watch. Reads of a large WAL are slower until it is checkpointed.
- **Not covered.** The thumbnail cache's own WAL (`thumbcache/src/writer.rs`, a separate file with its own checkpoints).
- **Validated (2026-11-23, the delete, on E:, the real catalog code, temporary probe since removed; alternating runs,
  3 each).**
  - Automatic checkpoint (today): drive median 435 ms; the slowest commit (window) median 219 ms, which is the checkpoint
    stall. WAL 4.6 MB at the end.
  - `wal_autocheckpoint = 0`: drive median 217 ms; the slowest window median 25 ms. The PASSIVE checkpoint after the
    loop: median 207 ms, `done` 1200 pages, busy 0. WAL 4.8 MB before it.
  - The stall is removed from the writer's commit (219 to 25 ms). The checkpoint's work is not removed: it moves to the
    thread that runs it, and the total is the same (217 + 207 = 424 ms against 435 ms).
  - **A second writer during the checkpoint (same day, E:, temporary probe since removed; 3 runs, a writer connection
    making small write transactions while the checkpoint ran).** The checkpoint took 229 to 1,215 ms (longer than alone,
    207 ms, because the writer kept adding WAL). The writer committed throughout: 8,000 to 41,000 writes during the
    checkpoint, with a median of 21 µs, the same as outside it (21 µs). But the tail was not clean: the slowest write
    during a checkpoint was 1.3 ms, 30 ms and 134 ms in the three runs. The 134 ms is not explained.
  - **An import round, over real JPEGs (same day, E:, the real `pg_import::import`, 300 photos of 640 x 480, 3 runs
    each way, alternating).** The round wrote 527 KiB of WAL, under the 1000-page automatic threshold (about 4 MB), so
    no automatic checkpoint ran in either mode. Medians: automatic 55 ms; off 55 ms, and the PASSIVE checkpoint after
    it 71 ms (27 pages). So for a round of this size the policy changes nothing. The WAL of a larger round is not
    measured; an import's rows are small, so the delete (about 4.8 MB of WAL for 10,000 files) is the case that
    crosses the threshold.
  - **The writer tail over ten runs (same day, E:, 278,478 writes during the checkpoints).** Median 22 µs, p99 85 µs,
    p99.9 175 µs. The maximum was 330 ms, in a run whose checkpoint took 3.75 s under sustained writing. Other runs'
    slowest writes were 1 to 35 ms, and one run had 0.9 ms. The checkpoint itself took 151 ms to 3.75 s, longer as the
    writer added WAL during it.
  - **Revised conclusion.** A PASSIVE checkpoint leaves the writer's typical latency unchanged (p99.9 under 0.2 ms), so a
    UI write does not pay for the checkpoint's work in the typical case. It is not a hard bound: a write can wait
    tens of milliseconds, and in one run out of ten about 0.3 s, while a checkpoint runs under sustained writing. The
    stated rule "never blocks a writer" is therefore too strong. The bound for a UI write is "typically microseconds,
    rarely tens of milliseconds, and in heavy write load occasionally a few hundred milliseconds".
  - **Trace of the slow writes (same day, temporary probe since removed; 4 runs per drive, the real catalog code).** On
    E: (hard disk) the checkpoints took 234 ms to 1.2 s, and each run had one write of 2.7 to 49 ms that overlapped the
    checkpoint. Those writes fell 28 to 89% of the way through it, not at its end. On D: (NVMe) the checkpoints took 6 to
    9 ms, and no write was slow in any run. The slow writes fell in the middle of the checkpoint, not at its end, where a
    WAL reset would be. The samples are small, and the two drives' checkpoints differ in length.
  - **Flush contention, tested and refuted (same day, E:, a temporary test with no SQLite; 4 runs).** A thread appended
    4 KiB every millisecond to one file while another wrote 16 MiB to a second file and called `sync_all` on it. Each flush
    took 148 to 256 ms, and roughly 500 appends during the flushes took 2 ms or more in none of the runs. So a flush on the
    same disk does not stall small writes to other files, and the flush is not the cause of the writer's stalls.
  - **The checkpoint's flush, and a control that did not work.** A checkpoint with `synchronous = OFF` (no flush) reported
    the same pages as one with `NORMAL` (about 3,900 each), but took about 46 µs against 0.3 to 3.7 s. Copying 15 MB of
    pages cannot take 46 µs, so this control is unexplained, and it cannot be used to separate the flush from the page
    writes.
  - **Below SQLite, the file operations (same day, E:, a temporary delegating VFS registered as SQLite's default file layer,
    timing every file call of the writer and the checkpointer; 3 runs; the real SQLite, the catalog's WAL setting).** The
    checkpointer's time is one `xSync` of the database file: 276 ms, 512 ms, 755 ms, 944 ms, 1.01 s across the runs, and
    its other calls take microseconds. The writer's only slow file operations are `xWrite` calls to the WAL: 33 to
    196 ms, at 66 to 86% into the checkpoint, inside that `xSync`. The writer had no gaps of 2 ms or more between its
    file calls, so the time is in the writes, not in SQLite's code between them. The writer's other calls (`xShmLock`,
    reads) take microseconds.
  - **Conclusion.** The stall is the writer's WAL write waiting while the database file's flush runs on the same
    volume. That is the flush hypothesis again, now at the file-call level. The earlier plain-file test did not stall
    small appends during a 16 MiB flush, so it does not refute this: its pattern differs (a sequential stream of one
    appender, not SQLite's WAL, which must extend a file that another flush is holding). What the kernel does during the
    wait (the NTFS log, or the volume's cache flush) is not shown.
  - **Elevated trace (same day; `wpr` with the general, disk and file profiles, in an administrator shell; a standalone
    Python reproduction on E:, `scratchpad/etw_repro.py`, not the catalog).** The reproduction shows the stall without the
    catalog: writes of 6 to 72 ms overlap a checkpoint of about one second, and a 10 s checkpoint has writes of 400 to
    550 ms. The trace (2 GB `.etl`, in the scratchpad) shows the E: disk busy 70 to 93% of each second for about ten
    seconds around those checkpoints, with few requests per second: a queue of large transfers, not many small ones.
    That fits the writer's small WAL writes waiting behind the checkpoint's writes on the same disk.
  - **Queue or flush, tested (same day, E:, a standalone script in the scratchpad, 3 runs each).** A writer appended
    small commits while (a) nothing else ran, (b) a 16 MiB write went to another file with no flush, (c) the same write
    was flushed. Alone and unflushed: no commit of 2 ms or more, worst 0.2 to 0.3 ms. Flushed: 0 to 2 commits of 2 ms or
    more, worst 2.9 ms in the median run. The unflushed write lands in the OS cache and does not stall the writer, so the
    queue of cached writes is not the cause; the flush is. (The stalls of the checkpoint runs are larger, because the
    checkpoint's flush covers more dirty data.)
  - **Small checkpoints, tested (same day, E:, paced writer: 4,000 commits in both modes).** One checkpoint of all the
    WAL, started while the writer runs, against a checkpoint every 200 commits (21 in all). The single checkpoint: 2 slow
    commits, worst 39 ms, p99 108 µs, checkpoints 313 ms in total. The 21 small ones: 13 slow commits, worst 34 ms, p99
    146 µs, checkpoints 1,044 ms in total. So small checkpoints do not reduce the stall. Each checkpoint carries a fixed
    flush cost, so more checkpoints mean more stalls and more total time. Rejected as a fix, though a small checkpoint
    stalls for less time each.
  - **Not validated:** the individual request latencies (which disk request the writer's WAL write waited on, and for how
    long): the per-request dump is too large to read in this session, and the analysis needs the trace viewer; the 330 ms
    write (not reproduced at the file level); the thumbnail cache's WAL (out of scope). Two scratch folders on E:
    (`E:\etw-trace-scratch`, `E:\wal-queue-tests`) are left for the user to delete: the tool refused to remove them.
  - **Options left:** a writer-side bound is not available from the catalog, because the stall is a flush on the same
    disk. Lowering the flush cost (`synchronous = OFF` for the checkpoint) would remove the flush, but would let a
    power loss leave the database file behind the WAL reset. That is a durability decision, not a tuning one, and is
    not recommended. Moving the catalog to an SSD removes the stall on that drive (measured: the NVMe drive had none).

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
9. Tests (below), one `cargo build`, `cargo test` for the crates touched, then `just lint`.

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
