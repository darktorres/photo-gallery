# Adaptive reader tuning per drive

Part C of one plan that was split in phase 0. The combined text is in commit `a8efee20`. The other parts are `HEADLESS_SESSION_PLAN.md` (Part A) and `STORAGE_AND_CLEANUPS_PLAN.md` (Part B).

**Global phase order:** A1, A2 (`HEADLESS_SESSION_PLAN.md`); B3, B4 (`STORAGE_AND_CLEANUPS_PLAN.md`); C5 to C10 (this document). Phases C5 to C7 do not depend on the CLI removal. Phases C8 to C10 wait for B3, because they change `Options`, and C9 also needs `RunnerConfig` from A1.

## Context

The pipeline reads files with a fixed number of reader threads (`READERS = 8`, `crates/jobs/src/pipeline.rs:56`) and a fixed memory budget (`(readers + workers) × 12 MiB`, `pipeline.rs:53, 89`). Both were chosen on one machine. Measurements in this work show that the right reader count depends on the drive and on the cache state:

- Raw whole-file reads, cache bypassed (24 MB files, MB/s at 1/2/4/8 readers): Exos 178/102/131/115, Toshiba 48/50/46/33, WDC 101/76/69/51. One reader was best on all three HDDs. File latency grew 6–17× from 1 to 8 readers.
- Raw random 1 MB reads, Exos: 94/132/148/156 MB/s at 1/2/4/8 readers. p99 latency rose from 31 ms to 293 ms.
- The real pipeline with the real thumbnail maker, warm cache (64 GB RAM, 12 CPUs): about 40–45 files/s on all three HDDs at every count. Decoding is the limit there.
- `docs/reference/IO.md` (earlier, cold real files) measured the opposite: 8 reads in flight reached about 235 MB/s, against 64 MB/s at depth 1. The controller therefore measures instead of trusting either.
- Not yet measured: SSDs, and cold-cache runs with decoding included.

## Decisions (made by the user)

- The app's lanes use the controller. There are no public tuning flags. Counts and memory come from the governor and the controller (see Part A).
- Developer pins are typed: `RunnerConfig.pin_readers` (Part A). A pinned lane runs `Readers::Fixed(n)` and writes no profile.
- Learned values are persisted per physical disk and per pass kind, in their own file.
- Every drive's lane is tuned. The app already runs every pass per lane (`crates/library/src/passes.rs`), so `hash` and `borders` are per lane too. `pair` reads no files through the pipeline's readers, so it is out of scope.
- Cold-cache measurements use an elevated standby-cache purge. **Ask the user before each purge**, because it changes machine-wide memory state.
- The memory budget is a soft target, as decided in `docs/plans/TYPE_MISMATCH_VALIDATION.md` item 2. Each process keeps its own pool.
- Memory governance is part of this plan, not deferred. It closes `docs/plans/TYPE_MISMATCH_VALIDATION.md` item 3, the lanes' slots and memory cap. (That item is section 3 of the file, not item 5.)

## Design

### The controller: `crates/jobs/src/tuner.rs` (new)

A pure, deterministic state machine. It has no threads and no clock trait. The repo has none, so the convention is a `Duration` or `now` argument, as in `control.rs` `Running::mark` and `coordinator.rs` `Probe`.

```rust
pub struct Sample { elapsed: Duration, paused: Duration /* ctl.paused_total() diff */, reads: u64,
                    disk_ns: u64 /* around the read only: no pool wait */, bytes: u64, oldest_read_ns: u64,
                    work_wait_ns: u64, permit_wait_ns: u64, work_busy_ns: u64,
                    pool_order_wait_ns: u64, pool_capacity_wait_ns: u64, active_readers: f32, files: u64 }
pub struct Decision { target: NonZeroUsize, reason: Reason }
impl Tuner { pub fn new(cfg: TunerConfig, prior: Prior) -> Self; pub fn update(&mut self, s: &Sample) -> Decision }
```

**Signals per window** (about 3 s, and at least 16 reads):

