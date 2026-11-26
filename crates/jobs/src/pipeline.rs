//! The shape every pass over the originals shares: files read in the given (on-disk) order with a few
//! reads in flight, processed on a CPU pool, results handed to one writer on the calling thread.
//!
//! Reads go to the disk in list order from `readers` threads at once. ≥ 8 reads in flight let
//! the drive reorder them and reach sequential speed even in path order, and sparse reads only speed up with
//! queue depth.
//!
//! What bounds the files in flight (read, waiting, or being processed) is their count (`Options::slots`), never their
//! bytes: the app is for heavy users, and a pass holds whatever its files in flight take (no memory cap, by decision).
//! Every file is read into a buffer from a pool of at most that many buffers, and handed back once processed. Readers
//! wait while every slot is taken, so they stay a bounded number of files ahead of the workers, and the buffers are
//! reused from file to file: a pass makes about `slots` of them, whatever its length.
//!
//! The readers are started up to their most (`Readers::max`, no more than the pass has items), and a gate lets a number
//! of them claim items (`crate::gate`): all of them for a fixed count, and for a tuned one (`Readers::Auto`) the
//! target a sampler thread sets each window from the pass's meters, through the lane's tuner (`crate::tuner`). The files
//! are opened and read through `Options::source` (`crate::source`), the system's files by default.

use std::io::{Seek as _, SeekFrom};
use std::num::NonZeroUsize;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

// crossbeam-channel rather than std::sync::mpsc: the readers and workers share one receiver each (`raw_rx.clone()`), and
// std's `Receiver` cannot be cloned. Every one-to-one handoff elsewhere in the workspace uses std::sync::mpsc.
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded, unbounded};
use pg_platform::native::NativePath;
use pg_platform::priority::{self, Priority};
use serde_json::json;

use crate::control::Control;
use crate::failure::{ChangedWhileRead, Class, Failed};
use crate::gate::ReaderGate;
use crate::panics;
use crate::source::{Access, FileSource, OsFiles, ReadSeek};
use crate::tuner::{Decision, Reason, Sample, Tuner};

/// How many threads read a pass's files.
#[derive(Clone)]
pub enum Readers {
    /// This many, from start to end: the crop's jobs, the tests, and a developer's pinned run.
    Fixed(NonZeroUsize),
    /// As many as the lane's tuner finds best on its drive, up to `Tuning::max`.
    Auto(Tuning),
}

impl Readers {
    /// The most readers the pass starts: what its slots are sized by.
    pub fn max(&self) -> NonZeroUsize {
        match self {
            Readers::Fixed(n) => *n,
            Readers::Auto(tuning) => tuning.max,
        }
    }
}

/// What a tuned pass needs: its bound, its lane's tuner (one per drive and pass kind, kept across passes), and who is
/// told when the target changes.
#[derive(Clone)]
pub struct Tuning {
    /// The most readers.
    pub max: NonZeroUsize,
    /// The lane's tuner for this pass kind.
    pub tuner: Arc<Mutex<Tuner>>,
    /// Told each change of the target, with the decision (the lane's progress, and the dev-mode log).
    pub on_change: Option<Arc<dyn Fn(Decision) + Send + Sync>>,
}

/// How a pipeline runs: how many threads read and how many process, and how much is in flight at once. Every count is
/// typed so that no caller can build a pool with none of them: zero can't be written, and the slots have a maximum.
#[derive(Clone)]
pub struct Options {
    /// The threads that read the files: a fixed count, or a tuned one.
    pub readers: Readers,
    /// The threads that process the files.
    pub workers: NonZeroUsize,
    /// The files in flight at most: being read, read and waiting for a worker, or being processed. A file a worker
    /// opens itself (a video) counts only while it waits. [`Options::slots`] by default.
    pub slots: Slots,
    /// Thread priority of the readers and workers (a `Priority`). Below normal keeps the app's own threads
    /// (the UI, thumbnails on screen) ahead of a background pass.
    pub priority: Priority,
    /// Where the files are opened and read: the system's files ([`OsFiles`]), or a test's source.
    pub source: Arc<dyn FileSource>,
}

/// The reads in flight of a pipeline (`Options::new`), and the default the other pools are sized by.
pub const READERS: NonZeroUsize = NonZeroUsize::MIN.saturating_add(7);

/// The most files a pass holds in flight ([`Slots`]). The pool makes room for that many buffers up front
/// (`Vec::with_capacity`), so a larger count would be a large allocation for nothing: the default for 8 readers and
/// 128 workers is 264.
pub const MAX_SLOTS: usize = 1 << 16;

/// The most workers the default options take, whatever the CPU count: their default slots (`READERS + 2 × workers`)
/// then stay below [`MAX_SLOTS`].
const MAX_DEFAULT_WORKERS: NonZeroUsize = NonZeroUsize::MIN.saturating_add(4095);

/// The logical CPUs the pools are sized by: `available_parallelism`, or 4 where the platform cannot say.
pub fn cpus() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get())
}

/// The workers of the default options: a worker per logical CPU (4 where the platform cannot say), at most 4096.
pub fn default_workers() -> NonZeroUsize {
    std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN.saturating_add(3)).min(MAX_DEFAULT_WORKERS)
}

/// Why reader and worker counts make no [`Options`] ([`Options::sized`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeRefused {
    /// The files in flight they make (`readers + 2 × workers`) are more than [`MAX_SLOTS`].
    TooManySlots,
}

impl std::fmt::Display for SizeRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SizeRefused::TooManySlots => write!(f, "more than {MAX_SLOTS} files in flight"),
        }
    }
}

impl std::error::Error for SizeRefused {}

/// A count of files in flight: at least one, and at most [`MAX_SLOTS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Slots(NonZeroUsize);

impl Slots {
    /// `n` slots, or `None` for zero or more than [`MAX_SLOTS`].
    pub fn new(n: usize) -> Option<Slots> {
        if n > MAX_SLOTS {
            return None;
        }
        NonZeroUsize::new(n).map(Slots)
    }

    /// The count.
    pub fn get(self) -> usize {
        self.0.get()
    }
}

impl Options {
    /// [`READERS`] reads in flight, a worker per logical CPU (at most 4096), and their [`Options::slots`].
    #[expect(clippy::expect_used, reason = "8 readers and at most 4096 workers make 8200 slots, under MAX_SLOTS")]
    pub fn new(priority: Priority) -> Options {
        Options::sized(Readers::Fixed(READERS), default_workers(), priority).expect("the default counts fit")
    }

    /// `readers` and `workers`, with the [`Options::slots`] that follow from them (from the readers' most, for a tuned
    /// count), reading the system's files, or why they are refused: more slots than [`MAX_SLOTS`]. Nothing is clamped.
    pub fn sized(readers: Readers, workers: NonZeroUsize, priority: Priority) -> Result<Options, SizeRefused> {
        let most = readers.max();
        Ok(Options {
            readers,
            workers,
            slots: Options::slots(most, workers).ok_or(SizeRefused::TooManySlots)?,
            priority,
            source: Arc::new(OsFiles),
        })
    }

    /// The default count of files in flight: one per read, and two per worker (the one it processes and the next,
    /// so that it doesn't wait for the disk). As fast as no bound by count (24 MP JPEGs on the HDD,
    /// 8 readers and 12 workers: 273 MB/s with 32 slots, or 56, or unbounded; 271 with 20). `None` past [`MAX_SLOTS`].
    pub fn slots(readers: NonZeroUsize, workers: NonZeroUsize) -> Option<Slots> {
        Slots::new(workers.get().checked_mul(2)?.checked_add(readers.get())?)
    }
}

/// How an item is read, as `read` says.
#[derive(Clone, Copy, Debug)]
pub enum Read {
    /// Whole, from the path `read` wrote into the reader's buffer ([`NativePath::set`]): `process` gets its bytes.
    File,
    /// Not by the pipeline: `process` gets no bytes and opens the file itself (a video, which is streamed, not
    /// loaded). `bytes` is what it counts as read (the file's size, or 0 for a part of it).
    Streamed {
        /// The bytes the file counts as read in the pass's progress.
        bytes: u64,
    },
    /// What `gather` makes of the file, read by it (it seeks where it needs to, and returns the bytes `process` gets: a
    /// JPEG's header, a PNG's chunks without its image data). `hint` is how many bytes it is expected to need: the room
    /// its buffer is given before `gather` runs. A file no longer than `hint` is read whole, and so is one `gather` gives
    /// nothing for (`None`, or a panic). An error of `gather` is the file's read error.
    Gather {
        /// How many bytes `gather` is expected to need.
        hint: usize,
        /// Reads the bytes `process` gets from the file, seeking where it needs to: through the source's reader
        /// ([`ReadSeek`]), so any source serves a gather as it serves a whole read.
        gather: fn(&mut dyn ReadSeek) -> std::io::Result<Option<Vec<u8>>>,
    },
}

/// What a worker sends back: an item's result with the bytes it counts as read, or the error of a setup that failed
/// (or panicked), after which the worker stops.
enum Outcome<R> {
    Done { index: usize, result: Result<R, Failed>, read: usize },
    SetupFailed(Failed),
}

/// Where a pass's threads spend their time: nanoseconds per stage, summed over the threads of the stage, and what the
/// readers read. Every meter is lock-free (`Relaxed`): a statistic, which no other memory is ordered by. The sampler of a
/// tuned pass reads their differences each window; the perf record reads their totals.
#[derive(Default)]
struct Timing {
    /// Readers: reading a file, including its turn in the pool.
    read: Clock,
    /// Readers: handing a read file on, which waits while the workers are behind.
    read_send: Clock,
    /// Readers: the time they were let through by the gate, summed: over the wall time, the active readers on average.
    active: Clock,
    /// Readers: the time at the disk, around the opens and the reads only (no wait for the pool).
    disk: Clock,
    /// Readers: the files read at the disk (not a streamed one), and their bytes.
    reads: Counter,
    bytes: Counter,
    /// Workers: waiting for a read file.
    work_wait: Clock,
    /// Workers: waiting for a CPU permit, when the run's lanes share the cores.
    permit_wait: Clock,
    /// Workers: processing a file.
    work: Clock,
    /// Workers: handing a result on, which waits while the sink is behind.
    work_send: Clock,
    /// The sink: waiting for a result.
    sink_wait: Clock,
    /// The sink: its own work per result (for [`batched`], counting and handing batches over).
    sink: Clock,
    /// The writer of [`batched`]: waiting for a batch.
    write_wait: Clock,
    /// The writer of [`batched`]: flushing a batch.
    write: Clock,
    /// What the tuner did over the pass, for a tuned one.
    tuned: Mutex<Option<Tuned>>,
}

/// A tuned pass's readers over its run, for the perf record.
#[derive(Clone, Copy, Debug)]
struct Tuned {
    /// The target at the end, and the lowest and highest it had.
    chosen: usize,
    min: usize,
    max: usize,
    /// The last decision's reason.
    reason: Reason,
    /// The changes of target and the probes in this pass.
    changes: u64,
    probes: u64,
}

