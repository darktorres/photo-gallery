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

use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use anyhow::Result;
use crossbeam_channel::bounded;
use pg_platform::priority::{self, Priority};

use crate::control::Control;

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

/// Read each item's file, run `process(state, i, bytes)` (with per-worker state from `init`), and call
/// `sink(i, result, bytes read)` in completion order. `sink` returns false to stop early; so does a stop request on
/// `ctl`, and reading waits while `ctl` is paused.
///
/// `read(item, wide)` writes the file's path into `wide`, as [`pg_platform::verbatim::verbatim_wide`] does, and returns
/// true; the reader's buffer is reused from file to file, so no path is made per file. An item it returns false
/// for isn't read: `process` gets no bytes and opens the file itself (videos, which are streamed, not loaded).
pub fn run<T, S, R>(
    items: &[T],
    read: impl Fn(&T, &mut Vec<u16>) -> bool + Sync,
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
    let stop = AtomicBool::new(false);
    let pool = Pool::new(opts.buffer_bytes, opts.slots);
    // A place per slot (an array, made once): a read file waits for the workers, never for the channel.
    let (raw_tx, raw_rx) = bounded::<(usize, std::io::Result<Lease>)>(pool.slots);
    let (res_tx, res_rx) = bounded::<(usize, Result<R>, usize)>(opts.workers * 4);

    std::thread::scope(|s| {
        // However the pass ends (done, stopped, an error from `sink`), readers waiting on the pool stop waiting.
        let _stop = StopOnDrop(&stop, &pool);
        for _ in 0..opts.readers {
            let raw_tx = raw_tx.clone();
            let (next, stop, read, pool) = (&next, &stop, &read, &pool);
            s.spawn(move || {
                priority::set_current_thread(opts.priority);
                let mut wide = Vec::new();
                while !stop.load(Ordering::Relaxed) && ctl.checkpoint() {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let bytes = if read(item, &mut wide) {
                        match read_whole(pool, i, &wide) {
                            Some(bytes) => bytes,
                            None => break, // stopped
                        }
                    } else {
                        pool.pass(i);
                        Ok(Lease::empty())
                    };
                    if raw_tx.send((i, bytes)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(raw_tx);

        for _ in 0..opts.workers {
            let (raw_rx, res_tx) = (raw_rx.clone(), res_tx.clone());
            let (init, process) = (&init, &process);
            s.spawn(move || {
                priority::set_current_thread(opts.priority);
                let mut state = match init() {
                    Ok(st) => st,
                    Err(e) => {
                        let _ = res_tx.send((usize::MAX, Err(e), 0));
                        return;
                    }
                };
                for (i, read) in raw_rx {
                    // The buffer goes back to the pool as the lease is dropped, before the result is sent.
                    let (out, len) = match read {
                        Ok(lease) => (process(&mut state, i, &lease.bytes), lease.bytes.len()),
                        Err(e) => (Err(anyhow::Error::new(e).context("read")), 0),
                    };
                    if res_tx.send((i, out, len)).is_err() {
                        break;
                    }
                }
            });
        }
        drop((raw_rx, res_tx));

        // Leaving this closure drops `res_rx`, which unblocks and ends the workers, then the readers.
        for (i, out, len) in res_rx {
            if i == usize::MAX {
                return Err(out.err().unwrap().context("worker setup"));
            }
            if !sink(i, out, len)? || ctl.stop_requested() {
                break;
            }
        }
        Ok(())
    })
}

/// Read the file at `path` (item `i`; NUL-terminated UTF-16) into a buffer from the pool, its size reserved in list
/// order. `None` once the pass stops.
fn read_whole<'p>(pool: &'p Pool, i: usize, path: &[u16]) -> Option<std::io::Result<Lease<'p>>> {
    let mut file = match pg_platform::file::open_read(path).and_then(|f| Ok((f.metadata()?.len(), f))) {
        Ok((size, file)) => (file, size as usize),
        Err(e) => {
            pool.pass(i);
            return Some(Err(e));
        }
    };
    let mut lease = pool.take(i, file.1)?;
    Some(file.0.read_to_end(&mut lease.bytes).map(|_| lease))
}

/// Ends the pass: tells the readers and the pool.
struct StopOnDrop<'a>(&'a AtomicBool, &'a Pool);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
        self.1.stop();
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
        let mut st = self.state.lock().unwrap();
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
                while st.held - smallest + size > self.budget && freed < others {
                    st.held -= st.idle[freed].capacity();
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
            st = self.changed.wait(st).unwrap();
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
        let mut st = self.state.lock().unwrap();
        while st.serving != i && !st.stopped {
            st = self.changed.wait(st).unwrap();
        }
        if st.serving == i {
            st.serving += 1;
            self.changed.notify_all();
        }
    }

    /// A buffer back from a worker, kept for the next file. `counted` is its capacity when lent: a file that grew
    /// while it was read grew it.
    fn give(&self, bytes: Vec<u8>, counted: usize) {
        let mut st = self.state.lock().unwrap();
        st.held = st.held + bytes.capacity() - counted;
        st.lent -= 1;
        st.idle.push(bytes);
        self.changed.notify_all();
    }

    fn stop(&self) {
        self.state.lock().unwrap().stopped = true;
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use pg_platform::verbatim::verbatim_wide;
    use std::fs::File;

    use super::*;

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
        let dir = pg_testkit::test_folder(r"D:\PhotoGallery\scratch_rate", "pg-pipeline");
        let paths: Vec<String> = (0..200)
            .map(|i| {
                let path = dir.join(format!("{i}.bin"));
                std::fs::write(&path, vec![i as u8; 100 + i * 7]).unwrap();
                path.to_str().unwrap().to_string()
            })
            .collect();
        let opts = Options { readers: 4, workers: 1, slots: 6, buffer_bytes: 1 << 20, priority: Priority::Normal };
        let mut seen = vec![false; paths.len()];
        run(
            &paths,
            |p, wide| {
                verbatim_wide(&[p], wide);
                true
            },
            &opts,
            &Control::new(),
            || Ok(()),
            |_, i, bytes| {
                std::thread::sleep(std::time::Duration::from_micros(200));
                assert_eq!(bytes.len(), 100 + i * 7);
                assert!(bytes.iter().all(|&b| b == i as u8));
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

    /// A file opens as std opens it: a long path (verbatim), a name outside ASCII, and a missing file's error.
    #[test]
    fn files_open_as_std_opens_them() {
        let root = pg_testkit::test_folder(r"D:\PhotoGallery\scratch_rate", "pg-open");
        let mut dir = root.clone();
        while dir.as_os_str().len() < 300 {
            dir.push("a-folder-name-of-forty-characters-long--");
        }
        std::fs::create_dir_all(&dir).unwrap();
        let mut wide = Vec::new();
        for name in ["f.bin", "fotografía ñandú 写真.jpg"] {
            let path = dir.join(name);
            std::fs::write(&path, name.as_bytes()).unwrap();
            verbatim_wide(&[path.to_str().unwrap()], &mut wide);
            let mut bytes = Vec::new();
            pg_platform::file::open_read(&wide).unwrap().read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, name.as_bytes());
        }
        verbatim_wide(&[dir.join("missing.jpg").to_str().unwrap()], &mut wide);
        let ours = pg_platform::file::open_read(&wide).unwrap_err();
        let std = File::open(dir.join("missing.jpg")).unwrap_err();
        assert_eq!((ours.kind(), ours.raw_os_error()), (std.kind(), std.raw_os_error()));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