- Throughput, in bytes/s and files/s, and the mean disk latency per read.
- `starved = work_wait / (workers × elapsed)`. Workers are idle waiting for reads, so the pass is read-bound and may explore.
- `blocked = (pool_capacity_wait + permit_wait) / ((active_readers + workers) × elapsed)`. Readers wait for a buffer or a slot because the workers are behind, or workers wait for a CPU permit because other lanes hold the cores. Either way the pass is decode-bound, and the controller holds.

**Why these meters.** The first design took `blocked` from `read_send`, and said CPU contention shows up as `work_wait`. Neither holds in the code:

- `read_send` times `raw_tx.send` into a channel bounded by `pool.slots` (`pipeline.rs:267, 309-312`). Leases are capped by the same `slots`, so the send practically never blocks.
- A reader held back by slow workers waits inside `Pool::take`. `timing.read` counts that as read time (`pipeline.rs:285-307`).
- The worker's `ctl.cpu_permit()` wait (`pipeline.rs:347`) sits between `work_wait` (the `recv`) and `work` (which starts after it). It is counted nowhere.

So three new meters are added, all lock-free `Clock`s:

- `pool_order_wait`, the wait for the item's turn in the in-order grants, and `pool_capacity_wait`, the wait for budget or slots. Both are split inside `Pool::take` (`pipeline.rs`, about lines 805–850).
- `permit_wait`, around `cpu_permit()`.

`oldest_read_ns` is the age of the oldest read in flight, from a per-reader start timestamp. It lets the latency guard see a stall before the read completes. A `Clock` only adds after the fact, so a 5 s stall would otherwise land wholly in one window. Windows are compared by median, and a window is discarded when `paused` is non-zero (the sampler diffs `ctl.paused_total()`, `control.rs:307`). A pause is then not read as a throughput collapse.

**The algorithm is a candidate, chosen before the data.** A hill-climb with a latency guard is robust. A model-based estimator, which fits service time against reads in flight and picks the knee directly using Little's law, may do better. The two measurements disagree (IO.md against the raw probe). The meters phase therefore comes first. The algorithm is confirmed or replaced against the simulator and the measurements before it is wired in. Only the interface, `Sample` in and `Decision` out, is fixed now. The candidate thresholds are named constants in `TunerConfig`, and they are provisional:

1. Start at the stored profile value. Otherwise start from a prior by drive kind: rotational 2, solid 4, unknown 3. Bounds are 1 to 16.
2. The initial climb doubles the count while the gain stays at least 12%, which is above the Exos ±10% run noise. It then refines by ±1. Compare 3-window medians against a decayed table of throughput per count.
3. Latency guard: if mean latency is above 2× the low-count baseline for under 5% throughput gain, back off one step. This is the Exos signature.
4. Back off and hold down (30 s, doubling to 5 min) when throughput falls below 0.88× the best at the next lower count.
5. While stable, probe one step every ~20 windows (down when above the minimum, otherwise up), judged by the same 12% rule over 3 windows. This re-adapts when the cache cools or warms. Skip probes while noisy or decode-bound.
6. Noise: hold and report `Noisy` when the coefficient of variation over 5 windows exceeds 0.35. Discard windows that are short, have too few reads, or span a count change. Act only when 2 of 3 windows agree. Make at most one change per 2 windows.

### Pipeline: `crates/jobs/src/pipeline.rs`

- **A seam for the read**, so that the whole loop is testable without hardware. The pipeline opens and reads files directly (`read_whole`, `read_gather`, `pipeline.rs:628-700`, through `pg_platform::file::open_read`). No test can give it a disk with a chosen service-time curve, so the gate, sampler, tuner and pool could only be tested in pieces.
  - This is not a one-line seam. `Read::Gather.gather` is `fn(&mut std::fs::File)` (`pipeline.rs:114`), and a simulated disk cannot hand out a real `File` that sleeps.
  - The seam must own the read: `Options.source: Arc<dyn FileSource>`, with `read_whole(path, &mut Vec<u8>)` and `read_gather(path, hint, gather)`.
  - `gather` becomes `fn(&mut dyn ReadSeek)`, a `Read + Seek` trait object. That changes `pg_container::jpeg_import_bytes` and `png_import_bytes`, and the two `Read::Gather` call sites at `crates/import/src/lib.rs:146-149`.
  - The default `OsFiles` does what the code does now, including `pg_platform::file::open_read` and the size query `sized(file)`. The added cost is one virtual call per file.
  - A `SimDisk` in `pg_jobs_testkit` sleeps for a service time that depends on how many reads are in flight. It uses curves from the Exos and SSD measurements, with a mid-run "cache warm" step, in scaled-down time. End-to-end tests then check convergence and re-adaptation through the real pipeline, deterministically within bounds. These tests are in addition to the pure-controller simulator.
