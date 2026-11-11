//! The shape every pass over the originals shares (PLAN §4): files read in the given (on-disk) order with a few
//! reads in flight, processed on a CPU pool, results handed to one writer on the calling thread.
//!
//! Reads go to the disk in list order from `readers` threads at once. FINDINGS "I/O": ≥ 8 reads in flight let
//! the drive reorder them and reach sequential speed even in path order, and sparse reads only speed up with
//! queue depth.
//!
//! What bounds the files in flight (read, waiting, or being processed) is their count (`Options::slots`) and their
//! bytes (`Options::buffer_bytes`). Every file is read into a buffer from a pool of at most that many buffers and
//! bytes, and handed back once processed. Readers wait while either is taken, so they stay a bounded way ahead of
//! the workers, a folder of large files holds no more than one of small files, and the buffers are reused from file
//! to file: a pass makes about `slots` of them, whatever its length.

use std::io::Read as _;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossbeam_channel::bounded;
use pg_platform::native::NativePath;
use pg_platform::priority::{self, Priority};

use crate::control::Control;
use crate::panics::{self, Panicked};

pub struct Options {
    pub readers: usize,
    pub workers: usize,
    /// The files in flight at most: being read, read and waiting for a worker, or being processed. A file a worker
    /// opens itself (a video) counts only while it waits. [`Options::slots`] by default.
    pub slots: usize,
    /// The bytes of file buffers the pass holds at most (read, waiting, being processed, or idle in the pool). A
    /// file larger than all of it is read alone. [`Options::budget`] by default.
    pub buffer_bytes: usize,
    /// Thread priority of the readers and workers (a `Priority`). Below normal keeps the app's own threads
    /// (the UI, thumbnails on screen) ahead of a background pass.
    pub priority: Priority,
}

/// The budget per read in flight and per worker: a large camera JPEG (24 MP at high quality).
const BUFFER_PER_SLOT: usize = 12 << 20;

impl Options {
    /// 8 reads in flight, a worker per logical CPU, and their [`Options::budget`].
    pub fn new(priority: Priority) -> Options {
        let cpus = std::thread::available_parallelism().map_or(4, |n| n.get());
        Options {
            readers: 8,
            workers: cpus,
            slots: Options::slots(8, cpus),
            buffer_bytes: Options::budget(8, cpus),
            priority,
        }
    }

    /// The default count of files in flight: one per read, and two per worker (the one it processes and the next,
    /// so that it doesn't wait for the disk). As fast as no bound by count (FINDINGS "I/O": 24 MP JPEGs on the HDD,
    /// 8 readers and 12 workers: 273 MB/s with 32 slots, or 56, or unbounded; 271 with 20).
    pub fn slots(readers: usize, workers: usize) -> usize {
        readers + 2 * workers
    }

    /// The default budget: a large camera file for each read in flight and each worker, since a worker holds its
    /// file while it processes it. Less starves the readers (FINDINGS "I/O": 24 MP JPEGs on the HDD, 8 readers and
    /// 12 workers, read 4% slower with 128 MB at the default slots, and no faster with 512 MB than with this,
    /// 240 MB).
    pub fn budget(readers: usize, workers: usize) -> usize {
        (readers + workers) * BUFFER_PER_SLOT
    }
}

/// How an item is read, as `read` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// Whole, from the path `read` wrote into the reader's buffer ([`NativePath::set`]): `process` gets its bytes.
    File,
    /// Not by the pipeline: `process` gets no bytes and opens the file itself (a video, which is streamed, not
    /// loaded). `bytes` is what it counts as read (the file's size, or 0 for a part of it).
    Streamed { bytes: u64 },
}

/// What a worker sends back: an item's result with the bytes it counts as read, or the error of a setup that failed
/// (or panicked), after which the worker stops.
enum Outcome<R> {
    Done { index: usize, result: Result<R>, read: usize },
    SetupFailed(anyhow::Error),
}