#[derive(Default)]
struct Clock(AtomicU64);

impl Clock {
    /// Adds the time since `since`.
    fn add(&self, since: Instant) {
        let nanos = u64::try_from(since.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.0.fetch_add(nanos, Ordering::Relaxed);
    }

    fn nanos(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    fn secs(&self) -> f64 {
        Duration::from_nanos(self.nanos()).as_secs_f64()
    }
}

/// A count a meter adds to.
#[derive(Default)]
struct Counter(AtomicU64);

impl Counter {
    fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Adds a reader's time through the gate to [`Timing::active`] when dropped: a guard, so each `break` of the reader's
/// loop counts its time too.
struct Active<'a> {
    clock: &'a Clock,
    since: Instant,
}

impl<'a> Active<'a> {
    fn new(clock: &'a Clock) -> Active<'a> {
        Active { clock, since: Instant::now() }
    }
}

impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.clock.add(self.since);
    }
}

/// When each reader's read in flight began, in nanoseconds since the pass's start plus one (0: none), so the sampler sees
/// the age of the oldest read before it completes: a stall is then not one window's late surprise.
struct Flight {
    epoch: Instant,
    starts: Vec<AtomicU64>,
}

impl Flight {
    fn new(readers: usize) -> Flight {
        Flight { epoch: Instant::now(), starts: (0..readers).map(|_| AtomicU64::new(0)).collect() }
    }

    fn since_epoch(&self, at: Instant) -> u64 {
        u64::try_from(at.saturating_duration_since(self.epoch).as_nanos()).unwrap_or(u64::MAX)
    }

    /// Reader `id` began a read now.
    fn begin(&self, id: usize) {
        if let Some(start) = self.starts.get(id) {
            start.store(self.since_epoch(Instant::now()).saturating_add(1), Ordering::Relaxed);
        }
    }

    /// Reader `id` has no read in flight.
    fn end(&self, id: usize) {
        if let Some(start) = self.starts.get(id) {
            start.store(0, Ordering::Relaxed);
        }
    }

    /// The age of the oldest read in flight at `now` (0: none).
    fn oldest(&self, now: Instant) -> u64 {
        let now = self.since_epoch(now).saturating_add(1);
        self.starts
            .iter()
            .map(|start| start.load(Ordering::Relaxed))
            .filter(|&start| start > 0)
            .map(|start| now.saturating_sub(start))
            .max()
            .unwrap_or(0)
    }
}

impl Timing {
    /// Records the pass's stage times in the perf log, as a `pipeline` record under the span the pass runs in (inert
    /// without a sink): each stage's busy and waiting shares of the pass's wall time, and the time the user paused it.
    /// The readers' shares are of their time through the gate (their time-weighted active count), and a tuned pass adds
    /// what its tuner chose. A pass with a writer also reports the writer's stage.
    fn report(&self, items: usize, wall: Duration, paused: Duration, opts: &Options, writer: bool) {
        if !pg_perf::enabled() {
            return;
        }
        let secs = wall.as_secs_f64().max(1e-9);
        let count = |n: usize| f64::from(u32::try_from(n).unwrap_or(u32::MAX));
        let pct = |c: &Clock, threads: usize| 100.0 * c.secs() / (count(threads.max(1)) * secs);
        let active = self.active.secs().max(1e-9);
        let of_active = |c: &Clock| 100.0 * c.secs() / active;
        let reads = self.reads.get();
        let disk_ms = if reads == 0 { 0.0 } else { self.disk.secs() * 1e3 / pg_model::num::size(reads) };
        let mut fields = vec![
            ("items", json!(items)),
            ("readers", json!(opts.readers.max().get())),
            ("readers_avg", json!(self.active.secs() / secs)),
            ("workers", json!(opts.workers.get())),
            ("readers_busy_pct", json!(of_active(&self.read))),
            ("readers_handing_pct", json!(of_active(&self.read_send))),
            ("disk_ms_avg", json!(disk_ms)),
            ("workers_permit_wait_pct", json!(pct(&self.permit_wait, opts.workers.get()))),
            ("workers_busy_pct", json!(pct(&self.work, opts.workers.get()))),
            ("workers_wait_reads_pct", json!(pct(&self.work_wait, opts.workers.get()))),
            ("workers_handing_pct", json!(pct(&self.work_send, opts.workers.get()))),
            ("sink_busy_pct", json!(pct(&self.sink, 1))),
            ("sink_wait_pct", json!(pct(&self.sink_wait, 1))),
            ("paused_ms", json!(paused.as_secs_f64() * 1e3)),
        ];
        if writer {
            fields.push(("writer_busy_pct", json!(pct(&self.write, 1))));
            fields.push(("writer_wait_pct", json!(pct(&self.write_wait, 1))));
        }
        let tuned = *self.tuned.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(t) = tuned {
            fields.push(("readers_chosen", json!(t.chosen)));
            fields.push(("readers_min", json!(t.min)));
            fields.push(("readers_max", json!(t.max)));
            fields.push(("tuner_reason", json!(t.reason.name())));
            fields.push(("tuner_changes", json!(t.changes)));
            fields.push(("tuner_probes", json!(t.probes)));
        }
        pg_perf::record("pipeline", wall, &fields);
    }

    /// The meters' totals now, for a window's differences.
    fn snapshot(&self, pool: &Pool, ctl: &Control) -> Totals {
        Totals {
            at: Instant::now(),
            paused: ctl.paused_total(),
            reads: self.reads.get(),
            bytes: self.bytes.get(),
            disk: self.disk.nanos(),
            active: self.active.nanos(),
            work_wait: self.work_wait.nanos(),
            permit_wait: self.permit_wait.nanos(),
            work: self.work.nanos(),
            order_wait: pool.order_wait.nanos(),
            capacity_wait: pool.capacity_wait.nanos(),
        }
    }
}

/// The meters' totals at one moment: a window is the difference of two.
#[derive(Clone, Copy)]
struct Totals {
    at: Instant,
    paused: Duration,
    reads: u64,
    bytes: u64,
    disk: u64,
    active: u64,
    work_wait: u64,
    permit_wait: u64,
    work: u64,
    order_wait: u64,
    capacity_wait: u64,
}

impl Totals {
    /// The window from `self` to `later`, as the tuner takes it.
    fn window(&self, later: &Totals, oldest_read_ns: u64, workers: usize) -> Sample {
        let elapsed = later.at.saturating_duration_since(self.at);
        let active = later.active.saturating_sub(self.active);
        let active_readers =
            if elapsed.is_zero() { 0.0 } else { pg_model::num::size(active) / (elapsed.as_secs_f64() * 1e9) };
        let reads = later.reads.saturating_sub(self.reads);
        Sample {
            elapsed,
            paused: later.paused.saturating_sub(self.paused),
            reads,
            disk_ns: later.disk.saturating_sub(self.disk),
            bytes: later.bytes.saturating_sub(self.bytes),
            oldest_read_ns,
            work_wait_ns: later.work_wait.saturating_sub(self.work_wait),
            permit_wait_ns: later.permit_wait.saturating_sub(self.permit_wait),
            work_busy_ns: later.work.saturating_sub(self.work),
            pool_order_wait_ns: later.order_wait.saturating_sub(self.order_wait),
            pool_capacity_wait_ns: later.capacity_wait.saturating_sub(self.capacity_wait),
            active_readers,
            workers,
            files: reads,
        }
    }
}

/// The end of a pass, which its sampler waits on: set by [`Ender::halt`], so no window outlasts the pass.
#[derive(Default)]
struct Ending {
    done: Mutex<bool>,
    changed: Condvar,
}

impl Ending {
    fn end(&self) {
        *self.done.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.changed.notify_all();
    }

    /// Waits `window`, or until the pass ends. Whether it ended.
    fn wait(&self, window: Duration) -> bool {
        let done = self.done.lock().unwrap_or_else(PoisonError::into_inner);
        let (done, _) =
            self.changed.wait_timeout_while(done, window, |done| !*done).unwrap_or_else(PoisonError::into_inner);
        *done
    }
}

/// The sampler of a tuned pass: every window it takes the meters' differences, asks the lane's tuner, and moves the
/// gate to its target (never past the readers the pass started). It ends with the pass.
#[expect(clippy::too_many_arguments, reason = "the pass's parts the sampler reads: each is its own")]
fn sample(
    tuning: &Tuning,
    timing: &Timing,
    pool: &Pool,
    flight: &Flight,
    gate: &ReaderGate,
    ending: &Ending,
    ctl: &Control,
    workers: usize,
    started: usize,
) {
    let (window, changes_before, probes_before) = {
        let tuner = tuning.tuner.lock().unwrap_or_else(PoisonError::into_inner);
        (tuner.config().window, tuner.changes(), tuner.probes())
    };
    let mut last = timing.snapshot(pool, ctl);
    loop {
        if ending.wait(window) {
            return;
        }
        let now = timing.snapshot(pool, ctl);
        let s = last.window(&now, flight.oldest(now.at), workers);
        let (decision, changes, probes) = {
            let mut tuner = tuning.tuner.lock().unwrap_or_else(PoisonError::into_inner);
            let decision = tuner.update(&s);
            (decision, tuner.changes(), tuner.probes())
        };
        let target = decision.target.get().min(started);
        if target != gate.target() {
            gate.set_target(target);
            if let Some(told) = &tuning.on_change {
                told(decision);
            }
        }
        let mut tuned = timing.tuned.lock().unwrap_or_else(PoisonError::into_inner);
        let seen = tuned.get_or_insert(Tuned {
            chosen: target,
            min: target,
            max: target,
            reason: decision.reason,
            changes: 0,
            probes: 0,
        });
        seen.chosen = target;
        seen.min = seen.min.min(target);
        seen.max = seen.max.max(target);
        seen.reason = decision.reason;
        seen.changes = changes.saturating_sub(changes_before);
        seen.probes = probes.saturating_sub(probes_before);
        drop(tuned);
        last = now;
    }
}

/// What a per-file pass runs over and under: its items, its options and its control. [`run`] and [`batched`] take one.
pub struct Pass<'a, T> {
    /// What the pass runs over: item `i` is read, processed and sunk by its index.
    pub items: &'a [T],
    /// The pass's readers, workers, buffers and priority.
    pub opts: &'a Options,
    /// The pass's pause, stop and progress.
    pub ctl: &'a Control,
}

