//! The shape every pass over the originals shares (PLAN §4): files read in the given (on-disk) order with a few
//! reads in flight, processed on a CPU pool, results handed to one writer on the calling thread.
//!
//! Reads go to the disk in list order from `readers` threads at once. FINDINGS "I/O": ≥ 8 reads in flight let
//! the drive reorder them and reach sequential speed even in path order, and sparse reads only speed up with
//! queue depth.
//!
//! What bounds the memory is a byte budget (`Options::buffer_bytes`): every file is read into a buffer from a pool
//! of that many bytes, and handed back once processed. Readers wait while the budget is taken, so a folder of
//! large files holds as much as one of small files, and buffers are reused rather than allocated per file.

use std::fs::File;
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use anyhow::Result;
use crossbeam_channel::{bounded, unbounded};
use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY};

use crate::control::Control;

pub struct Options {
    pub readers: usize,
    pub workers: usize,
    /// The bytes of file buffers the pass holds at most (read, waiting, being processed, or idle in the pool). A
    /// file larger than all of it is read alone. [`Options::budget`] by default.
    pub buffer_bytes: usize,
    /// Thread priority of the readers and workers (`THREAD_PRIORITY_*`). Below normal keeps the app's own threads
    /// (the UI, thumbnails on screen) ahead of a background pass.
    pub priority: THREAD_PRIORITY,
}

/// The budget per read in flight and per worker: a large camera JPEG (24 MP at high quality).
const BUFFER_PER_SLOT: usize = 12 << 20;

impl Options {
    /// 8 reads in flight, a worker per logical CPU, and their [`Options::budget`].
    pub fn new(priority: THREAD_PRIORITY) -> Options {
        let cpus = std::thread::available_parallelism().map_or(4, |n| n.get());
        Options { readers: 8, workers: cpus, buffer_bytes: Options::budget(8, cpus), priority }
    }

    /// The default budget: a large camera file for each read in flight and each worker, since a worker holds its
    /// file while it processes it. Less starves the readers (FINDINGS "I/O": 24 MP JPEGs on the HDD, 8 readers and
    /// 12 workers, read 13% slower with 64 MB, and as fast as before with this, 240 MB).
    pub fn budget(readers: usize, workers: usize) -> usize {
        (readers + workers) * BUFFER_PER_SLOT
    }
}

fn set_priority(priority: THREAD_PRIORITY) {
    // SAFETY: GetCurrentThread's pseudo-handle is always valid for the calling thread.
    unsafe { SetThreadPriority(GetCurrentThread(), priority) };
}