- **Gate.** Spawn `min(max, items.len())` readers, so a pass of three files does not start sixteen threads. Gate them with `ReaderGate { target: AtomicUsize, lock, cv, stopped }`. `gate.enter(id, stop)` goes **before** `next.fetch_add` (`pipeline.rs:283`). The pool grants strictly in item order, so a reader must never park after claiming an index. A reader with `id >= target` parks on the condvar with a 100 ms timeout, so that it re-checks `ctl` and `stop`.
- **Release conditions.** A hang here blocks the scope join. The gate opens on `halt` and `StopOnDrop` (`pipeline.rs:706-719`) and on `ctl.wake`. It also releases when the items run out: a parked reader checks `next >= items.len()` on each timeout, and a reader that finds `items.get(i) == None` (`pipeline.rs:284`) closes the gate. Each of these needs its own test.
- **Meters.** Make `Timing` shareable as `Arc<Meters>`. Add `reads` and `disk_ns`. `disk_ns` is timed around the actual `read_to_end` (about `pipeline.rs:637, 662, 700`), separately from the pool wait that `timing.read` includes today. Add a time-weighted active-readers integral. `Clock` stays lock-free (`Relaxed`).
- **Sampler.** A dedicated thread inside the scope wakes every window, takes deltas, calls `tuner.update`, and stores the target in the gate. It stops on `halt`. It cannot run in the sink loop, which blocks in `results.recv()` (`pipeline.rs:383`).
- **`readers_busy_pct`** (`pipeline.rs:179`) divides by `opts.readers`. It must use the time-weighted active count instead.
- **`Options`.** `readers: Readers`, where `enum Readers { Fixed(NonZeroUsize), Auto(Tuning) }` and `Tuning { max, prior, pass_kind, profile }`. `Options::new` stays `Fixed(READERS)`. `Fixed` remains for crop jobs (`coordinator.rs:652`), for tests and for the developer pin. Slots and `buffer_bytes` derive from `max`.
- **`Pool`** is otherwise unchanged, apart from the two new wait meters inside `take`. Its in-order grants and its `BUDGET_WAIT` loop stay.
- **Gate order and lifetime.** The reader loop is: gate, then `ctl.checkpoint_until`, then `fetch_add`. A gate placed after the checkpoint would let a reader released after a pause claim one more item. `halt` (`pipeline.rs:706`, which takes `stop`, `pool` and `ctl`) also notifies the gate. The sampler waits on a condvar that `halt` and `StopOnDrop` notify, not on a plain 3 s sleep, so no `thread::scope` join is delayed. The active-readers count is an RAII guard, because the reader loop has `break` paths at `:289, :293, :310` that would leak a manual decrement. A reader that has claimed an index never parks, and leases drain through the workers, so the in-order grants cannot deadlock when the target shrinks.

### Wiring: the app's lanes only