/// Read each item's file, run `process(state, i, bytes)` (with per-worker state from `init`), and call
/// `sink(i, result, bytes read)` in completion order. `sink` returning false ends the pass at once. A stop request on
/// `ctl` ends the reading, and the pass ends once the results of the items in flight are sunk: the items not yet started
/// are dropped unrun, and reading waits while `ctl` is paused.
///
/// `read(item, wide)` says how the item is read ([`Read`]); for a file, it writes the path into `wide`, the reader's
/// buffer, reused from file to file, so no path is made per file.
///
/// A panic in `read` or `process` is that item's failure ([`Class::Panic`], through [`panics::guard`]), and the pass goes
/// on: a worker whose `process` panicked makes its state again with `init`. A panic in `init`, or an error from it,
/// fails the pass, as does a pass that ends with items sent to the workers and no result for them. An error of `init`,
/// `process` or `sink` converts into a [`Failed`] where it is returned.
pub fn run<T, S, R, I, P>(
    pass: Pass<'_, T>,
    read: impl Fn(&T, &mut NativePath) -> Read + Sync,
    init: impl Fn() -> Result<S, I> + Sync,
    process: impl Fn(&mut S, usize, &[u8]) -> Result<R, P> + Sync,
    sink: impl FnMut(usize, Result<R, Failed>, usize) -> Result<bool, Failed>,
) -> Result<(), Failed>
where
    T: Sync,
    R: Send,
    I: Into<Failed>,
    P: Into<Failed>,
{
    let Pass { items, opts, ctl } = pass;
    let timing = Timing::default();
    let started = Instant::now();
    let paused_before = ctl.paused_total();
    let result = run_timed(Pass { items, opts, ctl }, read, init, process, sink, &timing);
    timing.report(items.len(), started.elapsed(), ctl.paused_total().saturating_sub(paused_before), opts, false);
    result
}

