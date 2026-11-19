# Process library folders on different physical drives in parallel (lane per drive)

> **Status (2026-11-18).** Phases 0–6 are built in the tree, with the gaps listed below; nothing is committed.
> The measurements the plan asks for (Phase 0 items 1–3 and 5, the keep-rule A/B, the two-drive run on E: and D:) are not
> done, so the tuning constants are the ones the repo already had: 8 readers, 4 listing workers, a 240 MB buffer knee per
> lane, and a quarter of physical memory as the cap.
>
> | Phase | Built | Not built (and why) |
> |---|---|---|
> | 0 Calibration | The drive spike (volume extents, bus type, seek penalty, zero-access handles) and the work-list plans (`EXPLAIN QUERY PLAN`), in `docs/reference/IO.md`. | The timed runs on the real drives: they tune the constants, and need the machine. |
> | 1 Drives | `pg_platform::drive` (Windows extents, bus, seek penalty; Linux sysfs; union-find; `Opaque` for disks whose backing can't be seen; `Unknown` for buses that lie about seek penalty), and `pg_platform::memory` (physical RAM). | Linux nested mounts (a root holding another device). `Options::for_drive` and listing workers by `Kind`: no measured values yet, so no table. |
> | 2 Catalog | `VolumeSet`, volume-filtered work lists (`CROSS JOIN` join order; `+f.volume` for the checkpoint), the thumbs checkpoint per `(height, volume)`, busy retry at `BEGIN IMMEDIATE`, `remove_root` in batches, `Catalog::volumes()` (a loose index scan, not a range scan). | None. |
> | 3 Jobs | Lane child controls (own stop, pause and progress; shared global pause; tagged events; a per-lane pause that `Runner::set_lane_paused` sets and reads), `CpuGate` (wired by `Lanes::start`), `Budget` and `Governor` (each lane's budget split under the cap, rebalanced when a lane retires), and the pool reads its budget live. | Per-lane page-cache sizes: the connections keep their sizes. `Options::for_drive` and listing workers by kind: no measured values yet (R1). |
> | 4 Library | `lanes::plan` (fail closed, unplugged roots get no lane, opaque drives share one), the coordinator (deadline probes, routing, retirement, crop parking, turned files routed to the lanes of their volumes through `Writer::take_turned`), per-lane stops, `LaneProgress` with drive labels, the serial escape hatch (`Features::serial_drives`, a pref), notices that name the drive (`UnreadableFiles.drives`), `tests/lanes.rs` (8 tests). | The pairing cooldown (its length is a measurement, R1). Linux nested mounts (Phase 1). The lane tests the tree lacks (L1, L2, L4, L6): see the ledger. |
> | 5 CLI | `gallery` scan, import and thumbs run one lane per drive through `pg_library::lanes` (`command_groups`, `run_each`), each on its own connection and child control; `--serial` for one lane; C1 compares `--serial` with the lanes over two roots. | `report.rs` per-lane tagged progress: a lane's lines follow a heading with its drive's label. |
> | 6 Front ends | Per-lane rows with their own bar and pause in `StatusBar.qml`, over a `LaneModel`; every working or paused lane has a row, and the shared bar is gone. `LaneProgress` serves; there is no separate `LaneInfo`. | Screenshots of the stacked rows (`just shots`, the `lanes` scenario). |
> | 7 Docs | This file, `docs/reference/IO.md` (spike results), `docs/reference/STORAGE.md` (the drive kinds as the job detects them), `crates/README.md` (the lanes), and the scan plan's Phase 4 note on the device query. | `docs/plans/PLAN.md` "1 reader per HDD" still holds (one lane per spindle), so it is unchanged. |
>
> Verified: `cargo check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and the tests of
> pg-model, pg-client, pg-jobs, pg-platform, pg-library (incl. `tests/lanes.rs`), pg-catalog, pg-thumbcache, pg-app and
> pg-cli. Not run: `just check-linux` (the sysfs path), `just perf` (the callgrind gate), `just qmltest`, and a real two-drive run.

## Context

Today the library's background job is one thread (`crates/library/src/runner.rs`, `Runner` on one `pg_jobs::Service`).
Its round runs the passes in order on one `Catalog` connection and one `Control`: Crops, Cuts, Scans, Import, Thumbs,
Placeholders, Hashes, Pairing, Borders. Two library folders on two drives are therefore processed one after the other:

- `Scans` pops one root at a time and walks it to the end (`runner.rs:577`).
- Every other pass takes its work from the whole catalog, `ORDER BY f.volume, record` (`catalog/src/work.rs:30`), so a pass
  reads all of volume A and then all of volume B, with one pool of 8 readers (`jobs/src/pipeline.rs`).

Goal: files on different **physical drives** are scanned, imported, thumbnailed, hashed and border-checked at the same
time, while files on the **same spindle** (two partitions of one HDD) stay one stream, because parallel random reads on one
spindle thrash the head (`docs/plans/PLAN.md` "1 reader per HDD"; `docs/reference/IO.md`: 8 reads in flight reach
sequential speed). One drive must behave exactly as today.

Chosen by the user: a long-lived **lane per drive** running the whole pass sequence; **one status line per drive**; **the
CLI shares the module**; churn is accepted, do it properly. The cheaper alternative (one round, fan out inside each pass,
join at each pass boundary) was weighed and rejected: it cannot let a fast drive run ahead of a slow one, and "the SSD
folder gets thumbnails while the HDD still imports" is the point.

No design for this exists. `docs/plans/SCAN_OPTIMIZATION_PLAN.md` Phase 4 plans a device query (seek penalty, bus type) for
the listing workers; none was built (no `IOCTL_STORAGE_QUERY_PROPERTY` in `crates/`). Phase 1 builds it.

## Ground rules (repo conventions)

- Plan-doc style of `docs/plans/*.md`; this plan is copied to `docs/plans/MULTI_DRIVE_LANES_PLAN.md` when work starts.
- Implement a phase fully, then build and test once (no cargo between steps). Native Windows build in PowerShell with
  `QT_DIR` on PATH; the pg-dev container only for the Linux pass (`just check-linux`). No new branch: commit on `main`, only
  when asked, only the phase's files, `Cargo.lock` with any manifest change.
- Layering (`tools/crate-layers.txt`, `tools/boundaries.py`): `cfg(windows)`/`windows_sys`/`libc` only in `pg-platform`;
  `jobs` may not depend on `catalog` or `scan`; `unreachable_pub`; no `HashMap`/`HashSet`; no unwrap/index/panic in production
  code; `unsafe` needs a SAFETY comment; stored paths reach the disk only through `names::disk_path` (R-2); outcomes a front
  end words are `pg-model` data (R-1).
- **The working tree holds the user's uncommitted work** in `crates/jobs/src/pipeline.rs` (+273: `Read::Gather`, `Timing`,
  `LAST_REPORT`), `crates/import`, `crates/cli/src/lib.rs`, `crates/container`. Phases 3 and 5 edit `pipeline.rs` and the
  CLI: start them only after that work is committed, or merge onto it; never overwrite it.
- Measure with the keep rule (A/B at 6 pairs, turbo off, `keep.py --part time --rule bounded --margin 0.10`; cycles are
  diagnostic only). Trace a failure to its exact cause before fixing it; read the library source (Qt's is in
  `C:\Qt\6.12.0\Src`).
- Every phase leaves the tree working. Phases 1 to 3 change no behaviour; Phase 4 introduces lanes with a compatibility
  `Runner::progress()` (the busiest lane) so the app still works before the per-lane status lands in Phase 6.

## Design in one page

- **Drive** = the physical device(s) behind a volume: `DriveKey` (the set of disks) + `Kind` (`Rotational`, `Solid`,
  `Unknown`) + a label (`E:`). Runtime only, never stored (disk numbers change at reboot). Overlapping disk sets (a spanned
  volume on disks {1,2} and one on {2}) are merged by union-find into one drive. A volume id belongs to exactly one lane (a
  cloned disk shares its volume serial, `dir.rs:302-311`: those roots merge into one lane).
- **Fail closed**: if the drive of any library root can't be resolved, **every** root collapses into one lane (today's
  behaviour exactly). A mixed result would put two lanes on one spindle, the thrash this exists to avoid. The `Kind` only
  tunes readers; it never decides grouping.
- **Lane** = a long-lived thread (own `Service`, own `Catalog` connection, own child `Control`, own scan queue) per drive,
  running `Scans, Import, Thumbs, Placeholders, Hashes, Pairing, Borders` over **only its volumes** (`volume IN (…)`).
- **Runner** = a registry of lanes plus a coordinator thread for what is global: Crops/Cuts, topology, lane lifecycle,
  notice aggregation. An escape hatch in `FeatureCell` ("process drives one at a time", default off) forces one lane, for
  setups the probe can't see through (two VHDs on one HDD, Storage Spaces).
- **Stop and pause**: no shared stop flag. Each lane's `Control` has its own stop; the coordinator fans a stop out to the lane
  children (quit still does `service.quit()` first, then the stops, as `runner.rs:258-264`). Pause is two-level: global, and
  per lane; a lane is paused if either holds. A request for drive B stops only B's lane.
- **Pairing** is a global join but incremental (`pairs_owed`) and safe beside concurrent imports (it reads a `Reader`
  snapshot and guards on `pairing_epoch`). Each lane runs it after its Hashes pass, so it stays before Borders; it runs
  whatever is owed when the lane arrives.

## Phase 0: measure and probe first (calibration)

On the real machine (E: 22 TB Exos HDD, D: 2 TB NVMe), two scratch catalogs, the existing CLI (`gallery import --readers N
--workers M`, `PG_PIPELINE_STATS=1`):

1. Import folder X (E:) and folder Y (D:) one after the other, then at once as two processes. Gives the ceiling, whether the
   HDD import is disk-bound or decode-bound, and what two decode pools cost when they share the cores (sets the governor's
   CPU permits and memory budget in Phase 3).
2. D: alone at 8, 16 and 32 readers: the `Kind::Solid` default.
3. A second partition of one HDD, if any: both at once, to confirm the thrash.
4. **DONE (spike, 2026-11-17), result below.** The probe lives at
   `C:\Users\Torres\AppData\Local\Temp\claude\2c16f13ff155207f\01f9d099-b41f-4051-bae3-b38d6b69d39d\scratchpad\drive_probe.rs`
   (not in the tree: it fails the repo's clippy settings; `Win32_System_Ioctl` was added to the workspace features).
   Result, run elevated (this session is Administrator):
   - Volume GUID paths (`\\?\Volume{…}`) and drive-letter paths both open with **zero access** and return the same disk
     extents for C:, D:, E:, F:, G:.
   - Bus type and seek penalty match `Get-PhysicalDisk` for all five disks: disk 0 NVMe no penalty (D:), disk 1 SATA SSD no
     penalty (C:), disks 2, 3, 4 SATA HDDs with penalty (E:, F:, G:). No USB, VHD, Storage Spaces or iSCSI disk is present, so
     those bus answers are untested.
   - Not tested: a non-admin answer (this session is elevated, and no standard-user run was done), and a folder-mounted volume
     (it would need `mountvol`, a system change, not done here).
   - Still open for a later run: a non-admin account (standard user), a USB enclosure, and a Storage Spaces disk.
5. List-build cost at 2M rows (`catalog/src/fixture.rs`) of each work list with and without a one-volume filter. **Not done**:
   the plan spike below answered the plan shapes, and timing at 2M rows still needs a measurement run.

Output: a section in `docs/reference/IO.md` and the tuning table: `Kind` → `readers`, `Kind` → the scan's listing workers
(the walk's fixed `WORKERS = 4`, `scan/src/lib.rs:486`), the CPU permits, the per-lane memory budget. These numbers **tune**
the build; they don't decide whether it is built (the repo's gates set values, not go/no-go). One exception, which is about
the goal and not the churn: if the two-drive run shows no I/O gain at all (under the keep margin), the premise is wrong, and
that result goes to the user before any code is written.

## Phase 1: platform, the drive of a path (`pg_platform::drive`)

- `crates/platform/src/drive.rs`: `DriveKey`, `Kind`, `Drive { key, kind, label }`, trait `Drives { fn of(&self, path) ->
  io::Result<Drive> }`, `SystemDrives` (unit struct, the `RootFacts`/`SystemFacts` pattern, `dir.rs:68-85`) so consumers test
  against a fake. Union-find merge of overlapping keys is a pure function here.
- Windows: `volume_root` (`GetVolumePathNameW`, reuse `dir.rs`), then `GetVolumeNameForVolumeMountPointW` and open the
  `\\?\Volume{GUID}` (no trailing `\`), so a folder-mounted volume (`C:\mnt\vol\photos`, which `refusal.rs:41-64` accepts)
  works where `\\.\X:` would not. `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS` (retry on `ERROR_MORE_DATA`); per disk
  `IOCTL_STORAGE_QUERY_PROPERTY` `StorageDeviceSeekPenaltyProperty` and the bus type.
- **Kind is never `Solid` on a bus that lies about it**: USB (UAS and SAT bridges report "no seek penalty" for HDDs), file-backed
  virtual (VHD), Storage Spaces, iSCSI, unknown bus → `Unknown` (HDD defaults). Spanned volume: `Rotational` if any disk is.
  If the seek-penalty query needs more access than a normal user has (Phase 0 spike), `Kind` is `Unknown`; the key, from the
  volume's extents, needs none.
- **Disks whose physical backing can't be seen** (file-backed virtual, Storage Spaces, iSCSI, unknown bus: two of them may
  share one spindle) get the single `DriveKey::Opaque`: they all group into one lane, serialized with one another and running
  beside the visible physical drives. USB disks are distinct physical drives and keep their own keys.
- Linux: `stat` → `major:minor` → `/sys/dev/block/M:m`, a partition's parent disk, `slaves/*` recursively for `dm`/`md`,
  `queue/rotational`; major 0 (btrfs, overlay, ZFS) → error. Other targets → error.
- `Cargo.toml:54` windows-sys features gain `Win32_System_Ioctl` (+ `Cargo.lock`).
- Callers run `of` on a probe thread with a deadline, **one in-flight probe per root**, never respawned while pending (a
  probe on a dead drive can't be cancelled; `GUI_THREAD_PLAN.md:1134`), and never on the GUI thread.
- Tests: real call on a `pg_testkit::run_folder` scratch folder; the extent-buffer parser as a pure function (1 extent, 2,
  `MORE_DATA`, truncated); the kind-combination and bus-override tables; union-find (spanned/overlap); Linux sysfs parsing
  over a fake `/sys` tree (partition → disk, `dm` → slaves, `rotational` 0/1, major 0).

## Phase 2: catalog, volume sets, busy retry, shorter holds

- `VolumeSet { All, Only(Vec<u64>) }` (volumes are `u64`, cast to `i64` at the edge as `writers.rs:36`).
  `pending_import(retry, &VolumeSet)`, `pending_hashes`, `pending_borders`, `pending_placeholders` (`work.rs`) and the thumb
  cache's `pending_in` (`thumbcache/src/reader.rs:71-88`) take it; `Only` adds `AND f.volume IN (…)` to both branches of the
  `retry_errors` UNION. `All` emits today's SQL. **The `bound` counts (`work.rs:390, 404, 424, 443`) are filtered too**, or
  each lane sizes `WorkList::with_capacity` for the whole catalog.
- **Work-list plans, measured on the real schema (spike, 2026-11-17; the catalog never runs `ANALYZE`, so these are the plans
  production gets).** No migration is needed for any list; the spike replaced the earlier "queue rows carry `(volume, record)`"
  plan:
  - **Import, no retry:** `files_to_import (volume=?)`, no sort. Keep it, and assert it in a test.
  - **Import, retry UNION:** `USE TEMP B-TREE`, with or without the filter. It runs only on Retry, so the sort is accepted.
  - **Placeholders:** with `AND f.volume IN (…)` it is `SEARCH f USING INDEX sqlite_autoindex_files_1 (volume=?)`, a sort of
    the lane's rows only. Without the filter it scans `files_present`. Already per-lane cost; no change beyond the filter.
  - **Borders and hashes:** with the filter placed on the query as written, the planner drives from the queue
    (`SCAN a USING COVERING INDEX analysis_borders_due`, `SCAN h`) and probes `files` for every lane's rows: O(total owed)
    per lane. **Fix:** write the join with `files f` first (`FROM files f CROSS JOIN analysis a …` and
    `FROM files f CROSS JOIN hashes_owed h CROSS JOIN analysis a …`). The plan then becomes `SEARCH f USING INDEX
    sqlite_autoindex_files_1 (volume=?)` with the queue probed by key: O(the lane's files), one sort of the lane's rows.
    The `CROSS JOIN` keyword is what fixes the join order; the SQL meaning is unchanged.
  - **Thumbs, all of the pass:** `SCAN f` on the `(volume, file_ref)` index, with a partial sort on the last term. Fine.
  - **Thumbs, since the checkpoint (`seq > ?2`):** without a change, the filter `f.volume IN` switches the planner to the
    volume index and scans every file of the lane, losing the incremental pass. **Fix:** `+f.volume IN (…)`, which keeps
    `SEARCH f USING INDEX files_seq (seq>?)`. (The existing `+f.volume` in the ORDER BY does the same job for ordering.)
    `INDEXED BY` cannot be used here, since `thumb_sources` is a view.
  - **`volumes()`:** a plain `SELECT DISTINCT volume` is a range scan of the whole index, O(files), not O(volumes). The
    recursive-CTE loose scan is still needed for the unresolved remainder; it is not spiked yet.
  - Phase 0 item 5 still measures these at 2M rows, for time, not plan shape.
- The unresolved remainder (volumes not under any lane) comes from `Catalog::volumes()`, a recursive-CTE loose index scan
  over `UNIQUE(volume, file_ref)`; it can't filter `missing` and doesn't need to (the lists exclude missing). Lane volumes
  come from the roots (`pg_scan::root_volume`, Phase 4), not from `volumes()`.
- **Busy handling at the right place.** No pass handles `Error::is_busy` today (a busy `tx()` fails the pass, the round is
  `RoundFailed`, `again = false`). Busy occurs only at `BEGIN IMMEDIATE` (`transaction.rs:13-16`), so retry **there**: a
  bounded loop in `write_transaction` with a stop check, which covers every writer without per-writer currency reasoning.
  A "retry a batch" is unsound: `import::store` drains inside the transaction (`import/src/lib.rs:138-140`).
- `Catalog::remove_root` (`roots.rs:93-115`) is one transaction over the whole subtree with per-row triggers, a long write
  hold on a big root. Batch it like `mark_missing` (`roots.rs:200`, 1000 rows), keeping the lease deletion and the
  "walk ends as `Removed`" guarantee in its first transaction.
- Tests: result equality of `All` vs the union of per-volume lists; `(volume, record)` order; plan assertions from the spike: the import list (no
  sort) and the volume-filtered borders, hashes and thumbs-since-checkpoint lists (driven from `files`, the `+f.volume` form
  for the checkpoint); placeholders per lane; the retry import's sort accepted; `volumes()` must use the loose scan, not a
  full range scan; an id above `i64::MAX`; the busy retry (a held write lock released after N ms is waited out; a stop ends
  the wait); `remove_root` in batches keeps its guarantees.

## Phase 3: jobs, `Control` for lanes, shared CPU and memory

- `control.rs`: shared `Pause { paused, resumed }` behind an `Arc`; `Control::lane(&self) -> Control` is a child with its own
  `stop` (set under the same pause lock, so the "a stop can't be missed" invariant at `control.rs:118` holds), its own
  `progress` and events forwarded to the root bus tagged by lane. The root keeps a registry of children: **`set_paused` updates
  every child's `Running` clock** (today it updates only its own, `control.rs:176-184`, so each lane's elapsed time and rate
  would be wrong while paused). Children share the root's `on_change` and `on_ledger_change` hooks (both root-only
  `OnceLock`s today, `control.rs:187-204`). `request_stop` on a child stops that lane; the root's fans out to all children.
  `PhaseGuard` of a child never idles a sibling. `Event::{Progress, Finished}` carry a `LaneId`; `Control::snapshots()`.
  `snapshot()` stays for single-`Control` users.
- **Pause is two-level**: the root's global pause and a per-lane pause (pause the HDD's import, keep working the SSD's); a
  lane is paused if either holds, and each lane's clock stops for either.
- `Options::for_drive(kind, priority)` sets `readers` from Phase 0 (`Rotational`/`Unknown` 8 as measured).
- **`Governor`** (one per runner, `Options.gov: Option<Arc<_>>`, `None` = today), always built:
  - CPU: a fair ticket-FIFO `CpuGate` of `available_parallelism` permits around each worker's `process`, so two lanes' decode
    pools share the cores instead of oversubscribing them (BelowNormal/Idle priorities don't make them fair).
  - Memory: each lane keeps its own `Pool` (no cross-pool byte account: `Pool::take` grants strictly in list order, and a
    shared account could deadlock it), but the budget becomes resizable (`Pool::set_budget`) and the governor sets it: the
    measured knee per lane up to a cap (a share of RAM), split evenly when the cap binds, rebalanced when a lane starts or
    retires. It also sets the page cache of each lane's connections, which take a parameter instead of the fixed 256 MB
    (`schema.rs:595`).
  - Scan listing workers: `Pacing.workers` per lane from `Kind` (today a fixed `WORKERS = 4`, `scan/src/lib.rs:486`).
- `LAST_REPORT` (a process-global static) becomes a value in `Stats`, so concurrent passes don't overwrite each other's
  `PG_PIPELINE_STATS` report (after the user's `pipeline.rs` work is committed).
- Tests: child progress isolation; a lane stop leaves siblings running; the root stop reaches every lane including one
  waiting in `checkpoint_until`; global pause reaches every lane and each lane's clock stops; a per-lane pause leaves the
  others running; a phase begun while paused; hooks fire from children; existing pipeline tests unchanged. `CpuGate`
  fairness and permit bound; `Pool::set_budget` shrinking and growing under load without deadlock (a waiting take is granted
  alone, as an oversize file is); the governor's split and rebalance.

## Phase 4: library, lanes and the runner as a registry

- `crates/library/src/passes.rs` (moved from `runner.rs`): the `Pass` impls over a `LaneCx { inner, catalog, ctl, volumes }`,
  used by the runner's lanes and by the CLI (Phase 5).
- `lanes.rs`: `Topology::resolve(roots, &dyn Drives, probe)`: probes **concurrently**, each with its deadline and one in-flight
  probe per root; healthy drives start their lanes at once. Root → drive (Phase 1) and volume (`pg_scan::root_volume`, a small
  `pub fn` over `Lister::folder_volume`); union-find; fail closed (a **mounted** root whose drive can't be read, or the escape hatch on → one lane). A root that is
  **offline** (path not found, device not ready, device not connected: the error classes at `WEB_CLIENT_DESIGN.md:1069`) is
  not unresolved: it gets no lane, its files stay pending under the existing offline-file notice, so an unplugged external disk
  never serializes the rest; it is re-resolved when it comes back (each round and on the TTL).
  Cached per path with a TTL so a dead drive doesn't stall each round. `Runner::scan` is called from the GUI thread: it
  queues the root for the coordinator to route and never probes.
- `Lane { id, drive, queue (scans, busy), scanning slot, ctl: child, service }`, each with its own `Catalog::open`: the
  per-connection `temp.scan_seen` (the only temp table) is private, and leases refuse only overlapping folders
  (`roots.rs:124-150`), so two drives never conflict. A lane's `run` is today's loop (`runner.rs:433-465`): its own
  `clear_stop`, `round_caught`, `again`, and the Busy requeue timer; after a busy `RoundFailed` it retries on a timer instead
  of sleeping until the next wake.
- **Routing of every request** (today each does `request_stop` + wake of the one thread):
  - `scan(roots)`: the lane of each root; wakes that lane only.
  - `stop_scans_under(root)`: per lane, `retain` queue and `busy`; wake only a lane whose running scan is under the root.
  - `wake()` (the desktop calls it for every turned file, `desktop/library.rs:148`): route by the volumes of the turned files;
    broadcasting a stop would kill the parallelism and rebuild each lane's work list. Decide the exact signature by reading
    that call site and the writer's `Turned`.
  - `HeightCell` / `FeatureCell` `listen` hold one `Option` wake (`runner.rs:92-94, 130-132`): the registry installs **one
    broadcast closure** once; a height change ends each lane's Thumbs pass after its batch.
  - `save_crops` / `cut_borders`: the coordinator, on its own child `Control` (`Phase::Crop`, shown as a "job" entry), parks
    **only the lanes that own the files' volumes** through a gate, runs, releases. The gate is a `Mutex`+`Condvar` state
    machine whose waits check the lane's stop and quit (an `RwLock` read can't be interrupted, so `Runner::stop` could
    hang); other lanes keep working.
  - `stop(timeout)`: one shared deadline across the coordinator and every lane (today `Service::stop_within` is per service).
  - `set_paused(lane, paused)` / `is_paused(lane)`: `None` is the global pause (today's `toggle_pause`).
- **Lifecycle**: a lane is created on first need and retired (stop + join) when its drive has no root left. The retire
  decision and the routing share one registry lock, never held across `Service::stop`. A coordinator panic is caught like a
  round's (`round_caught`). A root that moves lanes mid-scan gets `ScanError::Busy` and the existing 5 s retry (accepted).
- **Pairing gate**: a mutex taken with try-lock and a stop-checking wait (a blocking lock would hide a lane's stop behind
  another lane's multi-minute `pair()`), poison tolerated. A lane runs Pairing only when its round stored hashes or analyses,
  or when it is about to go idle, and not within a cooldown of the last pairing while other lanes are busy: each `pair()`
  with owed rows loads every hash and rebuilds the join store (`pairs.rs:144, 165`), and every import batch on another lane
  bumps `meta.pairing_epoch`, which puts pairing's writes on the guarded slow path (cost, not correctness). The cooldown
  length is set by a measurement.
- **Notices without a model change.** Notice keys are persisted and `Part` is a `&'static str` (`notice.rs:887-890`), so a
  runtime drive id can't be in them; the triggers are global (`Trigger::Pass(pass)`, `Op::Round`, `CacheFull`). The
  coordinator aggregates instead: a per-lane map `pass → (unreadable, complete)` whose **sum** is posted as
  `UnreadableFiles { pass, count, drives }` (the drive labels are payload, not key: a `pg-model` change that
  `pg_client::text` words as "12 files on E: couldn't be read", so the user sees which drive), resolved only when every lane that ran the pass finished clean; `RoundFailed`,
  `RoundPanicked`, `CacheFull` and `LedgerNotWritten` are per-lane sets resolved when the set empties. Per-root scan notices
  already work (`Key::all("scan-note", Some(&path))`, `runner.rs:624`).
- **Thumbnails**: the checkpoint is one row per height (`ThumbCache::checked`, `thumbcache/src/lib.rs:280`; `ON CONFLICT(height)`
  in `writer.rs:282-293`), so a fast lane finishing first would skip a slow lane's files. Key it by `(height, volume)` (a
  `thumbs.sqlite` migration that carries `evictions`; the old checkpoint is dropped, costing one full recheck). `pending_in`
  takes `?2` per volume; a lane with several volumes uses the minimum. A volume with no checkpoint reads everything. A snapshot
  invalidated by another lane's eviction only means that lane redoes its check next round. `shrink()` is serialized by the
  cache's writer thread.
- **Rounds**: `Change::Round` is published per lane round (the trace stage `pg_stage_round_done` and the inbox poke at
  `desktop/library.rs:158-165` keep meaning, and a multi-hour HDD import no longer hides the SSD's rounds); `rounds()` is the
  sum; `Runner::idle()` says no lane is busy, for the measuring harness.
- **Linux nested mounts** (a root holding another device): `ScanStats` returns `(volume, probe)` for volumes met; the next
  topology refresh moves them to the right lane. On Windows a root is exactly one volume (`refusal.rs:41-64`).
- **Memory** is the governor's (Phase 3): a lane has its catalog, `thumbs::run_while`'s own `Catalog::open`
  (`thumbs/src/lib.rs:229`) and `pending_in`'s 128 MB (`reader.rs:27`), plus the coordinator's, and all of them take their
  page-cache size from it, so the total is a stated cap (the lanes' pools plus their page caches), not lanes × 256 MB.
  Verified by RSS with two lanes.
- Tests (extend the in-file runner tests, which build `Inner` as a literal at `runner.rs:959`; add `tests/lanes.rs`), with a
  fake `Drives` mapping scratch folders to keys:
  1. two drives run concurrently (lane A's pass blocks until lane B has started; times out otherwise);
  2. two roots on one drive share one lane, in order;
  3. a scan request for B doesn't stop A's import (A keeps advancing);
  4. removing a root while its lane imports (`Removed`, no writes after), and the lane retires;
  5. global pause and quit reach every lane, a per-lane pause only its lane; `stop` ends all threads inside one deadline,
     including a lane parked at the crop gate;
  6. a duplicate across drives is paired; Pairing runs before Borders in each lane; the cooldown;
  7. two lanes' unreadable counts add up and resolve only when both are clean; per-lane `RoundFailed` doesn't wipe another's;
  8. the thumb checkpoint: A completes, B is stopped, the next round redoes only B;
  9. one unresolved root collapses everything to one lane (= today); the escape hatch likewise;
  10. a crop on A's files parks only A; a turned file wakes only its lane;
  11. two lanes' batches never exceed the busy budget (`watch_holds`/`take_holds`); a busy `BEGIN` is retried;
  12. every existing runner test passes with one lane.

## Phase 5: CLI on the same module

- `pg_library::lanes::run_once(catalog_path, roots, opts, ctl, passes)` drives Topology + per-lane threads over `passes.rs`;
  `gallery scan | import | thumbs` use it; `--serial` forces one lane (today's path). `report.rs` `with_progress`
  (`report.rs:75-81`) prints per-lane lines from the tagged events. The CLI can now measure exactly the app's parallel path.
- Tests: CLI integration with two scratch roots, with and without `--serial`.

## Phase 6: app and front ends

- `pg-model`: `LaneId`, `LaneInfo { label, kind }`; `Progress` unchanged.
- As built (the stacked rows): the rows are a `LaneModel` (`crates/desktop/src/lane_model.rs`) over `Library::lane_lines()`
  (`LaneLines`: the crop's progress and each lane's, `LaneProgress`). The `lanes_status` idea is gone: a row's text is
  `pg_client::text::pass_status`, or `lane_paused_status` while it is paused. `Status.lanes` is gone with it. A row lasts
  while its lane had work less than `HOLD` ago (`lane_model.rs`), or while it is paused. The job's text and its failed count are on
  the rows, not in the status bar, which keeps only the selection, the write status, the toast and the Inbox. The shared
  progress bar and the global Pause are gone: each row has its own bar, Pause and failed count (`LaneRow.qml`), and
  `setLanePaused(number, paused)` names the lane by its number. The jobs' global pause is still in the library, with
  no button to set it.
- Tests: `text` single and multi-lane; desktop `just qmltest` for one and two lines; the runner tests' `finished_order`
  (`runner.rs:912-925`) adapted to the tagged `Event`; `just shots` refresh.

## Phase 7: docs

`docs/plans/MULTI_DRIVE_LANES_PLAN.md`; `docs/reference/IO.md` (Phase 0 numbers); `docs/reference/STORAGE.md` drive-kinds
table (real detection, the USB-bridge override, the Storage Spaces/VHD limit and the escape hatch); module docs of
`runner.rs`/`lanes.rs`; `crates/README.md` (no new layer edge: `library` already depends on `platform`, `catalog`, `jobs`,
`scan`, `thumbcache`); the `PLAN.md` "1 reader per HDD" line; `SCAN_OPTIMIZATION_PLAN.md` Phase 4's device-query note says
Phase 1 here built it.

## Not doing

- Persisting the drive in the catalog (disk numbers change across boots); a drive id in notice keys (aggregate instead).
- A byte account shared *across* pools (deadlock against `Pool::take`'s order); the governor resizes each pool instead.
- More than one lane per drive, a lane per partition or per root, network drives (already refused).
- Pairing per lane (it is a global join, gated).
- Resolving the physical disks under a VHD or Storage Spaces virtual disk: they group as one opaque lane, and the escape
  hatch remains.

## Decisions

- D1 Lane per physical drive, whole pass sequence per lane (user).
- D2 Fail closed: any unresolved root collapses everything to one lane; `Kind` tunes readers only.
- D3 `Unknown` kind uses the HDD defaults (8 readers); a bus that lies is never `Solid`.
- D4 One status line per drive; the CLI shares the module with `--serial` (user).
- D5 No shared stop flag: per-lane stops, fanned out by the coordinator; pause shared with a children registry.
- D6 Busy retried at `BEGIN IMMEDIATE`; `remove_root` batched.
- D7 Notices aggregated in the coordinator, no model change.
- D8 Crops/Cuts park only the lanes owning the files' volumes, through an interruptible gate.
- D9 The governor (fair CPU permits, resizable per-lane memory budget and page caches, listing workers by kind) is always
  built; Phase 0 sets its numbers.
- D10 Every work list is index-served per volume, by join order and the `+f.volume` form for the checkpoint, not by a migration
  (spike results in Phase 2).
- D11 Pause is global and per lane.
- D12 Drives whose backing can't be seen group into one opaque lane.

## Verification

- `just lint`, `just test`, `just qmltest` natively on Windows; `just check-linux` in the pg-dev container for the sysfs path;
  `just perf` (the callgrind gate, ±1%, `docs/perf/baseline.json`) since `pipeline.rs` and the work lists are hot paths, and
  the baseline is re-recorded by the repo's rule if a change is intended.
  The tests listed per phase, built and run once at each phase's end.
- Real two-drive run: the app with a folder on E: and one on D:: two status lines; D: gets thumbnails while E: imports;
  adding a folder on D: during E:'s import doesn't interrupt it; both disks active in Task Manager; RSS with two lanes.
- Throughput: `gallery import` on E: and D: as in Phase 0, against the parent commit, by the keep rule; one-drive and
  `--serial` runs must not regress.
- Settle by reading, not guessing: the `desktop/library.rs:148` wake call site; whether a zero-access
  `\\.\PhysicalDriveN` handle works without admin (Phase 0 spike); the thumbs `pending_in` plan with a volume filter
  (`reader.rs:66-70` orders by `file_ref & mask`, which may not be index-served); `Maker`/`Analyzer` per-worker memory (sizing a
  `CpuGate`); `thumbs.sqlite` migration mechanics; the small pre-existing window between `queue.pop_front` and setting
  `scanning` (`runner.rs:582-589`).

## Ledger: open gaps after the lane commit (`79e3010f`)

Everything here is either not built, built but untested, or verified but not run. Status: `open` until the work lands and
its check passes, then `done` with the commit that closed it. Phase numbers refer to the plan above.

### Not built

| ID | Phase | Gap | Blocked on | Status |
|---|---|---|---|---|
| N1 | 3 | `Options::for_drive(kind, priority)`: readers per drive kind | Phase 0 measurements (R1) | open |
| N2 | 3 | Scan listing workers per drive kind (`WORKERS = 4` is still fixed) | Phase 0 measurements (R1) | open |
| N3 | 3 | Per-lane page-cache sizing (catalog and thumbs connections keep their sizes) | Governor memory numbers | open |
| N4 | 2 | `Catalog::volumes()` loose-scan (recursive CTE over `(volume, file_ref)`) | none; needed only if a caller needs it | done (uncommitted: `work.rs`, tested by T6) |
| N5 | 1, 4 | Linux nested mounts: a root holding another device gets its volumes' lane | none (Linux only) | open |
| N6 | 4 | Turned-file wakes route to the lanes of their volumes; today `wake()` stops every lane. Needs `Change::Turned` to carry file ids, and the writeback crate to publish them | none | done (uncommitted): `Writer::take_turned` drains the ids and `Lanes::wake_files` routes them; `Change::Turned` stays id-less. Routing is tested only through `lanes_with` (L5) |
| N7 | 4 | Pairing cooldown while other lanes are busy | a measurement of `pair()` cost (R1) | open |
| N8 | 4 | Notices name the drive: `UnreadableFiles { pass, count, drives }` as a `pg-model` change, worded by `pg_client::text` | none | done (uncommitted; the notice's `drives` is stored only when set, so old notices read as before) |
| N9 | 5 | `run_once(catalog, roots, opts, ctl, passes)` entry point, so the CLI and the job share one module | none | done as `lanes::command_groups` and `lanes::run_each` (uncommitted); the CLI calls them, and the CLI's import and thumbs passes still call the pass crates directly, not `passes.rs` |
| N10 | 5 | `report.rs` prints per-lane lines from the tagged events (today a lane gets a heading only) | none | open |
| N11 | 6 | Per-lane rows and per-lane pause toggles in `Main.qml` (today one joined status line) | none | done (uncommitted): the rows are a `LaneModel`, stacked in `StatusBar.qml`, each with its own bar and pause (`LaneRow.qml`); `qmltest` covers them; not yet seen on screen (N13) |
| N12 | 6 | `LaneInfo` in `pg-model` (decide: `LaneProgress` may serve; if so, close the item as not needed) | decision | closed: not needed; `LaneProgress` serves the front ends |
| N13 | 6 | Screenshots of the stacked rows (`just shots`: the `lanes` scenario, from `tools/shots/lanes.txt`) | N11 | open: the scenario is added, its reference is recorded from the feature commit, and the screenshot read |
| N14 | 7 | `docs/plans/SCAN_OPTIMIZATION_PLAN.md` Phase 4 note that Phase 1 here built the device query; `crates/README.md` check | none | done (uncommitted): the note is in the scan plan; `crates/README.md` names the lanes |

### Built, not tested

| ID | Phase | Gap | Status |
|---|---|---|---|
| T1 | 3 | `Pool::set_budget` under load: a shrink holds grants back, a grow admits waiters, no deadlock, a budget change is seen within `BUDGET_WAIT` | done (uncommitted; four tests in `pipeline.rs`, pass) |
| T2 | 1 | Linux sysfs parser over a fake `/sys` tree: partition to disk, `dm` to slaves, `rotational` 0 and 1, major 0 | written (uncommitted; in `drive.rs`, Linux-only, so they run in V2 and have not run yet) |
| T3 | 2 | Work lists: result equality of `VolumeSet::All` with the union of per-volume lists, in `(volume, record)` order | done (uncommitted; in `work.rs`, pass) |
| T4 | 2 | Work-list plan assertions: the import list has no sort; the volume-filtered borders, hashes and thumbs-since-checkpoint lists drive from `files` | done (uncommitted; pass). Caveat: the spike did not reproduce the pre-fix borders plan, so the test pins the `CROSS JOIN` form, not the cause |
| T5 | 2 | `remove_root` in batches keeps its guarantees (lease deleted first; the walk ends as `Removed`) | done (uncommitted; `roots.rs`, pass). Doc mismatch in `remove_root` (its doc names one transaction for the marks; the code batches them) is not fixed |
| T6 | 2 | `volumes()` must not be a full range scan; an id above `i64::MAX` is refused | done (uncommitted; the id refusal is the existing `a_volume_set_takes_only_ids_a_catalog_can_store`) |

### Lane tests the plan asked for and the tree lacks (`tests/lanes.rs`: 7 written, of 12)

| ID | Plan item | Status |
|---|---|---|
| L1 | Two drives run concurrently: one lane's pass blocks until the other has started | open |
| L2 | Pairing runs before Borders in each lane; the pairing cooldown | open (cooldown is N7) |
| L3 | Two lanes' unreadable counts add up; they resolve only when both are clean (`UnreadableFiles`) | done at the tally's unit level (uncommitted; the sum, the drive names and the resolve are tested in `tally.rs`), not through `tests/lanes.rs` |
| L4 | Thumbs checkpoint: A completes, B is stopped, the next round redoes only B | open |
| L5 | A crop on A's files parks only A; a turned file wakes only its lane | partly (uncommitted): `lanes_with` is unit-tested in `coordinator.rs`; the crop park and the turned wake are not tested |
| L6 | Two lanes' batches never exceed the busy budget; a busy `BEGIN` is retried | open |
| L7 | Global pause and quit reach every lane; a per-lane pause only its lane; `stop` ends all threads inside one deadline, including a lane parked at the crop gate | partly (uncommitted): `a_per_lane_pause_pauses_only_its_lane` passes; the crop gate is not built, so the stop-while-parked case is not tested |

Written and passing: two drives make two lanes; folders on one drive share a lane; an unknown drive collapses the lanes;
serial makes one lane; a retired drive's lane ends; a stop ends every lane; each lane reports its drive's label; a
per-lane pause reaches only its lane (L7, partly).

### CLI tests

| ID | Plan item | Status |
|---|---|---|
| C1 | CLI integration with two scratch roots, with `--serial` and without: `scan`, `import` and `thumbs` each | done (uncommitted): `two_roots_scan_import_and_thumbs_agree_with_and_without_serial` in `cli/src/lib.rs`, passes |

### Verification not run

| ID | Check | Status |
|---|---|---|
| V1 | `just test` for the whole workspace (run so far: per-crate `cargo test`) | run this session (`cargo test --workspace --locked --no-fail-fast`): 1339 passed, 1 failed: F1, the flaky thumbs test, which predates this work |
| V2 | `just check-linux` in the pg-dev container (the sysfs path) | run this session: `just check-linux` passes (lint, both module-cycle graphs, 1337 tests on Linux, including the T2 sysfs tests) |
| V3 | `just perf` (callgrind gate, ±1%; `pipeline.rs` and the work lists are hot paths) | open |
| V4 | `just qmltest` | done: 108 passed, 0 failed (this session) |
| V5 | A real two-drive run on E: and D: (two status lines; D: gets thumbnails while E: imports; both disks active; RSS with two lanes) | open |
| V6 | Keep-rule A/B throughput against the parent commit (6 pairs, turbo off, `keep.py --part time --rule bounded --margin 0.10`) | open |

### Measurements

| ID | Item | Status |
|---|---|---|
| R1 | Phase 0 timed runs (items 1 to 3 and 5): E: and D: alone and together, D: at 8, 16 and 32 readers, the list-build timing at 2 M rows. Feeds N1, N2, N3 and N7 | open |

### Claims to correct

| ID | Where | What is wrong | Status |
|---|---|---|---|
| D1 | Status table, phase 3 | Lists the pool's live budget and the governor as built; neither has the tests in T1 | done: T1's tests are written and pass |
| D2 | Status table, phase 2 | Lists the work lists as index-served; T3 and T4 are what would show it | done: T3 and T4 pass; T4's caveat is in its row |
| D3 | Status table, phase 4 | Presents `tests/lanes.rs` as covering the lanes; it has 7 of the 12 planned lane tests (see L1 to L7) | done: the table now says 8 tests (one added for L7); L1, L2, L4 and L6 remain open |
| D4 | Commit `79e3010f` message | Says the lanes are built with the listed exceptions, without naming the untested items (T1 to T6, L1 to L7, C1) | open (history: correct in the ledger, not by amending) |

### Found while verifying (not in the original ledger)

| ID | Where | What | Status |
|---|---|---|---|
| N15 | `crates/library/src` | The module cycle `coordinator <-> lane <-> lanes <-> passes <-> runner`, in `79e3010f` (checked: it is in HEAD). `runner` owns `Shared` and `LaneProgress`, which `coordinator`, `lane` and `passes` import; `lane` and `passes` import each other (`Cx`, `Inner`). Fix: move `Shared` and `LaneProgress` to their own module (re-exported from `runner`), and separate the pass sequence from `Inner` so `lane` and `passes` don't import each other | done (uncommitted): `inner.rs` holds `Inner`, `shared.rs` the shared state and the cells, `pair_gate.rs` the pairing gate, and `probe_place` moved to `lanes.rs`; the text scan and both compiler graphs report no cycle |
| N16 | `crates/platform/src/drive.rs` | The module cycle `drive <-> drive::facts <-> drive::linux <-> drive::win <-> drive::windows_facts`, in `79e3010f` (in HEAD). Fix: move the types `Drive`, `DriveKey` and `Kind` into a leaf module that the parts import, so the parent only calls them | done (uncommitted): the types are in `drive/types.rs`, which the probes import; no cycle in the text scan or the Windows and Linux compiler graphs |
| N17 | `crates/catalog/src` | The Windows compiler graph's cycle `core <-> work` (the Linux graph is clean). cargo-modules files the `Catalog` methods of `work.rs` under `core`, and those methods use `VolumeSet` and `PlaceholderWork`, which lived in `work`. Fix: both types moved to `types.rs`, a leaf (`pg_catalog::VolumeSet` and `PlaceholderWork` keep their paths) | done (uncommitted); the Windows compiler graph reports no cycle |
| F2 | `crates/app/src/inbox.rs` test `the_close_gives_up_after_its_deadline` | Timing-dependent: it asserts the close returns within a 3 s window after the drain. It failed once in the parallel workspace run and passes 3 of 3 alone. `inbox.rs` is not changed here | open |
| F1 | `crates/thumbs/src/lib.rs` test `a_file_that_doesnt_decode_is_recorded_and_one_unread_is_not` | Flaky: run alone, it fails 2 of 5 times with `left: (2, 2)`, `right: (1, 1)` at the second run (a recorded failing file is listed again). It fails the same way on an export of HEAD, so it predates this work. It passed in the full workspace run. Not traced yet | open |