- **The drive binding.** `Inner` gains its drive binding (`DriveKey`, `Kind`). It is set beside `set_label` (`coordinator.rs:430-434`). That code runs on every tick under the registry lock, so the binding is compared before it is changed. The tuner is re-seeded only when it differs, because a kept lane's drive can change.
- **The tuner belongs to the lane, not to a pass.** A lane runs many short passes. A tuner made at each pass start would repeat its warm-up, cooldown, hold-down and probe timers every time, and it would never converge. `Inner` holds a `LaneTuner`: one `Tuner` per **pass kind** (Import, Thumbs, Placeholders, Hashes, Borders), each with its learned table. It survives across passes and is re-seeded when the drive binding changes.
- **The key is the pass kind**, set by the pass in `Tuning`. It is not a class derived from the items. Import mixes `Gather` for JPEG and PNG, `File` for other formats and `Streamed` for video, item by item (`crates/import/src/lib.rs:140-151`). No per-pass or per-window read class exists, and a class derived per window would flip the tuner's state mid-pass.
- **Read-bound windows only.** A table is updated only from read-bound windows. A decode-bound pass such as thumbnails learns a flat throughput-per-count curve. A probe down on that curve would drag the count to the minimum, and the next read-bound pass would inherit it.
- **Short passes.** A pass too short to fill a window (16 reads or 3 s) only seeds and carries the count. It teaches nothing.
- **Where it reads.** `Inner::options` (`inner.rs:63-66`) hands each pass a handle to its lane's tuner in `Readers::Auto`. The pass's sampler feeds the tuner.
- **Videos** are read by the worker itself (`Read::Streamed`). No reader is involved, so those items do not feed the sampler.
- **A lane is a connected group of physical disks**, so tuning per lane is tuning per disk. With the Windows storage rule in Part B, every lane's disks are known. A spanned volume is tuned as one unit.
- **`CpuGate` is unchanged.** Decode contention shows up as `permit_wait` and `pool_capacity_wait`, which yields a decode-bound hold.
- **The developer pin.** `RunnerConfig.pin_readers` (Part A) makes the lane use `Readers::Fixed(n)`, and a pinned lane writes no profile.

### Memory governance (closes TYPE_MISMATCH item 3)

The reader count, the files in flight and the buffer budget are one coupled quantity. Tuning only the reader count would leave memory wrong.

1. **No `max_lent` bound.** This was dropped after review: two mechanisms were doing one job. The reader gate already bounds the reads in flight, and the static budget already bounds the bytes held. A `max_lent = target + 2 × workers` bound would only trim the buffered files by `max − target` buffers, at most 15 × 12 MiB. `Pool.slots` is also the capacity of the `raw` channel (`pipeline.rs:267`), which any such bound would have to stay under. If measurements show that waiting buffers matter, the bound is added then, with that constraint.
2. **The lane's budget stays static.** If the budget followed the tuner's target, the governor's per-lane share would change with every decision. The budget feeds pool admission, which shapes the pool waits the tuner reads. That is a feedback loop, and one lane's target would move the other lanes' samples. So the governor splits the run's total among the lanes on the static maximum, as now. The seed is `Options::budget(READERS, cpus)` at `coordinator.rs:179`, recomputed for the new maximum. A lower target lowers memory only through fewer reads in flight.
3. **The run's soft cap comes from available memory.** It is no longer only a fixed quarter of physical memory:
   - `cap = min(physical / MEMORY_SHARE, held + available − headroom)`, where `held` is what the lanes' pools hold now.
   - The formula subtracts `held` back in. Without it, `available − headroom` would fall as the lanes fill and recover as they shrink, which is a ratchet.
   - The governor cannot hold a changing cap as built. `Governor.knee` and `cap` are plain fields set in `new` (`governor.rs:95-105`), and `rebalance()` is called only at `coordinator.rs:555`, when a lane is retired or registered.
   - So `cap` becomes an atomic with `set_cap`. The coordinator's lane loop calls `set_cap` and `rebalance` on a timer, every few seconds, with hysteresis so that small changes do nothing.
   - Available memory comes from `pg_platform::memory::available_bytes()`. On Windows this is `GlobalMemoryStatusEx`'s `ullAvailPhys`; on Linux it is `MemAvailable`. Other systems return `None`, as `physical_bytes` does at `memory.rs:10, 25, 41`. The coordinator reads only total physical memory today (`coordinator.rs:178`).
   - The cap stays soft. A file larger than the budget is read alone.

**Open from TYPE_MISMATCH item 3.** Its third problem is not addressed here. `read_gather` admits only the hint, then calls `grow_to(bytes.len())`, and `read_the_rest` grows to the full size with no budget check (`pipeline.rs:667, 697`). A read can therefore exceed the budget. This stays open, and it needs its own decision.

### Persistence: `drive-profiles.json`