/// [`run`], keeping each stage's time in `timing`.
fn run_timed<T, S, R, I, P>(
    pass: Pass<'_, T>,
    read: impl Fn(&T, &mut NativePath) -> Read + Sync,
    init: impl Fn() -> Result<S, I> + Sync,
    process: impl Fn(&mut S, usize, &[u8]) -> Result<R, P> + Sync,
    mut sink: impl FnMut(usize, Result<R, Failed>, usize) -> Result<bool, Failed>,
    timing: &Timing,
) -> Result<(), Failed>
where
    T: Sync,
    R: Send,
    I: Into<Failed>,
    P: Into<Failed>,
{
    let Pass { items, opts, ctl } = pass;
    let next = AtomicUsize::new(0);
    // The items the readers handed to the workers, to check that each came back with a result. The counters and the
    // stop flag here are SeqCst: each is set by one thread and read by another (or after the scope joins), so the
    // file uses one ordering for them; only the statistic (`Clock`) and the index claim are Relaxed.
    let sent = AtomicUsize::new(0);
    // The items a stopped pass dropped unrun: each was sent, and has no result.
    let dropped = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let pool = Pool::new(opts.slots);
    // A place per slot (an array, made once): a read file waits for the workers, never for the channel.
    let (raw_tx, raw_rx) = bounded::<(usize, Result<Lease, Failed>, usize)>(pool.slots);
    let (res_tx, res_rx) = bounded::<Outcome<R>>(opts.workers.get().saturating_mul(4));
    // The readers are started up to their most, and no more than the items: a pass of three files starts three. The
    // gate lets the target through: all of them for a fixed count, the tuner's for a tuned one.
    let started = opts.readers.max().get().min(items.len());
    let first = match &opts.readers {
        Readers::Fixed(n) => n.get(),
        Readers::Auto(tuning) => tuning.tuner.lock().unwrap_or_else(PoisonError::into_inner).target().get(),
    };
    let gate = ReaderGate::new(first.min(started.max(1)));
    let flight = Flight::new(started);
    let ending = Ending::default();
    let source = opts.source.as_ref();

    std::thread::scope(|s| {
        // However the pass ends (done, stopped, an error from `sink`), readers waiting on the pool, at the gate or paused
        // at a checkpoint stop waiting, and the sampler ends.
        let ender = Ender { stop: &stop, pool: &pool, ctl, gate: &gate, ending: &ending };
        let _stop = StopOnDrop(ender);
        if let Readers::Auto(tuning) = &opts.readers {
            let (timing, pool, flight, gate, ending) = (timing, &pool, &flight, &gate, &ending);
            let workers = opts.workers.get();
            s.spawn(move || sample(tuning, timing, pool, flight, gate, ending, ctl, workers, started));
        }
        for id in 0..started {
            let raw_tx = raw_tx.clone();
            let (next, sent, stop, read, pool, timing) = (&next, &sent, &stop, &read, &pool, timing);
            let (gate, flight) = (&gate, &flight);
            s.spawn(move || {
                priority::set_current_thread(opts.priority);
                let mut wide = NativePath::default();
                // The gate comes before the checkpoint and the claim: a reader never parks holding a claimed index, which
                // the pool's in-order grants would wait for; and one released after a pause claims no item it may not.
                let done = || {
                    stop.load(Ordering::SeqCst) || ctl.stop_requested() || next.load(Ordering::Relaxed) >= items.len()
                };
                while gate.enter(id, done) && ctl.checkpoint_until(stop) {
                    let _active = Active::new(&timing.active);
                    // Relaxed: the index only makes each item go to one reader; the item's data reaches the reader
                    // through `items`, and the hand-off to the workers goes through the channel.
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else {
                        // The items ran out: the readers parked at the gate need not wait for a target.
                        gate.close();
                        break;
                    };
                    let started = Instant::now();
                    flight.begin(id);
                    let reader = Reader { pool, source, timing, i };
                    let outcome = match panics::catch(AssertUnwindSafe(|| read(item, &mut wide))) {
                        Ok(Read::File) => reader.whole(&wide).map(|bytes| (bytes.map_err(read_error), 0)),
                        Ok(Read::Gather { hint, gather }) => {
                            reader.gather(&wide, hint, gather).map(|bytes| (bytes.map_err(read_error), 0))
                        }
                        Ok(Read::Streamed { bytes }) => {
                            pool.pass(i);
                            Some((Ok(Lease::empty()), usize::try_from(bytes).unwrap_or(usize::MAX)))
                        }
                        Err(payload) => {
                            // The item's turn in the pool is passed all the same, or the items after it would wait
                            // for it forever; the path it left half-written is not used again.
                            pool.pass(i);
                            wide = NativePath::default();
                            Some((Err(Failed::panic(panics::message(&*payload).to_string())), 0))
                        }
                    };
                    flight.end(id);
                    let Some((bytes, streamed)) = outcome else { break }; // stopped
                    timing.read.add(started);
                    let handing = Instant::now();
                    if raw_tx.send((i, bytes, streamed)).is_err() {
                        break;
                    }
                    timing.read_send.add(handing);
                    sent.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
        drop(raw_tx);

        for _ in 0..opts.workers.get() {
            let (raw_rx, res_tx) = (raw_rx.clone(), res_tx.clone());
            let (init, process, timing, stop, dropped) = (&init, &process, timing, &stop, &dropped);
            s.spawn(move || {
                priority::set_current_thread(opts.priority);
                // A worker whose setup failed (or panicked) sends its error, and stops.
                let setup = || match panics::guard(AssertUnwindSafe(init)) {
                    Ok(st) => Some(st),
                    Err(e) => {
                        let _ = res_tx.send(Outcome::SetupFailed(e));
                        None
                    }
                };
                let Some(mut state) = setup() else { return };
                loop {
                    let waiting = Instant::now();
                    let Ok((i, read, streamed)) = raw_rx.recv() else { break };
                    timing.work_wait.add(waiting);
                    // A stopped pass runs nothing it has not started: the item's buffer goes back to the pool (`read` drops
                    // here), and the item has no result.
                    if stop.load(Ordering::SeqCst) {
                        dropped.fetch_add(1, Ordering::SeqCst);
                        continue;
                    }
                    let (out, len, rebuild) = match read {
                        Ok(lease) => {
                            let len = lease.bytes.len().saturating_add(streamed);
                            // A CPU permit for the processing, when the run's lanes share the cores (not for the reading).
                            // Its wait is the workers' own meter: another lane holding the cores is a decode-bound pass.
                            let asking = Instant::now();
                            let permit = ctl.cpu_permit();
                            timing.permit_wait.add(asking);
                            let working = Instant::now();
                            let out = panics::guard(AssertUnwindSafe(|| process(&mut state, i, &lease.bytes)));
                            timing.work.add(working);
                            drop(permit);
                            // The buffer goes back to the pool before the result is sent.
                            drop(lease);
                            // A panic may have left the state half-changed: it is made again.
                            let rebuild = out.as_ref().is_err_and(|failed| failed.is_panic());
                            (out, len, rebuild)
                        }
                        Err(e) => (Err(e), 0, false),
                    };
                    let handing = Instant::now();
                    if res_tx.send(Outcome::Done { index: i, result: out, read: len }).is_err() {
                        break;
                    }
                    timing.work_send.add(handing);
                    if rebuild {
                        match setup() {
                            Some(st) => state = st,
                            None => return,
                        }
                    }
                }
            });
        }
        drop((raw_rx, res_tx));

        // Leaving this closure drops `res_rx`, which unblocks and ends the workers, then the readers.
        // The receiver is this closure's: an early return drops it, which ends the workers' waits before the scope joins them.
        let results = res_rx;
        let mut received = 0usize;
        let mut halted = false;
        loop {
            let waiting = Instant::now();
            let Ok(outcome) = results.recv() else { break };
            timing.sink_wait.add(waiting);
            let (i, out, len) = match outcome {
                Outcome::Done { index, result, read } => (index, result, read),
                Outcome::SetupFailed(error) => return Err(error.context("worker setup")),
            };
            received += 1;
            let working = Instant::now();
            let go_on = sink(i, out, len)?;
            timing.sink.add(working);
            if !go_on {
                return Ok(());
            }
            // A stop ends the reading, and the results still due are sunk: the items in flight finish, their results
            // included, and the queued ones are dropped. The loop goes on until the workers have ended.
            if !halted && ctl.stop_requested() {
                halted = true;
                ender.halt();
            }
        }
        // Every worker has ended, and every reader before them: what was sent is final, and each item has a result or was
        // dropped by a stopped pass.
        let sent = sent.load(Ordering::SeqCst);
        let dropped = dropped.load(Ordering::SeqCst);
        if received + dropped < sent {
            return Err(Failed::other(format!(
                "the workers ended with {} of the {sent} items read and no result for them",
                sent - received - dropped
            )));
        }
        Ok(())
    })
}

/// How a batched pass writes what it made: `size` results at a time, and with `every`, also once that long has passed
/// since the last write (so what a slow pass finds is not held back for long).
#[derive(Clone, Copy, Debug)]
pub struct Batch {
    /// The most items a batch holds.
    pub size: usize,
    /// The longest a batch waits before it is written, if the pass sets a wait.
    pub every: Option<Duration>,
}

/// A batched pass's per-item error (section 10, D2): the file's own failure, with its class ([`Failed`]: the pipeline's
/// read or panic, or one the pass made of the file, `Data` or `Read`), or the pass's own error (`E`), which the pass's
/// flush returns to halt the pass. Nothing per file is of class `Other` (D4).
#[derive(Debug)]
pub enum ItemError<E> {
    /// The file's failure: recorded or tried again by its class.
    File(Failed),
    /// The pass's error, met on this item: the flush halts the pass with it.
    Pass(E),
}

/// Why a batched pass ended before its items did (section 10, D1): the pipeline's own failure (a panic in the writer, a
/// lost item, a worker's setup that panicked), or the pass's own error (its flush's, or its worker setup's).
#[derive(Debug)]
pub enum Halt<E> {
    /// The pipeline failed.
    Pipeline(Failed),
    /// The pass's own error.
    Pass(E),
}

impl<E> Halt<E> {
    /// The pass's error, a pipeline failure made one by `From`.
    pub fn into_pass(self) -> E
    where
        E: From<Failed>,
    {
        match self {
            Halt::Pipeline(failed) => E::from(failed),
            Halt::Pass(e) => e,
        }
    }
}

impl<E: std::fmt::Display> std::fmt::Display for Halt<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Halt::Pipeline(failed) => write!(f, "{failed}"),
            Halt::Pass(e) => write!(f, "{e}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for Halt<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Halt::Pipeline(failed) => failed.source(),
            Halt::Pass(e) => e.source(),
        }
    }
}

/// One item's result, as a batched pass's flush gets it: the item itself (no index to look up, D2), and what it came to.
pub type Done<'a, T, R, E> = (&'a T, Result<R, ItemError<E>>);

/// [`run`] for a pass that writes its results in batches, the skeleton the per-file passes share. Each result is counted
/// on `ctl` (its bytes, and whether it failed) and kept, with its item. A batch that is due ([`Batch`]) goes to a writer
/// thread, which makes it with `flush` on the writer's state `writer`, while the counting goes on: the counter never waits
/// for a write. The writer takes a batch only when it is waiting for one; while it is busy the kept results keep growing
/// (commits coalesce), and the counting waits only when twice the batch size is kept. So at most three batches' worth
/// of results are held: the writer's, the one kept, and the one at its limit.
///
/// A stop request on `ctl` ends the pass as [`run`] does: the results of the items in flight are counted and kept, and
/// then written.
///
/// The errors are typed (section 10, D1–D3): `process` gives each item's result with an [`ItemError`], `init` and
/// `flush` the pass's own error `E`. `flush` returns false to stop the pass: the writer then discards what is queued,
/// and the pass ends after the results counted are written. An error of `flush` ([`Halt::Pass`]) or a panic in it
/// ([`Halt::Pipeline`]) ends the pass, which loses what is queued and kept; so does an error of `init` (the first one,
/// [`Halt::Pass`]) or of the pipeline. The writer's state comes back in any case, with the pass's result: a caller that
/// has more to write (a ledger) writes it from the state.
pub fn batched<'a, T, S, R, W, E>(
    pass: Pass<'a, T>,
    read: impl Fn(&T, &mut NativePath) -> Read + Sync,
    init: impl Fn() -> Result<S, E> + Sync,
    process: impl Fn(&mut S, &T, &[u8]) -> Result<R, ItemError<E>> + Sync,
    batch: Batch,
    writer: W,
    flush: impl FnMut(&mut W, &mut Vec<Done<'a, T, R, E>>) -> Result<bool, E> + Send,
) -> (W, Result<(), Halt<E>>)
where
    T: Sync,
    R: Send,
    W: Send,
    E: Send,
{
    let Pass { items, opts, ctl } = pass;
    let size = batch.size.max(1);
    // The first error of a worker's setup, typed: the pipeline sees a failure, and the pass returns this.
    let setup_error: Mutex<Option<E>> = Mutex::new(None);
    let setup_error = &setup_error;
    let init = move || {
        init().map_err(|e| {
            setup_error.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(e);
            Failed::other("the pass's worker couldn't be set up".to_string())
        })
    };
    // Each item's own result: never a failure of the pipeline's own (`Ok` around the item's result), but for an index the
    // list doesn't hold, which no reader hands out.
    let process = |state: &mut S, i: usize, bytes: &[u8]| -> Result<Result<R, ItemError<E>>, Failed> {
        let item = items.get(i).ok_or_else(|| Failed::other(format!("no item {i} in the pass")))?;
        Ok(process(state, item, bytes))
    };
    // A rendezvous: the sink hands a batch over only to a writer that is waiting for one. The vectors the writer is done
    // with come back on the other channel, to be kept again.
    let (to_writer, from_sink) = bounded::<Vec<Done<'a, T, R, E>>>(0);
    let (recycled_tx, recycled_rx) = unbounded::<Vec<Done<'a, T, R, E>>>();
    // Set when the writer failed or stopped: the pass ends, and nothing more is handed over.
    let failed = AtomicBool::new(false);
    let failed = &failed;
    let timing = Timing::default();
    let timing = &timing;
    let started = Instant::now();
    let paused_before = ctl.paused_total();

    let out = std::thread::scope(|s| {
        let writing = s.spawn(move || {
            let mut writer = writer;
            let mut flush = flush;
            let mut outcome: Result<(), Halt<E>> = Ok(());
            let mut stopped = false;
            loop {
                let waiting = Instant::now();
                let Ok(mut kept) = from_sink.recv() else { break };
                timing.write_wait.add(waiting);
                if !stopped {
                    let flushing = Instant::now();
                    match panics::catch(AssertUnwindSafe(|| flush(&mut writer, &mut kept))) {
                        Ok(Ok(true)) => {}
                        Ok(Ok(false)) => stopped = true,
                        Ok(Err(e)) => {
                            outcome = Err(Halt::Pass(e));
                            stopped = true;
                        }
                        Err(payload) => {
                            outcome = Err(Halt::Pipeline(Failed::panic(panics::message(&*payload).to_string())));
                            stopped = true;
                        }
                    }
                    timing.write.add(flushing);
                    if stopped {
                        failed.store(true, Ordering::SeqCst);
                    }
                }
                kept.clear();
                // Nothing waits for the vector once the sink has ended.
                let _ = recycled_tx.send(kept);
            }
            (writer, outcome)
        });

        let mut kept: Vec<Done<'a, T, R, E>> = Vec::with_capacity(size);
        let mut since = Instant::now();
        let ran = run_timed(
            Pass { items, opts, ctl },
            read,
            init,
            process,
            |i, result, len| -> Result<bool, Failed> {
                let item = items.get(i).ok_or_else(|| Failed::other(format!("a result for no item {i}")))?;
                // The pipeline's own failure of the item (its read, a panic) is the file's.
                let result = result.unwrap_or_else(|failed| Err(ItemError::File(failed)));
                ctl.add(len, result.is_err());
                kept.push((item, result));
                if failed.load(Ordering::SeqCst) {
                    return Ok(false);
                }
                let every = batch.every.is_some_and(|every| since.elapsed() >= every);
                if kept.len() >= size || every {
                    let wait = kept.len() >= 2 * size;
                    if !hand_over(&to_writer, &recycled_rx, &mut kept, size, wait) {
                        failed.store(true, Ordering::SeqCst);
                        return Ok(false);
                    }
                    since = Instant::now();
                }
                Ok(!failed.load(Ordering::SeqCst))
            },
            timing,
        );
        // The results counted and not yet handed over are the last batch: written, unless the pass failed.
        if ran.is_ok() && !failed.load(Ordering::SeqCst) && !kept.is_empty() {
            let _ = hand_over(&to_writer, &recycled_rx, &mut kept, size, true);
        }
        drop(to_writer);
        let (writer, outcome) = match writing.join() {
            Ok(done) => done,
            Err(panic) => std::panic::resume_unwind(panic),
        };
        // A worker's setup that failed with the pass's error is that error; the writer's own comes next.
        let setup = setup_error.lock().unwrap_or_else(PoisonError::into_inner).take();
        let result = match (ran, outcome, setup) {
            (Ok(()), Ok(()), _) => Ok(()),
            (Err(_), _, Some(e)) => Err(Halt::Pass(e)),
            (Ok(()), Err(halt), _) | (Err(_), Err(halt @ Halt::Pass(_)), None) => Err(halt),
            (Err(e), Ok(()), None) => Err(Halt::Pipeline(e)),
            (Err(e), Err(Halt::Pipeline(w)), None) => {
                Err(Halt::Pipeline(e.context(&format!("and the writer ended with: {w}"))))
            }
        };
        (writer, result)
    });
    timing.report(items.len(), started.elapsed(), ctl.paused_total().saturating_sub(paused_before), opts, true);
    out
}

/// Hands the kept results to the writer, and keeps the vector the writer returns (or a new one) in their place. A
/// batch the writer is not waiting for is kept, to grow, unless `wait` (then the sink waits for the writer). Whether the
/// writer is still there to take it.
fn hand_over<X>(
    to_writer: &Sender<Vec<X>>,
    recycled: &Receiver<Vec<X>>,
    kept: &mut Vec<X>,
    size: usize,
    wait: bool,
) -> bool {
    /// What became of the batch.
    enum Handed {
        /// The writer has it.
        Taken,
        /// The writer is busy: the batch is kept, to grow.
        Busy,
        /// The writer has ended.
        Gone,
    }
    let batch = std::mem::take(kept);
    let handed = if wait {
        match to_writer.send(batch) {
            Ok(()) => Handed::Taken,
            Err(_) => Handed::Gone,
        }
    } else {
        match to_writer.try_send(batch) {
            Ok(()) => Handed::Taken,
            Err(TrySendError::Full(batch)) => {
                *kept = batch;
                Handed::Busy
            }
            Err(TrySendError::Disconnected(_)) => Handed::Gone,
        }
    };
    match handed {
        Handed::Taken => {
            *kept = recycled.try_recv().unwrap_or_else(|_| Vec::with_capacity(size));
            true
        }
        Handed::Busy => true,
        Handed::Gone => false,
    }
}

/// The error of an item that couldn't be read: a [`Class::Read`] whose text is `read`, then the I/O error.
fn read_error(e: std::io::Error) -> Failed {
    // A whole read whose length changed is its own failure, so a pass can tell it apart (`Failed::is_changed`).
    if e.get_ref().is_some_and(|inner| inner.is::<ChangedWhileRead>()) {
        if let Some(Ok(changed)) = e.into_inner().map(|inner| inner.downcast::<ChangedWhileRead>()) {
            return Failed::new(Class::Read, *changed);
        }
        return Failed::other("the file changed while it was read".to_string());
    }
    Failed::new(Class::Read, ReadFailure(e))
}

/// A failed read of an item, as the error of its failure: its text is `read`, and the I/O error is its source.
#[derive(Debug)]
struct ReadFailure(std::io::Error);

impl std::fmt::Display for ReadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("read")
    }
}

impl std::error::Error for ReadFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// One item's read on a reader's thread: the pool its buffer comes from, the source its file is opened through, the
/// meters it adds to, and the item's index (its turn in the pool).
struct Reader<'p> {
    pool: &'p Pool,
    source: &'p dyn FileSource,
    timing: &'p Timing,
    i: usize,
}