/// Read each item's file, run `process(state, i, bytes)` (with per-worker state from `init`), and call
/// `sink(i, result, bytes read)` in completion order. `sink` returns false to stop early; so does a stop request on
/// `ctl`, and reading waits while `ctl` is paused.
///
/// `read(item, wide)` says how the item is read ([`Read`]); for a file, it writes the path into `wide`, the reader's
/// buffer, reused from file to file, so no path is made per file.
///
/// A panic in `read` or `process` is that item's error ([`panics::Panicked`], through [`panics::guard`]), and the
/// pass goes on: a worker whose `process` panicked makes its state again with `init`. A panic in `init`, or an error
/// from it, fails the pass, as does a pass that ends with items sent to the workers and no result for them.
pub fn run<T, S, R>(
    items: &[T],
    read: impl Fn(&T, &mut NativePath) -> Read + Sync,
    opts: &Options,
    ctl: &Control,
    init: impl Fn() -> Result<S> + Sync,
    process: impl Fn(&mut S, usize, &[u8]) -> Result<R> + Sync,
    mut sink: impl FnMut(usize, Result<R>, usize) -> Result<bool>,
) -> Result<()>
where
    T: Sync,
    R: Send,
{
    let next = AtomicUsize::new(0);
    // The items the readers handed to the workers, to check that each came back with a result.
    let sent = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let pool = Pool::new(opts.buffer_bytes, opts.slots);
    // A place per slot (an array, made once): a read file waits for the workers, never for the channel.
    let (raw_tx, raw_rx) = bounded::<(usize, std::io::Result<Lease>, usize)>(pool.slots);
    let (res_tx, res_rx) = bounded::<Outcome<R>>(opts.workers * 4);

    std::thread::scope(|s| {
        // However the pass ends (done, stopped, an error from `sink`), readers waiting on the pool or paused at a
        // checkpoint stop waiting.
        let _stop = StopOnDrop(&stop, &pool, ctl);
        for _ in 0..opts.readers {
            let raw_tx = raw_tx.clone();
            let (next, sent, stop, read, pool) = (&next, &sent, &stop, &read, &pool);
            s.spawn(move || {
                priority::set_current_thread(opts.priority);
                let mut wide = NativePath::default();
                while ctl.checkpoint_until(stop) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let (bytes, streamed) = match panics::catch(AssertUnwindSafe(|| read(item, &mut wide))) {
                        Ok(Read::File) => match read_whole(pool, i, &wide) {
                            Some(bytes) => (bytes, 0),
                            None => break, // stopped
                        },
                        Ok(Read::Streamed { bytes }) => {
                            pool.pass(i);
                            (Ok(Lease::empty()), usize::try_from(bytes).unwrap_or(usize::MAX))
                        }
                        Err(payload) => {
                            // The item's turn in the pool is passed all the same, or the items after it would wait
                            // for it forever; the path it left half-written is not used again.
                            pool.pass(i);
                            wide = NativePath::default();
                            (Err(std::io::Error::other(Panicked(panics::message(&*payload).to_string()))), 0)
                        }
                    };
                    if raw_tx.send((i, bytes, streamed)).is_err() {
                        break;
                    }
                    sent.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
        drop(raw_tx);

        for _ in 0..opts.workers {
            let (raw_rx, res_tx) = (raw_rx.clone(), res_tx.clone());
            let (init, process) = (&init, &process);
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
                for (i, read, streamed) in raw_rx {
                    let (out, len, rebuild) = match read {
                        Ok(lease) => {
                            let len = lease.bytes.len().saturating_add(streamed);
                            let out = panics::guard(AssertUnwindSafe(|| process(&mut state, i, &lease.bytes)));
                            // The buffer goes back to the pool before the result is sent.
                            drop(lease);
                            // A panic may have left the state half-changed: it is made again.
                            let rebuild = out.as_ref().is_err_and(panics::is_panic);
                            (out, len, rebuild)
                        }
                        Err(e) => (Err(read_error(e)), 0, false),
                    };
                    if res_tx.send(Outcome::Done { index: i, result: out, read: len }).is_err() {
                        break;
                    }
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
        let mut received = 0usize;
        for outcome in res_rx {
            let (i, out, len) = match outcome {
                Outcome::Done { index, result, read } => (index, result, read),
                Outcome::SetupFailed(error) => return Err(error.context("worker setup")),
            };
            received += 1;
            if !sink(i, out, len)? || ctl.stop_requested() {
                return Ok(());
            }
        }
        // Every worker has ended, and every reader before them: what was sent is final.
        let sent = sent.load(Ordering::SeqCst);
        if received < sent {
            anyhow::bail!("the workers ended with {} of the {sent} items read and no result for them", sent - received);
        }
        Ok(())
    })
}

/// How a batched pass writes what it made: `size` results at a time, and with `every`, also once that long has passed
/// since the last write (so what a slow pass finds is not held back for long).
#[derive(Clone, Copy, Debug)]
pub struct Batch {
    pub size: usize,
    pub every: Option<Duration>,
}

/// [`run`] for a pass that writes its results in batches, the skeleton the per-file passes share: each result is
/// counted on `ctl` (its bytes, and whether it failed) and kept, and `flush` gets the kept ones (`(index, result)`, in
/// completion order) to write and empty, when the batch is due ([`Batch`]) and once more when the pass ends. `flush`
/// returns false to stop the pass, with no last flush. An error (of the pass, or of `flush`) loses the batch kept.
#[allow(clippy::too_many_arguments)]
pub fn batched<T, S, R>(
    items: &[T],
    read: impl Fn(&T, &mut NativePath) -> Read + Sync,
    opts: &Options,
    ctl: &Control,
    init: impl Fn() -> Result<S> + Sync,
    process: impl Fn(&mut S, usize, &[u8]) -> Result<R> + Sync,
    batch: Batch,
    mut flush: impl FnMut(&mut Vec<(usize, Result<R>)>) -> Result<bool>,
) -> Result<()>
where
    T: Sync,
    R: Send,
{
    let mut kept: Vec<(usize, Result<R>)> = Vec::with_capacity(batch.size);
    let mut since = Instant::now();
    let mut going = true;
    run(items, read, opts, ctl, init, process, |i, result, len| {
        ctl.add(len, result.is_err());
        kept.push((i, result));
        if kept.len() >= batch.size || batch.every.is_some_and(|every| since.elapsed() >= every) {
            since = Instant::now();
            going = flush(&mut kept)?;
        }
        Ok(going)
    })?;
    if going {
        flush(&mut kept)?;
    }
    Ok(())
}

/// The error of an item that couldn't be read. A panic in `read` comes as an I/O error holding it, and stays a panic.
fn read_error(e: std::io::Error) -> anyhow::Error {
    match e.get_ref().and_then(|inner| inner.downcast_ref::<Panicked>()) {
        Some(panicked) => anyhow::Error::new(panicked.clone()),
        None => anyhow::Error::new(e).context("read"),
    }
}

/// Read the file at `path` (item `i`) into a buffer from the pool, its size reserved in list
/// order. `None` once the pass stops.
fn read_whole<'p>(pool: &'p Pool, i: usize, path: &NativePath) -> Option<std::io::Result<Lease<'p>>> {
    let sized = |f: std::fs::File| -> std::io::Result<(usize, std::fs::File)> {
        let size = usize::try_from(f.metadata()?.len())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::FileTooLarge, e))?;
        Ok((size, f))
    };
    let mut file = match pg_platform::file::open_read(path).and_then(sized) {
        Ok((size, file)) => (file, size),
        Err(e) => {
            pool.pass(i);
            return Some(Err(e));
        }
    };
    let mut lease = pool.take(i, file.1)?;
    Some(file.0.read_to_end(&mut lease.bytes).map(|_| lease))
}

/// Ends the pass: tells the readers, the pool, and the readers paused at a checkpoint (which look at the stop under
/// the pause lock, so the wake can't come between their check and their wait).
struct StopOnDrop<'a>(&'a AtomicBool, &'a Pool, &'a Control);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
        self.1.stop();
        self.2.wake();
    }
}