`Prefs` is user-visible and flat, and it has a field-order golden test. This is machine-learned data that can be deleted at any time. So it is a sibling file, not a `Prefs` field. It reuses `JsonStore`, which is atomic, quarantines a corrupt file and falls back to defaults.

- **Location.** The file sits in the layout's home folder beside the preferences (`Layout::home`, `crates/library/src/layout.rs:61`). It is machine-level, so it survives a catalog rebuild. `pg-dev` runs with an isolated home by default (Part A), so a sweep or a test never writes the real profile.
- **Entries.** One per (disk, pass kind): kind, the stable reader count and the range tested, MB/s, sample count, timestamp and OS build. The map is a `BTreeMap`, because the workspace bans `HashMap`. The stored type implements `Default`, which `JsonStore<T>` needs.
- **Layering.** `JsonStore` lives in `crates/app/src/store.rs`. `library`, where `Inner` and the tuner live, cannot depend on `app` (`tools/crate-layers.txt`). So the profile map and a `ProfileStore` trait (`load`, `save`) are defined in `pg_library`, with the value type in `pg_model`. `pg_app` implements the trait over `JsonStore`, and passes it in through `RunnerConfig`, as `Drives` is injected today.
- **Key.** A fixed 64-bit FNV-1a hash of `vendor|product|serial`. `DefaultHasher` is not stable across Rust versions, and `platform` has no hash crate. A lane over several disks keys on the hash of its sorted disk keys, so its entry is its own.
- **Stable disk key probe.** A new probe in `crates/platform/src/drive.rs`, through the `disk_fact` seam:
  - Windows: `STORAGE_DEVICE_DESCRIPTOR`'s vendor, product and serial.
  - Linux: `/sys/block/<d>/device/{model,serial,wwid}`.
  - `DriveKey` today is a set of disk numbers or a `dev_t`, which is not stable across reboots.
  - A disk without a serial (some USB bridges) gets no profile, only the kind prior.
- **Invalidation.** Distrust an entry when it is older than 30 days, when the kind differs, when samples are too few, or when the first window's throughput is more than 30% below the stored value. A stored value only seeds the start. The controller still verifies it.
- **One write rule.** A lane writes its learned entries when it goes idle or is retired. Only the pass kinds whose table has at least 60 s or 200 reads of read-bound windows, and whose last decision was stable, are written.

### Visibility

- The `pg_perf` pipeline record gains `readers_chosen`, `readers_min`, `readers_max` and `readers_avg` (time-weighted), `tuner_reason`, `tuner_changes`, `tuner_probes` and `disk_ms_avg`. `gallery perf` can then say "read-bound at N readers". The command itself is `pg-dev perf` after Part A.
- One log line per change, for the dev-mode log view. Library crates call no `pg_log` today, so a small hook is wired from the app rather than adding a dependency edge. Check `tools/crate-layers.txt` before adding one.
- An optional `readers` detail on `LaneProgress`. No notice for normal adaptation.
- `Headless` results include the decisions.

## Phases

Phase numbers are global. Phases C5 to C7 are independent of the CLI removal. C8 to C10 wait for B3, and C9 also needs A1's `RunnerConfig`.

- **C5 Meters:** shared `Meters`, the per-read disk timer, the active-readers integral, the two pool-wait meters and `permit_wait`, `oldest_read_ns`, and a corrected `readers_busy_pct`. Additive only. Run the manual measurements that set the thresholds before any threshold is locked.
- **C6 Gate and read seam:** `ReaderGate` with tests, still `Fixed`, and the `FileSource` seam with `OsFiles` as the default and the `ReadSeek` change to the `Gather` callers. Behaviour is unchanged. The seam is needed by the simulated disk in phase C7.
- **C7 Tuner:** the pure state machine, its simulator and its property tests. Not yet wired in.
- **C8 Memory governance:** `available_bytes()`, the atomic cap with `set_cap`, the coordinator's timed `rebalance`, and the soft cap `min(physical / 4, held + available − headroom)`. Tests: the governor's pure split (extend the `governor.rs` tests), a test that the cap does not ratchet as lanes fill, and the headroom measured on real use.
- **C9 Wire auto:** `Readers::Auto` in `Inner::options`, the lane-owned `LaneTuner`, the drive binding, and the kind prior only. Also the visibility fields. Needs B3's typed `Options`.
- **C10 Persistence:** the disk-key probe, `drive-profiles.json`, the `ProfileStore` trait and invalidation.