impl<'p> Reader<'p> {
    /// The file at `path`, opened for `access`, with its open timed as disk time. An error passes the item's turn.
    fn open(&self, path: &NativePath, access: Access) -> std::io::Result<crate::source::Opened> {
        let opening = Instant::now();
        let opened = self.source.open(path, access);
        self.timing.disk.add(opening);
        if opened.is_err() {
            self.pool.pass(self.i);
        }
        opened
    }

    /// `file`'s bytes from where it is to its end, into `lease`: timed as disk time, and counted as a read.
    fn read_rest(&self, file: &mut dyn ReadSeek, lease: &mut Lease<'_>) -> std::io::Result<()> {
        let reading = Instant::now();
        let before = lease.bytes.len();
        let read = file.read_to_end(&mut lease.bytes);
        self.timing.disk.add(reading);
        self.note(lease.bytes.len().saturating_sub(before));
        read.map(drop)
    }

    /// A read of `bytes` done: one more read for the meters.
    fn note(&self, bytes: usize) {
        self.timing.reads.add(1);
        self.timing.bytes.add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }

    /// Read the file at `path` into a buffer from the pool, its size reserved in list order. `None` once the pass stops.
    /// Bytes not as many as the file's length when it was opened are an error ([`ChangedWhileRead`], section 7): the file
    /// changed while it was read, so nothing is made of them.
    fn whole(&self, path: &NativePath) -> Option<std::io::Result<Lease<'p>>> {
        let mut opened = match self.open(path, Access::Sequential) {
            Ok(opened) => opened,
            Err(e) => return Some(Err(e)),
        };
        let mut lease = self.pool.take(self.i, opened.size)?;
        if let Err(e) = self.read_rest(opened.file.as_mut(), &mut lease) {
            return Some(Err(e));
        }
        if lease.bytes.len() != opened.size {
            let as_u64 = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
            let changed = ChangedWhileRead { opened: as_u64(opened.size), read: as_u64(lease.bytes.len()) };
            return Some(Err(std::io::Error::other(changed)));
        }
        Some(Ok(lease))
    }

    /// [`Read::Gather`] of the file at `path`: what `gather` makes of it, read from the reader's thread in its turn. Its
    /// buffer is taken in the item's turn, with room for the hint, before the read, so the reads in flight stay in list
    /// order. A file no longer than `hint` is read whole. Whatever `gather` doesn't give (`None`, or a panic) is the whole
    /// file, read from its start. The file is opened for reads here and there: the system reads no further ahead than
    /// each read asks (measured on an NVMe disk, its read-ahead after each end of a PNG cost a fifth of the rate). `None`
    /// once the pass stops.
    fn gather(
        &self,
        path: &NativePath,
        hint: usize,
        gather: fn(&mut dyn ReadSeek) -> std::io::Result<Option<Vec<u8>>>,
    ) -> Option<std::io::Result<Lease<'p>>> {
        let opened = match self.open(path, Access::Random) {
            Ok(opened) => opened,
            Err(e) => return Some(Err(e)),
        };
        let (mut file, size) = (opened.file, opened.size);
        if size <= hint {
            let mut lease = self.pool.take(self.i, size)?;
            return Some(self.read_rest(file.as_mut(), &mut lease).map(|()| lease));
        }
        let mut lease = self.pool.take(self.i, hint)?;
        let gathering = Instant::now();
        let gathered = panics::catch(AssertUnwindSafe(|| gather(file.as_mut())));
        self.timing.disk.add(gathering);
        match gathered {
            Ok(Ok(Some(bytes))) => {
                self.note(bytes.len());
                if let Err(e) = lease.grow_to(bytes.len()) {
                    return Some(Err(e));
                }
                lease.bytes.extend_from_slice(&bytes);
                return Some(Ok(lease));
            }
            Ok(Err(e)) => return Some(Err(e)),
            Ok(Ok(None)) | Err(_) => {}
        }
        // Not gathered: the whole file, from its start (the gather may have moved the file's position).
        if let Err(e) = file.seek(SeekFrom::Start(0)) {
            return Some(Err(e));
        }
        lease.bytes.clear();
        if let Err(e) = lease.grow_to(size) {
            return Some(Err(e));
        }
        Some(self.read_rest(file.as_mut(), &mut lease).map(|()| lease))
    }
}

/// What ends a pass: its stop flag, its pool, its control, its reader gate and the end its sampler waits on.
#[derive(Clone, Copy)]
struct Ender<'a> {
    stop: &'a AtomicBool,
    pool: &'a Pool,
    ctl: &'a Control,
    gate: &'a ReaderGate,
    ending: &'a Ending,
}

impl Ender<'_> {
    /// Stops the reading of a pass: the readers stop at their next checkpoint, a reader waiting for the pool or parked
    /// at the gate stops waiting, and the readers paused at a checkpoint (which look at the stop under the pause lock,
    /// so the wake can't come between their check and their wait). The sampler ends. The workers then drop what they
    /// have not started.
    fn halt(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.pool.stop();
        self.gate.close();
        self.ending.end();
        self.ctl.wake();
    }
}

/// Ends the pass when the scope's closure returns (done, stopped, an error from `sink`, or a sink that said stop).
struct StopOnDrop<'a>(Ender<'a>);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        self.0.halt();
    }
}

/// File buffers within a count (`slots`), and no byte bound: a pass holds whatever its files in flight take. Grants are
/// in list order (item `i` after every item before it), so the disk is read in order and a large file is never passed
/// over by the smaller ones behind it.
struct Pool {
    slots: usize,
    state: Mutex<PoolState>,
    changed: Condvar,
    /// The readers' wait for their item's turn in the in-order grants.
    order_wait: Clock,
    /// The readers' wait, in their turn, for a slot: the workers are behind.
    capacity_wait: Clock,
}

struct PoolState {
    /// The next item to be granted.
    serving: usize,
    /// The buffers lent out.
    lent: usize,
    /// Never more than `slots` (lent and idle together), so made once with room for that many.
    idle: Vec<Vec<u8>>,
    stopped: bool,
}

/// A buffer lent out by the pool, given back when dropped.
struct Lease<'a> {
    pool: Option<&'a Pool>,
    bytes: Vec<u8>,
}

impl Lease<'_> {
    /// No buffer (a file not read whole).
    fn empty() -> Self {
        Lease { pool: None, bytes: Vec::new() }
    }

    /// Makes room for `size` bytes in the buffer (a gather's bytes, or the whole file). An allocation that fails is the
    /// item's error, not a panic.
    fn grow_to(&mut self, size: usize) -> std::io::Result<()> {
        let additional = size.saturating_sub(self.bytes.len());
        self.bytes.try_reserve_exact(additional).map_err(|e| std::io::Error::new(std::io::ErrorKind::OutOfMemory, e))
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Some(pool) = self.pool {
            pool.give(std::mem::take(&mut self.bytes));
        }
    }
}

impl Pool {
    /// A pool of `slots` buffers. The slots are at most [`MAX_SLOTS`], so the room made up front is bounded.
    fn new(slots: Slots) -> Pool {
        let slots = slots.get();
        let state = PoolState { serving: 0, lent: 0, idle: Vec::with_capacity(slots), stopped: false };
        Pool {
            slots,
            state: Mutex::new(state),
            changed: Condvar::new(),
            order_wait: Clock::default(),
            capacity_wait: Clock::default(),
        }
    }