/// Read each item's file (`read(item)`, its path), run `process(state, i, bytes)` (with per-worker state from
/// `init`), and call `sink(i, result, bytes read)` in completion order. `sink` returns false to stop early; so
/// does a stop request on `ctl`, and reading waits while `ctl` is paused. An item `read` gives no path for isn't
/// read: `process` gets no bytes and opens the file itself (videos, which are streamed, not loaded).
pub fn run<T, S, R>(
    items: &[T],
    read: impl Fn(&T) -> Option<&str> + Sync,
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
    let pool = Pool::new(opts.buffer_bytes);
    // Unbounded: the pool bounds what the read files hold; a video holds nothing.
    let (raw_tx, raw_rx) = unbounded::<(usize, std::io::Result<Lease>)>();
    let (res_tx, res_rx) = bounded::<(usize, Result<R>, usize)>(opts.workers * 4);

    std::thread::scope(|s| {
        // However the pass ends (done, stopped, an error from `sink`), readers waiting on the pool stop waiting.
        let _stop = StopOnDrop(&stop, &pool);
        for _ in 0..opts.readers {
            let raw_tx = raw_tx.clone();
            let (next, stop, read, pool) = (&next, &stop, &read, &pool);
            s.spawn(move || {
                set_priority(opts.priority);
                while !stop.load(Ordering::Relaxed) && ctl.checkpoint() {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let bytes = match read(item) {
                        Some(path) => match read_whole(pool, i, path) {
                            Some(bytes) => bytes,
                            None => break, // stopped
                        },
                        None => {
                            pool.pass(i);
                            Ok(Lease::empty())
                        }
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
                set_priority(opts.priority);
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

/// Read the file at `path` (item `i`) into a buffer from the pool, its size reserved in list order. `None` once the
/// pass stops.
fn read_whole<'p>(pool: &'p Pool, i: usize, path: &str) -> Option<std::io::Result<Lease<'p>>> {
    let mut file = match File::open(path).and_then(|f| Ok((f.metadata()?.len(), f))) {
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

/// File buffers within a byte budget. Reservations are granted in list order (item `i` after every item before
/// it), so the disk is read in order and a large file is never passed over by the smaller ones behind it.
struct Pool {
    budget: usize,
    state: Mutex<PoolState>,
    changed: Condvar,
}

struct PoolState {
    /// The next item to be granted.
    serving: usize,
    /// The capacity of every buffer there is: lent out, or idle.
    held: usize,
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
    fn new(budget: usize) -> Pool {
        let state = PoolState { serving: 0, held: 0, idle: Vec::new(), stopped: false };
        Pool { budget: budget.max(1), state: Mutex::new(state), changed: Condvar::new() }
    }

    /// A buffer that holds `size` bytes for item `i`, once the items before it have theirs and the budget has
    /// room (or nothing else is held, for a file larger than the budget). `None` once the pass stops.
    fn take(&self, i: usize, size: usize) -> Option<Lease<'_>> {
        let mut st = self.state.lock().unwrap();
        loop {
            if st.stopped {
                return None;
            }
            if st.serving == i {
                // The smallest idle buffer that is large enough.
                let fit = (st.idle.iter().enumerate())
                    .filter(|(_, b)| b.capacity() >= size)
                    .min_by_key(|(_, b)| b.capacity())
                    .map(|(k, _)| k);
                if let Some(k) = fit {
                    let mut bytes = st.idle.swap_remove(k);
                    bytes.clear();
                    return Some(self.grant(&mut st, bytes));
                }
                // None is: make room by freeing idle buffers, largest first.
                st.idle.sort_unstable_by_key(Vec::capacity);
                while st.held + size > self.budget
                    && let Some(b) = st.idle.pop()
                {
                    st.held -= b.capacity();
                }
                if st.held + size <= self.budget || st.held == 0 {
                    let bytes = Vec::with_capacity(size);
                    st.held += bytes.capacity();
                    return Some(self.grant(&mut st, bytes));
                }
            }
            st = self.changed.wait(st).unwrap();
        }
    }

    fn grant(&self, st: &mut PoolState, bytes: Vec<u8>) -> Lease<'_> {
        st.serving += 1;
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

    /// A buffer back from a worker (`counted`: its capacity when lent). One that grew past an eighth of the budget
    /// (a file far larger than the rest) is freed rather than kept.
    fn give(&self, bytes: Vec<u8>, counted: usize) {
        let mut st = self.state.lock().unwrap();
        st.held = st.held + bytes.capacity() - counted;
        if bytes.capacity() > self.budget / 8 {
            st.held -= bytes.capacity();
        } else {
            st.idle.push(bytes);
        }
        self.changed.notify_all();
    }

    fn stop(&self) {
        self.state.lock().unwrap().stopped = true;
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grants come in list order, within the budget, and a file larger than it alone.
    #[test]
    fn pool_grants_in_order_within_the_budget() {
        let pool = Pool::new(100);
        let a = pool.take(0, 60).unwrap();
        pool.pass(1);
        // 60 + 50 is over: item 2 waits until `a` is back, then reuses nothing (too small) but fits.
        std::thread::scope(|s| {
            let waiter = s.spawn(|| pool.take(2, 50).map(|l| l.bytes.capacity()));
            std::thread::sleep(std::time::Duration::from_millis(50));
            assert!(!waiter.is_finished());
            drop(a);
            assert!(waiter.join().unwrap().unwrap() >= 50);
        });
        // Over the budget: granted once nothing is held (each buffer so far was over an eighth of it, so freed).
        let big = pool.take(3, 500).unwrap();
        assert!(big.bytes.capacity() >= 500);
        drop(big);
        // It was freed, not kept (over an eighth of the budget).
        assert_eq!(pool.state.lock().unwrap().held, 0);
        // A small one is kept and reused.
        let small = pool.take(4, 10).unwrap();
        let cap = small.bytes.capacity();
        drop(small);
        assert_eq!(pool.take(5, 8).unwrap().bytes.capacity(), cap);
    }

    #[test]
    fn a_stopped_pool_wakes_its_waiters() {
        let pool = Pool::new(100);
        std::thread::scope(|s| {
            let waiter = s.spawn(|| pool.take(3, 10).is_none());
            std::thread::sleep(std::time::Duration::from_millis(50));
            pool.stop();
            assert!(waiter.join().unwrap());
        });
    }
}