## Verification

- **Commands:** `cargo test -p pg-jobs --locked`, plus the per-crate tests for each phase, then `just check`, `just lint`, `just modgraph`, `cargo doc -D warnings` and `cargo machete`. Clippy denies `unwrap`, indexing and `as` casts, requires `#[expect]` rather than `#[allow]`, and bans `HashMap`. Qt-linking tests run with the Qt bin on the PATH (`C:/Qt/6.12.0/msvc2022_64`).
- **Tuner (pure unit tests, with `t0 + Duration`, no sleeps):** grow on a clear gain; no growth within noise; latency backoff; hold-down; decode-bound hold; discarded windows; cooldown; probe keep and revert.
- **Tuner (property tests,** with `proptest` added to `pg-jobs` dev-dependencies, in the style of `crates/client/tests/controller_laws.rs`): the target stays within 1 to `max`; a strictly improving stream never lowers it; the same stream always gives the same decisions; a stationary stream changes at most K times in W windows.
- **Simulated disk:** a service-time curve per kind, from the Exos data and an SSD-like saturating curve, plus a mid-stream warm-cache step. The check is convergence within one step, and re-adaptation after the step.
- **Real threads** (in `pipeline.rs` `mod tests`, with `within_timeout`, `pg_testkit::run_folder` and a sleeping `process`): the target changes mid-run and peak concurrency follows it; stop and pause with readers parked both complete the pass; the release on exhausted items does not hang the scope join.
- **Governance:** the cap does not ratchet as lanes fill and empty.
- **Manual validation** (the IO.md and IMPORT_READS_PLAN protocol: app closed, cold cache, three rounds in rotating order, medians and ranges, `PG_PERF_LOG`). Compare auto against fixed counts (developer pins 1, 2, 4 and 8) on Exos, WDC, Toshiba, a SATA SSD and an NVMe drive, using real photo folders. Still missing: the SSD baseline, and a cold-cache run with decoding included. Those cold runs need the elevated standby-cache purge, so **ask the user first**.

## Risks and unverified

- IO.md (depth helps on cold HDDs) conflicts with the raw whole-file probe. The thresholds are locked only after the phase C5 measurements.
- Windows' disk counters are localized, and their busy percentage saturates. The controller therefore uses only the pipeline's own timings.
- Shrinking the target cannot cancel reads in flight, only later claims. The reaction is one file late.
- The controller may chase antivirus or search-indexer load. The noise hold and the decayed tables limit that, and the controller responds to latency whatever the cause.
- Whether `Readers` as an enum interacts badly with B3's typed `readers` field is unknown until B3 is done.
- The headroom value for the memory cap is unmeasured.
- The `FileSource` seam's cost on the Windows open path is unmeasured. It should change only who calls the open. Phase C6 is the place to confirm that.

## Critical files

- `crates/jobs/src/pipeline.rs`, `crates/jobs/src/tuner.rs` (new): the gate, meters, sampler and `Options`.
- `crates/jobs/src/governor.rs`, `crates/platform/src/memory.rs`: the available memory and the soft cap.
- `crates/library/src/coordinator.rs`, `inner.rs`, `lane.rs`, `passes.rs`: the drive binding, the lane tuner and the timed rebalance.
- `crates/platform/src/drive.rs`: the stable disk key.
- `crates/app/src/store.rs`, `prefs.rs`: the `ProfileStore` implementation.
- `crates/import/src/lib.rs` and `crates/container`: the `FileSource` seam's callers.

## Existing code to reuse

`Budget` (`governor.rs`) as the model for a lane-owned live atomic. `CpuGate` as the model for a gate. `JsonStore` (`crates/app/src/store.rs`) for the profile file. `disk_fact` (`drive.rs`) for the disk probe. `Running::mark(now)` for time-argument tests. `pg_testkit::run_folder_with` and `pg_jobs_testkit` for scratch libraries and timeouts. `pg_perf::record` for the performance fields.