    /// A buffer with room for `size` bytes for item `i`, once the items before it have theirs and a slot is free. `None`
    /// once the pass stops.
    ///
    /// The smallest idle buffer that is large enough is reused. If none is, the smallest idle one grows to `size` (the
    /// larger ones stay for the files that need them), and a new buffer is made only when none is idle. So there are
    /// never more buffers than slots, and a pass allocates once per slot, plus once per file larger than the buffer it
    /// is given.
    fn take(&self, i: usize, size: usize) -> Option<Lease<'_>> {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if st.stopped {
                return None;
            }
            if st.serving == i && st.lent < self.slots {
                // Largest first, so the smallest is last.
                st.idle.sort_unstable_by_key(|b| std::cmp::Reverse(b.capacity()));
                let bytes = match st.idle.iter().rposition(|b| b.capacity() >= size) {
                    Some(k) => st.idle.remove(k),
                    None => {
                        let mut bytes = st.idle.pop().unwrap_or_default();
                        bytes.clear();
                        bytes.reserve_exact(size);
                        bytes
                    }
                };
                return Some(self.grant(&mut st, bytes));
            }
            // Its turn not come is the order's wait; its turn come, and no slot free, the capacity's.
            let meter = if st.serving == i { &self.capacity_wait } else { &self.order_wait };
            let waiting = Instant::now();
            st = self.changed.wait(st).unwrap_or_else(PoisonError::into_inner);
            meter.add(waiting);
        }
    }

    fn grant(&self, st: &mut PoolState, mut bytes: Vec<u8>) -> Lease<'_> {
        bytes.clear();
        st.serving += 1;
        st.lent += 1;
        self.changed.notify_all();
        Lease { pool: Some(self), bytes }
    }

    /// Item `i` takes no buffer (not read whole, or not readable): the next one is served once it is its turn.
    fn pass(&self, i: usize) {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        while st.serving != i && !st.stopped {
            let waiting = Instant::now();
            st = self.changed.wait(st).unwrap_or_else(PoisonError::into_inner);
            self.order_wait.add(waiting);
        }
        if st.serving == i {
            st.serving += 1;
            self.changed.notify_all();
        }
    }

    /// A buffer back from a worker, kept for the next file.
    fn give(&self, bytes: Vec<u8>) {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        st.lent -= 1;
        st.idle.push(bytes);
        self.changed.notify_all();
    }

    fn stop(&self) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).stopped = true;
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use pg_platform::native::{NativePath, Unit};
    use std::fs::File;
    use std::io::Read as _;

    use super::*;

    /// `s` as the system takes it: UTF-16 units on Windows, bytes elsewhere.
    #[cfg(windows)]
    fn units(s: &str) -> Vec<Unit> {
        s.encode_utf16().collect()
    }

    #[cfg(unix)]
    fn units(s: &str) -> Vec<Unit> {
        s.as_bytes().to_vec()
    }

    /// Options of `readers`, `workers` and `slots`, at normal priority.
    fn sized(readers: usize, workers: usize, slots: usize) -> Options {
        Options {
            readers: Readers::Fixed(NonZeroUsize::new(readers).unwrap()),
            workers: NonZeroUsize::new(workers).unwrap(),
            slots: Slots::new(slots).unwrap(),
            priority: Priority::Normal,
            source: Arc::new(OsFiles),
        }
    }

    /// A pool of `slots` buffers.
    fn pool(slots: usize) -> Pool {
        Pool::new(Slots::new(slots).unwrap())
    }

    // A count of slots is at least one and at most the maximum; the defaults' arithmetic refuses what overflows.
    #[test]
    fn slots_out_of_range_are_refused() {
        assert_eq!(Slots::new(0), None);
        assert_eq!(Slots::new(MAX_SLOTS + 1), None);
        assert_eq!(Slots::new(MAX_SLOTS).map(Slots::get), Some(MAX_SLOTS));
        let n = |n: usize| NonZeroUsize::new(n).unwrap();
        assert_eq!(Options::slots(n(8), n(12)).map(Slots::get), Some(32));
        assert_eq!(Options::slots(n(8), n(MAX_SLOTS)), None, "past the maximum");
        assert_eq!(Options::slots(n(1), n(usize::MAX)), None, "2 × workers overflows");
        let refused = Options::sized(Readers::Fixed(n(8)), n(MAX_SLOTS), Priority::Normal).err();
        assert_eq!(refused, Some(SizeRefused::TooManySlots));
        let default = Options::new(Priority::Normal);
        assert_eq!(default.readers.max(), READERS);
        assert_eq!(Some(default.slots), Options::slots(default.readers.max(), default.workers));
    }

    /// What the pool holds: the leases lent, and no more buffers than slots.
    fn check(pool: &Pool, lent: &[&Lease]) {
        let st = pool.state.lock().unwrap();
        assert_eq!(st.lent, lent.len());
        assert!(st.idle.len() + st.lent <= pool.slots);
    }

    // Grants come in list order: an item waits for every item before it, whatever its size, and a large file is
    // granted as a small one is (no byte bound).
    #[test]
    fn pool_grants_in_list_order() {
        let pool = pool(4);
        let a = pool.take(0, 60).unwrap();
        std::thread::scope(|s| {
            let waiter = s.spawn(|| pool.take(2, 50).map(|l| l.bytes.capacity()));
            std::thread::sleep(std::time::Duration::from_millis(50));
            assert!(!waiter.is_finished(), "item 2 waits for item 1's turn");
            pool.pass(1);
            assert!(waiter.join().unwrap().unwrap() >= 50);
        });
        let big = pool.take(3, 50 << 20).unwrap();
        assert!(big.bytes.capacity() >= 50 << 20, "a large file is granted beside the others");
        check(&pool, &[&a, &big]);
    }

    // Files of growing and shrinking sizes read one after another: one buffer, grown when a file needs more.
    #[test]
    fn a_pool_reuses_its_buffers_whatever_the_sizes() {
        let pool = pool(8);
        for (i, size) in [100, 300, 200, 1000, 50, 4000, 4000, 10].into_iter().enumerate() {
            let lease = pool.take(i, size).unwrap();
            assert!(lease.bytes.capacity() >= size);
            check(&pool, &[&lease]);
            drop(lease);
            assert_eq!(pool.state.lock().unwrap().idle.len(), 1, "item {i}");
        }
        // Two in flight: two buffers. The larger is kept for a large file, the smaller grown for the next one.
        let a = pool.take(8, 5000).unwrap();
        let b = pool.take(9, 20).unwrap();
        check(&pool, &[&a, &b]);
        drop((a, b));
        let c = pool.take(10, 6000).unwrap();
        let d = pool.take(11, 5000).unwrap();
        check(&pool, &[&c, &d]);
        assert_eq!(pool.state.lock().unwrap().idle.len(), 0);
    }

    // However far ahead the readers are, there are never more buffers than slots: a reader waits for a slot.
    #[test]
    fn a_pool_lends_no_more_buffers_than_slots() {
        let pool = pool(3);
        let leases: Vec<Lease> = (0..3).map(|i| pool.take(i, 16 << 10).unwrap()).collect();
        check(&pool, &leases.iter().collect::<Vec<_>>());
        std::thread::scope(|s| {
            let waiter = s.spawn(|| pool.take(3, 16 << 10).map(|l| l.bytes.capacity()));
            std::thread::sleep(std::time::Duration::from_millis(50));
            assert!(!waiter.is_finished(), "a fourth buffer while three are lent");
            drop(leases);
            assert!(waiter.join().unwrap().unwrap() >= 16 << 10);
        });
        check(&pool, &[]);
        assert_eq!(pool.state.lock().unwrap().idle.len(), 3);
    }

    #[test]
    fn a_stopped_pool_wakes_its_waiters() {
        let pool = pool(1);
        let held = pool.take(0, 10).unwrap();
        std::thread::scope(|s| {
            // One waits for its turn, one for a slot.
            let turn = s.spawn(|| pool.take(3, 10).is_none());
            let slot = s.spawn(|| pool.take(1, 10).is_none());
            std::thread::sleep(std::time::Duration::from_millis(50));
            pool.stop();
            assert!(turn.join().unwrap());
            assert!(slot.join().unwrap());
        });
        drop(held);
    }

    // A whole pass of small files with one slow worker, the readers far ahead of it: every file is read whole and
    // processed once, through slots and a channel far smaller than the pass.
    #[test]
    fn a_pass_reads_every_file_within_its_slots() {
        let dir = pg_testkit::run_folder("scratch_jobs", "pipeline").unwrap();
        let paths: Vec<String> = (0..200)
            .map(|i| {
                let path = dir.join(format!("{i}.bin"));
                std::fs::write(&path, vec![u8::try_from(i).unwrap(); 100 + i * 7]).unwrap();
                path.to_str().unwrap().to_string()
            })
            .collect();
        let opts = sized(4, 1, 6);
        let mut seen = vec![false; paths.len()];
        run(
            Pass { items: &paths, opts: &opts, ctl: &Control::new() },
            |p, wide| {
                wide.set(&[&units(p)]);
                Read::File
            },
            || Ok::<(), Failed>(()),
            |_, i, bytes| {
                std::thread::sleep(std::time::Duration::from_micros(200));
                assert_eq!(bytes.len(), 100 + i * 7);
                assert!(bytes.iter().all(|&b| usize::from(b) == i));
                Ok::<(), Failed>(())
            },
            |i, out, _| {
                out.unwrap();
                assert!(!std::mem::replace(&mut seen[i], true));
                Ok(true)
            },
        )
        .unwrap();
        assert!(seen.iter().all(|&s| s));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Runs `f` on a thread of its own, and fails if it takes longer than a pass of a few files ever should: a
    /// regression that hangs the pass fails the test instead of hanging it.
    fn within_timeout<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(std::time::Duration::from_secs(60)).expect("the pass hung (or panicked)")
    }

    /// `count` small files in a new folder, with their indices.
    fn numbered_files(name: &str, count: usize) -> (std::path::PathBuf, Vec<(usize, String)>) {
        let dir = pg_testkit::run_folder("scratch_jobs", name).unwrap();
        let items = (0..count)
            .map(|i| {
                let path = dir.join(format!("{i}.bin"));
                std::fs::write(&path, vec![u8::try_from(i).unwrap(); 10 + i]).unwrap();
                (i, path.to_str().unwrap().to_string())
            })
            .collect();
        (dir, items)
    }

    /// Each item's result in a pass whose `read` or `process` panics on item 3, and how many times `init` ran.
    fn a_pass_panicking_on_item_3(in_read: bool) -> (Vec<Option<Result<usize, bool>>>, usize) {
        let (dir, items) = numbered_files(if in_read { "pipeline-read-panic" } else { "pipeline-panic" }, 8);
        let opts = sized(2, 2, 4);
        let inits = AtomicUsize::new(0);
        let mut results = vec![None; items.len()];
        run(
            Pass { items: &items, opts: &opts, ctl: &Control::new() },
            |(i, p), wide| {
                assert!(!(in_read && *i == 3), "item 3 can't be read");
                wide.set(&[&units(p)]);
                Read::File
            },
            || {
                inits.fetch_add(1, Ordering::SeqCst);
                Ok::<(), Failed>(())
            },
            |_, i, bytes| {
                assert!(in_read || i != 3, "item 3 can't be processed");
                Ok::<usize, Failed>(bytes.len())
            },
            |i, out, _| {
                // `Err(true)`: the item's error is a panic.
                results[i] = Some(out.map_err(|e| e.is_panic()));
                Ok(true)
            },
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        (results, inits.into_inner())
    }

    // A stop asked while item 5 is processed ends the reading, and the pass sinks every item that ran: item 5's result
    // too, which lands after the quick items' (it sleeps first). Nothing that had not started runs.
    #[test]
    fn a_stop_sinks_the_items_in_flight_and_runs_no_more() {
        let (dir, items) = numbered_files("pipeline-stop", 40);
        let opts = sized(2, 4, 8);
        let ctl = Control::new();
        let ran = Mutex::new(Vec::new());
        let sunk = Mutex::new(Vec::new());
        run(
            Pass { items: &items, opts: &opts, ctl: &ctl },
            |(_, p), wide| {
                wide.set(&[&units(p)]);
                Read::File
            },
            || Ok::<(), Failed>(()),
            |_, i, _| {
                ran.lock().unwrap().push(i);
                if i == 5 {
                    ctl.request_stop();
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
                Ok::<usize, Failed>(i)
            },
            |i, out, _| {
                out.unwrap();
                sunk.lock().unwrap().push(i);
                Ok(true)
            },
        )
        .unwrap();
        let mut ran = ran.into_inner().unwrap();
        let mut sunk = sunk.into_inner().unwrap();
        ran.sort_unstable();
        sunk.sort_unstable();
        assert_eq!(sunk, ran, "every item that ran has its result sunk");
        assert!(sunk.contains(&5), "the item the stop was asked in has its result");
        assert!(ran.len() < items.len(), "the items not started were not run");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A sink that says stop ends the pass at once: it is not called again, and nothing is drained after it.
    #[test]
    fn a_sink_that_says_stop_is_not_called_again() {
        let (dir, items) = numbered_files("pipeline-sink-stop", 20);
        let opts = sized(2, 2, 4);
        let calls = AtomicUsize::new(0);
        run(
            Pass { items: &items, opts: &opts, ctl: &Control::new() },
            |(_, p), wide| {
                wide.set(&[&units(p)]);
                Read::File
            },
            || Ok::<(), Failed>(()),
            |_, i, _| Ok::<_, Failed>(i),
            |_, _, _| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(false)
            },
        )
        .unwrap();
        assert_eq!(calls.into_inner(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_panic_in_process_is_that_items_error_and_the_pass_goes_on() {
        let (results, inits) = within_timeout(|| a_pass_panicking_on_item_3(false));
        for (i, result) in results.iter().enumerate() {
            let expected = if i == 3 { Err(true) } else { Ok(10 + i) };
            assert_eq!(result.as_ref(), Some(&expected), "item {i}");
        }
        // Two workers, and the one that panicked made its state again.
        assert_eq!(inits, 3);
    }

    #[test]
    fn a_panic_in_read_is_that_items_error_and_the_pass_goes_on() {
        let (results, inits) = within_timeout(|| a_pass_panicking_on_item_3(true));
        for (i, result) in results.iter().enumerate() {
            let expected = if i == 3 { Err(true) } else { Ok(10 + i) };
            assert_eq!(result.as_ref(), Some(&expected), "item {i}");
        }
        assert_eq!(inits, 2);
    }

    #[test]
    fn a_failed_init_fails_the_pass() {
        let outcome = within_timeout(|| {
            let (dir, items) = numbered_files("pipeline-init", 4);
            let opts = sized(2, 2, 4);
            let outcome = run(
                Pass { items: &items, opts: &opts, ctl: &Control::new() },
                |(_, p), wide| {
                    wide.set(&[&units(p)]);
                    Read::File
                },
                || -> Result<(), Failed> { panic!("no state") },
                |_, _, _| Ok::<(), Failed>(()),
                |_, _, _| Ok(true),
            );
            let _ = std::fs::remove_dir_all(&dir);
            outcome.map_err(|e| format!("{e:#}"))
        });
        let error = outcome.unwrap_err();
        assert!(error.contains("worker setup") && error.contains("no state"), "{error}");
    }

    // `sink` fails while the job is paused: the readers paused at their checkpoint (two of them) stop waiting, and
    // the pass returns its error without a resume.
    #[test]
    fn a_sink_error_while_paused_returns_without_a_resume() {
        let outcome = within_timeout(|| {
            let (dir, items) = numbered_files("pipeline-paused", 40);
            let opts = sized(2, 1, 64);
            let ctl = Control::new();
            let outcome = run(
                Pass { items: &items, opts: &opts, ctl: &ctl },
                |(_, p), wide| {
                    wide.set(&[&units(p)]);
                    Read::File
                },
                || Ok::<(), Failed>(()),
                |_, _, _| Ok::<(), Failed>(()),
                |_, _, _| {
                    ctl.set_paused(true);
                    // Time for both readers to come back to their checkpoint and wait there.
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    Err(Failed::other("the sink fails".to_string()))
                },
            );
            let _ = std::fs::remove_dir_all(&dir);
            (outcome.map_err(|e| e.to_string()), ctl.is_paused())
        });
        assert_eq!(outcome, (Err("the sink fails".to_string()), true));
    }

    // A file opens as std opens it: a long path (verbatim), a name outside ASCII, and a missing file's error.
    #[test]
    fn files_open_as_std_opens_them() {
        let root = pg_testkit::run_folder("scratch_jobs", "open").unwrap();
        let mut dir = root.clone();
        while dir.as_os_str().len() < 300 {
            dir.push("a-folder-name-of-forty-characters-long--");
        }
        std::fs::create_dir_all(&dir).unwrap();
        let mut wide = NativePath::default();
        for name in ["f.bin", "fotografía ñandú 写真.jpg"] {
            let path = dir.join(name);
            std::fs::write(&path, name.as_bytes()).unwrap();
            wide.set(&[&units(path.to_str().unwrap())]);
            let mut bytes = Vec::new();
            pg_platform::file::open_read(&wide).unwrap().read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, name.as_bytes());
        }
        wide.set(&[&units(dir.join("missing.jpg").to_str().unwrap())]);
        let ours = pg_platform::file::open_read(&wide).unwrap_err();
        let std = File::open(dir.join("missing.jpg")).unwrap_err();
        assert_eq!((ours.kind(), ours.raw_os_error()), (std.kind(), std.raw_os_error()));
        let _ = std::fs::remove_dir_all(&root);
    }
    /// Four files of 5000, 6000, 7000 and 300 bytes, each read by [`Read::Gather`] with a hint of 1000: the length each
    /// gives `process`. The gather gives 10 bytes for the first, nothing for the second, and panics for the third; the
    /// fourth is no longer than the hint.
    fn a_gather_pass() -> Vec<usize> {
        let dir = pg_testkit::run_folder("scratch_jobs", "pipeline-gather").unwrap();
        let items: Vec<(usize, String)> = [5000usize, 6000, 7000, 300]
            .into_iter()
            .enumerate()
            .map(|(i, n)| {
                let path = dir.join(format!("{i}.bin"));
                std::fs::write(&path, vec![u8::try_from(i).unwrap(); n]).unwrap();
                (i, path.to_str().unwrap().to_string())
            })
            .collect();
        let opts = sized(2, 2, 4);
        let mut lengths = vec![0; items.len()];
        run(
            Pass { items: &items, opts: &opts, ctl: &Control::new() },
            |(_, p), wide| {
                wide.set(&[&units(p)]);
                Read::Gather {
                    hint: 1000,
                    gather: |file| match file.seek(SeekFrom::End(0)).unwrap() {
                        5000 => Ok(Some(vec![9; 10])),
                        6000 => Ok(None),
                        7000 => panic!("a gather that panics"),
                        _ => panic!("a file no longer than the hint is not gathered"),
                    },
                }
            },
            || Ok::<(), Failed>(()),
            |_, _, bytes| Ok::<usize, Failed>(bytes.len()),
            |i, out, _| {
                lengths[i] = out.unwrap();
                Ok(true)
            },
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        lengths
    }

    #[test]
    fn a_gather_gives_its_bytes_and_a_file_it_gives_nothing_for_is_read_whole() {
        assert_eq!(within_timeout(a_gather_pass), [10, 6000, 7000, 300]);
    }

    /// The reads in flight of a [`Slow`] source: now, and the most at once.
    #[derive(Default)]
    struct InFlight {
        now: AtomicUsize,
        most: AtomicUsize,
    }

    /// A source of 64 KiB files whose first read of each sleeps `delay`, counting the reads in flight: a disk whose
    /// throughput grows with the readers.
    struct Slow {
        delay: Duration,
        flight: Arc<InFlight>,
    }

    struct SlowFile {
        data: std::io::Cursor<Vec<u8>>,
        delay: Duration,
        flight: Arc<InFlight>,
        slept: bool,
    }

    impl std::io::Read for SlowFile {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if !std::mem::replace(&mut self.slept, true) {
                let now = self.flight.now.fetch_add(1, Ordering::SeqCst) + 1;
                self.flight.most.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(self.delay);
                self.flight.now.fetch_sub(1, Ordering::SeqCst);
            }
            self.data.read(buf)
        }
    }

    impl std::io::Seek for SlowFile {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.data.seek(pos)
        }
    }

    impl FileSource for Slow {
        fn open(&self, _path: &NativePath, _access: Access) -> std::io::Result<crate::source::Opened> {
            let file = SlowFile {
                data: std::io::Cursor::new(vec![7; 64 << 10]),
                delay: self.delay,
                flight: self.flight.clone(),
                slept: false,
            };
            Ok(crate::source::Opened { file: Box::new(file), size: 64 << 10 })
        }
    }

    /// Options of `readers` tuned from one reader up to `max`, with windows of 20 ms, over a [`Slow`] source of `delay`.
    fn tuned(max: usize, delay: Duration) -> (Options, Arc<InFlight>) {
        let flight = Arc::new(InFlight::default());
        let cfg = crate::tuner::TunerConfig::scaled(Duration::from_millis(20));
        let one = NonZeroUsize::MIN;
        let tuner =
            Arc::new(Mutex::new(Tuner::new(cfg, crate::tuner::Prior { start: one, expected: None, fallback: one })));
        let tuning = Tuning { max: NonZeroUsize::new(max).unwrap(), tuner, on_change: None };
        let mut opts = Options::sized(Readers::Auto(tuning), NonZeroUsize::new(2).unwrap(), Priority::Normal).unwrap();
        opts.source = Arc::new(Slow { delay, flight: flight.clone() });
        (opts, flight)
    }

    /// A pass of `items` over `opts` whose process does nothing: the results it sank.
    fn pass_of(items: usize, opts: &Options, ctl: &Control) -> usize {
        let paths: Vec<usize> = (0..items).collect();
        let mut sunk = 0;
        run(
            Pass { items: &paths, opts, ctl },
            |_, wide| {
                wide.set(&[&units("x")]);
                Read::File
            },
            || Ok::<(), Failed>(()),
            |_, _, bytes| Ok::<usize, Failed>(bytes.len()),
            |_, out, _| {
                out.unwrap();
                sunk += 1;
                Ok(true)
            },
        )
        .unwrap();
        sunk
    }

    /// A source whose file holds `actual` bytes but was `listed` long when it was opened: it changed while it was read.
    struct Changing {
        listed: usize,
        actual: usize,
    }

    impl FileSource for Changing {
        fn open(&self, _path: &NativePath, _access: Access) -> std::io::Result<crate::source::Opened> {
            let file = std::io::Cursor::new(vec![7; self.actual]);
            Ok(crate::source::Opened { file: Box::new(file), size: self.listed })
        }
    }

    /// One item read as `how` from a [`Changing`] source: its result, as the bytes processed or whether its failure is a
    /// changed read.
    fn read_changing(listed: usize, actual: usize, how: Read) -> Vec<Result<usize, bool>> {
        let mut opts = sized(1, 1, 4);
        opts.source = Arc::new(Changing { listed, actual });
        let mut outcomes = Vec::new();
        run(
            Pass { items: &[0usize], opts: &opts, ctl: &Control::new() },
            |_, wide| {
                wide.set(&[&units("x")]);
                how
            },
            || Ok::<(), Failed>(()),
            |_, _, bytes| Ok::<usize, Failed>(bytes.len()),
            |_, out, _| {
                outcomes.push(out.map_err(|e| e.is_changed()));
                Ok(true)
            },
        )
        .unwrap();
        outcomes
    }

    // A whole read whose bytes are not the file's length at its open (it shrank or grew while it was read) is a changed
    // read, a failure of its own (section 7); one whose length held is read. A gather's bytes are its own length on
    // purpose, and are not checked.
    #[test]
    fn a_whole_read_whose_length_changed_is_a_changed_read() {
        assert_eq!(read_changing(100, 60, Read::File), vec![Err(true)], "shrank");
        assert_eq!(read_changing(100, 140, Read::File), vec![Err(true)], "grew");
        assert_eq!(read_changing(100, 100, Read::File), vec![Ok(100)], "held");
        let gather = Read::Gather { hint: 10, gather: |_| Ok(Some(vec![1, 2, 3])) };
        assert_eq!(read_changing(100, 100, gather), vec![Ok(3)], "a gathered header");
    }

    // A fixed count lets every reader through: no more reads are in flight than the readers.
    #[test]
    fn a_fixed_count_reads_no_more_at_once_than_its_readers() {
        let flight = Arc::new(InFlight::default());
        let mut opts = sized(3, 2, 8);
        opts.source = Arc::new(Slow { delay: Duration::from_millis(2), flight: flight.clone() });
        assert_eq!(within_timeout(move || pass_of(60, &opts, &Control::new())), 60);
        assert!(flight.most.load(Ordering::SeqCst) <= 3);
    }

    // A pass of fewer items than its readers starts no more readers than items, and the parked readers are released
    // when the items run out: the pass ends, the scope joins.
    #[test]
    fn readers_parked_at_the_gate_are_released_when_the_items_run_out() {
        let (opts, flight) = tuned(16, Duration::from_millis(1));
        assert_eq!(within_timeout(move || pass_of(5, &opts, &Control::new())), 5);
        assert_eq!(flight.most.load(Ordering::SeqCst), 1, "one reader let through, the others parked");
    }

    // A stop while readers are parked ends the pass.
    #[test]
    fn a_stop_with_readers_parked_ends_the_pass() {
        let (opts, _) = tuned(8, Duration::from_millis(5));
        let sunk = within_timeout(move || {
            let ctl = Control::new();
            let paths: Vec<usize> = (0..10_000).collect();
            let mut sunk = 0usize;
            run(
                Pass { items: &paths, opts: &opts, ctl: &ctl },
                |_, wide| {
                    wide.set(&[&units("x")]);
                    Read::File
                },
                || Ok::<(), Failed>(()),
                |_, _, _| Ok::<(), Failed>(()),
                |_, _, _| {
                    sunk += 1;
                    if sunk == 20 {
                        ctl.request_stop();
                    }
                    Ok(true)
                },
            )
            .unwrap();
            sunk
        });
        assert!(sunk < 10_000, "the stop ended the pass after {sunk}");
    }

    // A pause while readers are parked, then a resume, and the pass completes.
    #[test]
    fn a_pause_with_readers_parked_completes_after_the_resume() {
        let (opts, _) = tuned(8, Duration::from_millis(1));
        let sunk = within_timeout(move || {
            let ctl = Arc::new(Control::new());
            ctl.set_paused(true);
            let resume = ctl.clone();
            let resumer = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                resume.set_paused(false);
            });
            let sunk = pass_of(200, &opts, &ctl);
            resumer.join().unwrap();
            sunk
        });
        assert_eq!(sunk, 200);
    }

    // A lease grown past its first room is given back as one buffer, kept at its grown size for the next file.
    #[test]
    fn a_grown_lease_is_given_back_as_one_buffer() {
        let pool = pool(4);
        let mut lease = pool.take(0, 100).unwrap();
        lease.grow_to(5000).unwrap();
        assert!(lease.bytes.capacity() >= 5000);
        check(&pool, &[&lease]);
        drop(lease);
        check(&pool, &[]);
        let st = pool.state.lock().unwrap();
        assert_eq!(st.idle.len(), 1);
        assert!(st.idle.iter().all(|b| b.capacity() >= 5000));
    }

    /// `pool.take(i, size)` on a thread of its own: the capacity granted (`None` once the pool stops), sent to the
    /// receiver. A test waits for it with a timeout, so a take that never comes back fails the test instead of hanging it.
    fn take_on_thread(pool: &Arc<Pool>, i: usize, size: usize) -> std::sync::mpsc::Receiver<Option<usize>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let pool = Arc::clone(pool);
        std::thread::spawn(move || {
            let _ = tx.send(pool.take(i, size).map(|lease| lease.bytes.capacity()));
        });
        rx
    }

    // A take that waits for a slot is granted when a buffer comes back, and a stop ends one that waits: it returns
    // `None`.
    #[test]
    fn a_take_waiting_for_a_slot_is_granted_when_one_is_back_or_ended_by_a_stop() {
        within_timeout(|| {
            let pool = Arc::new(pool(1));
            let a = pool.take(0, 60).unwrap();
            let waiting = take_on_thread(&pool, 1, 50);
            assert!(
                matches!(
                    waiting.recv_timeout(std::time::Duration::from_millis(100)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ),
                "a second buffer while the one slot is lent"
            );
            drop(a);
            let granted =
                waiting.recv_timeout(std::time::Duration::from_secs(10)).expect("granted once a slot is back");
            assert!(granted.is_some_and(|capacity| capacity >= 50));
            let b = pool.take(2, 10).unwrap();
            let waiting = take_on_thread(&pool, 3, 10);
            pool.stop();
            let ended = waiting.recv_timeout(std::time::Duration::from_secs(10)).expect("the stop ends the take");
            assert_eq!(ended, None);
            drop(b);
        });
    }
}