/// File buffers within a count (`slots`) and a byte budget. Reservations are granted in list order (item `i` after
/// every item before it), so the disk is read in order and a large file is never passed over by the smaller ones
/// behind it.
struct Pool {
    budget: usize,
    slots: usize,
    state: Mutex<PoolState>,
    changed: Condvar,
}

struct PoolState {
    /// The next item to be granted.
    serving: usize,
    /// The capacity of every buffer there is: lent out, or idle.
    held: usize,
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
    /// Its capacity as the pool counts it.
    counted: usize,
}

impl Lease<'_> {
    /// No buffer (a file not read whole).
    fn empty() -> Self {
        Lease { pool: None, bytes: Vec::new(), counted: 0 }
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Some(pool) = self.pool {
            pool.give(std::mem::take(&mut self.bytes), self.counted);
        }
    }
}

impl Pool {
    fn new(budget: usize, slots: usize) -> Pool {
        let slots = slots.max(1);
        let state = PoolState { serving: 0, held: 0, lent: 0, idle: Vec::with_capacity(slots), stopped: false };
        Pool { budget: budget.max(1), slots, state: Mutex::new(state), changed: Condvar::new() }
    }

    /// A buffer that holds `size` bytes for item `i`, once the items before it have theirs, a slot is free, and the
    /// budget has room (or nothing else is held, for a file larger than the budget). `None` once the pass stops.
    ///
    /// The smallest idle buffer that is large enough is reused. If none is, the smallest idle one grows to `size`
    /// (the larger ones stay for the files that need them), and a new buffer is made only when none is idle. So
    /// there are never more buffers than slots, and a pass allocates once per slot, plus once per file larger than
    /// the buffer it is given.
    fn take(&self, i: usize, size: usize) -> Option<Lease<'_>> {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if st.stopped {
                return None;
            }
            if st.serving == i && st.lent < self.slots {
                // Largest first, so the smallest is last.
                st.idle.sort_unstable_by_key(|b| std::cmp::Reverse(b.capacity()));
                if let Some(k) = st.idle.iter().rposition(|b| b.capacity() >= size) {
                    let mut bytes = st.idle.remove(k);
                    bytes.clear();
                    return Some(self.grant(&mut st, bytes));
                }
                // None is large enough: the smallest grows, and the other idle ones are freed, largest first, while
                // it wouldn't fit the budget.
                let smallest = st.idle.last().map_or(0, Vec::capacity);
                let others = st.idle.len().saturating_sub(1);
                let mut freed = 0;
                let state = &mut *st;
                for buffer in state.idle.iter().take(others) {
                    if state.held - smallest + size <= self.budget {
                        break;
                    }
                    state.held -= buffer.capacity();
                    freed += 1;
                }
                st.idle.drain(..freed);
                if st.held - smallest + size <= self.budget || st.held == smallest {
                    let mut bytes = st.idle.pop().unwrap_or_default();
                    bytes.clear();
                    bytes.reserve_exact(size);
                    st.held = st.held - smallest + bytes.capacity();
                    return Some(self.grant(&mut st, bytes));
                }
            }
            st = self.changed.wait(st).unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn grant(&self, st: &mut PoolState, bytes: Vec<u8>) -> Lease<'_> {
        st.serving += 1;
        st.lent += 1;
        self.changed.notify_all();
        let counted = bytes.capacity();
        Lease { pool: Some(self), bytes, counted }
    }

    /// Item `i` takes no buffer (not read whole, or not readable): the next one is served once it is its turn.
    fn pass(&self, i: usize) {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        while st.serving != i && !st.stopped {
            st = self.changed.wait(st).unwrap_or_else(PoisonError::into_inner);
        }
        if st.serving == i {
            st.serving += 1;
            self.changed.notify_all();
        }
    }

    /// A buffer back from a worker, kept for the next file. `counted` is its capacity when lent: a file that grew
    /// while it was read grew it.
    fn give(&self, bytes: Vec<u8>, counted: usize) {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        st.held = st.held + bytes.capacity() - counted;
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

    /// What the pool holds: its bytes are the sum of its buffers' capacities, and there are no more buffers than
    /// slots.
    fn check(pool: &Pool, lent: &[&Lease]) {
        let st = pool.state.lock().unwrap();
        let capacities: usize =
            st.idle.iter().map(Vec::capacity).sum::<usize>() + lent.iter().map(|l| l.bytes.capacity()).sum::<usize>();
        assert_eq!(st.held, capacities);
        assert_eq!(st.lent, lent.len());
        assert!(st.idle.len() + st.lent <= pool.slots);
    }

    /// Grants come in list order, within the budget, and a file larger than it alone.
    #[test]
    fn pool_grants_in_order_within_the_budget() {
        let pool = Pool::new(100, 4);
        let a = pool.take(0, 60).unwrap();
        pool.pass(1);
        // 60 + 50 is over: item 2 waits until `a` is back, then grows it.
        std::thread::scope(|s| {
            let waiter = s.spawn(|| pool.take(2, 50).map(|l| l.bytes.capacity()));
            std::thread::sleep(std::time::Duration::from_millis(50));
            assert!(!waiter.is_finished());
            drop(a);
            assert!(waiter.join().unwrap().unwrap() >= 50);
        });
        // Over the budget: granted, alone, by growing the one buffer there is.
        let big = pool.take(3, 500).unwrap();
        assert!(big.bytes.capacity() >= 500);
        check(&pool, &[&big]);
        // While it is lent, nothing else fits.
        std::thread::scope(|s| {
            let waiter = s.spawn(|| pool.take(4, 10).map(|l| l.bytes.capacity()));
            std::thread::sleep(std::time::Duration::from_millis(50));
            assert!(!waiter.is_finished());
            drop(big);
            // Back, it is the only buffer, and large enough: reused.
            assert!(waiter.join().unwrap().unwrap() >= 500);
        });
        check(&pool, &[]);
    }

    /// Files of growing and shrinking sizes read one after another: one buffer, grown when a file needs more.
    #[test]
    fn a_pool_reuses_its_buffers_whatever_the_sizes() {
        let pool = Pool::new(1 << 20, 8);
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

    /// However far ahead the readers are, there are never more buffers than slots: a reader waits for a slot.
    #[test]
    fn a_pool_lends_no_more_buffers_than_slots() {
        let pool = Pool::new(1 << 30, 3);
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

    /// A growth that doesn't fit the budget frees the larger idle buffers first, and only as many as it needs.
    #[test]
    fn a_pool_frees_idle_buffers_to_fit_a_growth() {
        let pool = Pool::new(1000, 4);
        let leases: Vec<Lease> = [400, 300, 200].iter().enumerate().map(|(i, &n)| pool.take(i, n).unwrap()).collect();
        drop(leases);
        // 900 held. 500 doesn't fit any: the smallest (200) grows to 500, and 900 - 200 + 500 is over by 200, so the
        // largest (400) goes; 300 stays.
        let lease = pool.take(3, 500).unwrap();
        check(&pool, &[&lease]);
        let st = pool.state.lock().unwrap();
        assert_eq!(st.idle.iter().map(Vec::capacity).collect::<Vec<_>>(), [300]);
    }

    #[test]
    fn a_stopped_pool_wakes_its_waiters() {
        let pool = Pool::new(100, 1);
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

    /// A whole pass of small files with one slow worker, the readers far ahead of it: every file is read whole and
    /// processed once, through slots and a channel far smaller than the pass.
    #[test]
    fn a_pass_reads_every_file_within_its_slots() {
        let dir = pg_testkit::test_folder("scratch_rate", "pg-pipeline").unwrap();
        let paths: Vec<String> = (0..200)
            .map(|i| {
                let path = dir.join(format!("{i}.bin"));
                std::fs::write(&path, vec![u8::try_from(i).unwrap(); 100 + i * 7]).unwrap();
                path.to_str().unwrap().to_string()
            })
            .collect();
        let opts = Options { readers: 4, workers: 1, slots: 6, buffer_bytes: 1 << 20, priority: Priority::Normal };
        let mut seen = vec![false; paths.len()];
        run(
            &paths,
            |p, wide| {
                wide.set(&[&units(p)]);
                Read::File
            },
            &opts,
            &Control::new(),
            || Ok(()),
            |_, i, bytes| {
                std::thread::sleep(std::time::Duration::from_micros(200));
                assert_eq!(bytes.len(), 100 + i * 7);
                assert!(bytes.iter().all(|&b| usize::from(b) == i));
                Ok(())
            },
            |i, out, _| {
                out.unwrap();
                assert!(!std::mem::replace(&mut seen[i], true));
                Ok(true)
            },
        )
        .unwrap();
        assert!(seen.iter().all(|&s| s));
        std::fs::remove_dir_all(&dir).unwrap();
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
        let dir = pg_testkit::test_folder("scratch_rate", name).unwrap();
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
        let (dir, items) = numbered_files(if in_read { "pg-pipeline-read-panic" } else { "pg-pipeline-panic" }, 8);
        let opts = Options { readers: 2, workers: 2, slots: 4, buffer_bytes: 1 << 20, priority: Priority::Normal };
        let inits = AtomicUsize::new(0);
        let mut results = vec![None; items.len()];
        run(
            &items,
            |(i, p), wide| {
                assert!(!(in_read && *i == 3), "item 3 can't be read");
                wide.set(&[&units(p)]);
                Read::File
            },
            &opts,
            &Control::new(),
            || {
                inits.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            |_, i, bytes| {
                assert!(in_read || i != 3, "item 3 can't be processed");
                Ok(bytes.len())
            },
            |i, out, _| {
                // `Err(true)`: the item's error is a panic.
                results[i] = Some(out.map_err(|e| panics::is_panic(&e)));
                Ok(true)
            },
        )
        .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        (results, inits.into_inner())
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
            let (dir, items) = numbered_files("pg-pipeline-init", 4);
            let opts = Options { readers: 2, workers: 2, slots: 4, buffer_bytes: 1 << 20, priority: Priority::Normal };
            let outcome = run(
                &items,
                |(_, p), wide| {
                    wide.set(&[&units(p)]);
                    Read::File
                },
                &opts,
                &Control::new(),
                || -> Result<()> { panic!("no state") },
                |_, _, _| Ok(()),
                |_, _, _| Ok(true),
            );
            std::fs::remove_dir_all(&dir).unwrap();
            outcome.map_err(|e| format!("{e:#}"))
        });
        let error = outcome.unwrap_err();
        assert!(error.contains("worker setup") && error.contains("no state"), "{error}");
    }

    /// `sink` fails while the job is paused: the readers paused at their checkpoint (two of them) stop waiting, and
    /// the pass returns its error without a resume.
    #[test]
    fn a_sink_error_while_paused_returns_without_a_resume() {
        let outcome = within_timeout(|| {
            let (dir, items) = numbered_files("pg-pipeline-paused", 40);
            let opts = Options { readers: 2, workers: 1, slots: 64, buffer_bytes: 1 << 20, priority: Priority::Normal };
            let ctl = Control::new();
            let outcome = run(
                &items,
                |(_, p), wide| {
                    wide.set(&[&units(p)]);
                    Read::File
                },
                &opts,
                &ctl,
                || Ok(()),
                |_, _, _| Ok(()),
                |_, _, _| {
                    ctl.set_paused(true);
                    // Time for both readers to come back to their checkpoint and wait there.
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    anyhow::bail!("the sink fails")
                },
            );
            std::fs::remove_dir_all(&dir).unwrap();
            (outcome.map_err(|e| e.to_string()), ctl.is_paused())
        });
        assert_eq!(outcome, (Err("the sink fails".to_string()), true));
    }

    /// A file opens as std opens it: a long path (verbatim), a name outside ASCII, and a missing file's error.
    #[test]
    fn files_open_as_std_opens_them() {
        let root = pg_testkit::test_folder("scratch_rate", "pg-open").unwrap();
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
        std::fs::remove_dir_all(&root).unwrap();
    }
}