#[cfg(test)]
mod batched_tests {
    use pg_platform::priority::Priority;

    use super::*;

    fn options() -> Options {
        let mut opts = Options::new(Priority::Normal);
        opts.workers = NonZeroUsize::new(2).unwrap();
        opts
    }

    fn no_file(_: &usize, _: &mut NativePath) -> Read {
        Read::Streamed { bytes: 0 }
    }

    /// Each item's result: the item itself.
    fn itself(_: &mut (), i: &usize, _: &[u8]) -> Result<usize, ItemError<Failed>> {
        Ok(*i)
    }

    // A worker's setup that fails with the pass's error halts the pass with that error, typed (D3): not a failure of the
    // pipeline's own, and not a text to read it back from.
    #[test]
    fn a_failed_setup_is_the_passes_own_error() {
        #[derive(Debug, PartialEq)]
        struct NoMaker;
        let items: Vec<usize> = (0..10).collect();
        let ctl = Control::new();
        let (_, result) = batched(
            Pass { items: &items, opts: &options(), ctl: &ctl },
            no_file,
            || Err::<(), NoMaker>(NoMaker),
            |_, i, _| Ok::<usize, ItemError<NoMaker>>(*i),
            Batch { size: 100, every: None },
            (),
            |_, batch| {
                batch.clear();
                Ok(true)
            },
        );
        assert!(matches!(result, Err(Halt::Pass(NoMaker))), "{result:?}");
    }

    // An item's pass error reaches the flush with its item, and the flush halts the pass with it (D2).
    #[test]
    fn an_items_pass_error_halts_the_pass_through_the_flush() {
        let items: Vec<usize> = (0..300).collect();
        let ctl = Control::new();
        let (_, result) = batched(
            Pass { items: &items, opts: &options(), ctl: &ctl },
            no_file,
            || Ok::<(), String>(()),
            |_, i, _| if *i == 7 { Err(ItemError::Pass(format!("item {i}"))) } else { Ok(*i) },
            Batch { size: 50, every: None },
            (),
            |_, batch| {
                for (item, result) in batch.drain(..) {
                    if let Err(ItemError::Pass(e)) = result {
                        assert_eq!(*item, 7, "the error comes with its item");
                        return Err(e);
                    }
                }
                Ok(true)
            },
        );
        assert!(matches!(&result, Err(Halt::Pass(e)) if e == "item 7"), "{result:?}");
    }

    // Every result reaches the writer, in batches, and the writer's state comes back with the pass's `Ok`.
    #[test]
    fn every_result_is_written_and_the_writer_comes_back() {
        let items: Vec<usize> = (0..1000).collect();
        let ctl = Control::new();
        let (written, result) = batched(
            Pass { items: &items, opts: &options(), ctl: &ctl },
            no_file,
            || Ok::<(), Failed>(()),
            |_, i, _| Ok::<_, ItemError<Failed>>(*i * 2),
            Batch { size: 100, every: None },
            Vec::new(),
            |written: &mut Vec<usize>, batch| {
                written.extend(batch.drain(..).filter_map(|(_, r)| r.ok()));
                Ok(true)
            },
        );
        result.unwrap();
        let mut got = written;
        got.sort_unstable();
        assert_eq!(got, (0..1000).map(|i| i * 2).collect::<Vec<_>>());
        assert_eq!(ctl.snapshot().done, 1000);
    }

    // The count goes on while a write waits: a count held behind the write would stop at one batch.
    #[test]
    fn the_count_goes_on_while_a_write_waits() {
        let items: Vec<usize> = (0..400).collect();
        let ctl = Control::new();
        let (_, result) = batched(
            Pass { items: &items, opts: &options(), ctl: &ctl },
            no_file,
            || Ok::<(), Failed>(()),
            itself,
            Batch { size: 100, every: None },
            (),
            |_, batch| {
                let started = Instant::now();
                while ctl.snapshot().done < 150 {
                    if started.elapsed() > Duration::from_secs(20) {
                        return Err(Failed::other("the count stopped behind a write".to_string()));
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                batch.clear();
                Ok(true)
            },
        );
        result.unwrap();
        assert_eq!(ctl.snapshot().done, 400);
    }

    // A write that asks to stop ends the pass: nothing is written after it, and the pass is `Ok`.
    #[test]
    fn a_stop_from_the_writer_writes_nothing_more() {
        let items: Vec<usize> = (0..1000).collect();
        let ctl = Control::new();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let (_, result) = batched(
            Pass { items: &items, opts: &options(), ctl: &ctl },
            no_file,
            || Ok::<(), Failed>(()),
            itself,
            Batch { size: 100, every: None },
            (),
            |_, batch| {
                batch.clear();
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(false)
            },
        );
        result.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    // A write that fails ends the pass with its error, and the writer's state still holds what was written before.
    #[test]
    fn an_error_from_the_writer_keeps_what_was_written_before() {
        let items: Vec<usize> = (0..1000).collect();
        let ctl = Control::new();
        let (written, result) = batched(
            Pass { items: &items, opts: &options(), ctl: &ctl },
            no_file,
            || Ok::<(), Failed>(()),
            itself,
            Batch { size: 100, every: None },
            Vec::new(),
            |written: &mut Vec<usize>, batch| {
                if written.len() >= 100 {
                    return Err(Failed::other("disk full".to_string()));
                }
                written.extend(batch.drain(..).map(|(i, _)| *i));
                Ok(true)
            },
        );
        assert!(result.is_err());
        assert!(written.len() >= 100);
    }

    // A panic in a write is the pass's error, not a crash, and the writer's state comes back.
    #[test]
    fn a_panic_in_the_writer_is_the_passes_error() {
        let items: Vec<usize> = (0..300).collect();
        let ctl = Control::new();
        let (_, result) = batched(
            Pass { items: &items, opts: &options(), ctl: &ctl },
            no_file,
            || Ok::<(), Failed>(()),
            itself,
            Batch { size: 100, every: None },
            (),
            |_, batch| -> Result<bool, Failed> {
                batch.clear();
                panic!("a write panics");
            },
        );
        let error = result.unwrap_err();
        assert!(matches!(&error, Halt::Pipeline(failed) if failed.is_panic()), "{error}");
    }

    // A batch is never held longer than twice the batch size, however slow the writes are.
    #[test]
    fn no_batch_grows_past_twice_the_size() {
        let items: Vec<usize> = (0..2000).collect();
        let ctl = Control::new();
        let (largest, result) = batched(
            Pass { items: &items, opts: &options(), ctl: &ctl },
            no_file,
            || Ok::<(), Failed>(()),
            itself,
            Batch { size: 50, every: None },
            0usize,
            |largest: &mut usize, batch| {
                *largest = (*largest).max(batch.len());
                std::thread::sleep(Duration::from_millis(2));
                batch.clear();
                Ok(true)
            },
        );
        result.unwrap();
        assert!(largest <= 100, "a batch of {largest} results");
        assert!(largest >= 50);
    }
}
