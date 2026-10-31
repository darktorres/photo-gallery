//! The shape every pass over the originals shares (PLAN §4): files read in the given (on-disk) order with a few
//! reads in flight, processed on a CPU pool, results handed to one writer on the calling thread.
//!
//! Reads go to the disk in list order from `readers` threads at once. FINDINGS "I/O": ≥ 8 reads in flight let
//! the drive reorder them and reach sequential speed even in path order, and sparse reads only speed up with
//! queue depth.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::Result;
use crossbeam_channel::bounded;
use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY};

use crate::control::Control;

pub struct Options {
    pub readers: usize,
    pub workers: usize,
    /// Thread priority of the readers and workers (`THREAD_PRIORITY_*`). Below normal keeps the app's own threads
    /// (the UI, thumbnails on screen) ahead of a background pass.
    pub priority: THREAD_PRIORITY,
}

impl Options {
    /// 8 reads in flight, a worker per logical CPU.
    pub fn new(priority: THREAD_PRIORITY) -> Options {
        let cpus = std::thread::available_parallelism().map_or(4, |n| n.get());
        Options { readers: 8, workers: cpus, priority }
    }
}

fn set_priority(priority: THREAD_PRIORITY) {
    // SAFETY: GetCurrentThread's pseudo-handle is always valid for the calling thread.
    unsafe { SetThreadPriority(GetCurrentThread(), priority) };
}

/// A long-path-safe form of an absolute path.
pub fn long_path(path: &str) -> String {
    if path.len() < 240 || path.starts_with(r"\\") { path.to_string() } else { format!(r"\\?\{path}") }
}

/// Read `paths[i]`, run `process(state, i, bytes)` (with per-worker state from `init`), and call `sink(i, result,
/// bytes read)` in completion order. `sink` returns false to stop early; so does a stop request on `ctl`, and
/// reading waits while `ctl` is paused. Files for which `whole(i)` is false aren't read: `process` gets no bytes
/// and opens the file itself (videos, which are streamed, not loaded).
pub fn run<S, R>(
    paths: &[String],
    whole: impl Fn(usize) -> bool + Sync,
    opts: &Options,
    ctl: &Control,
    init: impl Fn() -> Result<S> + Sync,
    process: impl Fn(&mut S, usize, &[u8]) -> Result<R> + Sync,
    mut sink: impl FnMut(usize, Result<R>, usize) -> Result<bool>,
) -> Result<()>
where
    R: Send,
{
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (raw_tx, raw_rx) = bounded::<(usize, std::io::Result<Vec<u8>>)>(opts.readers * 4);
    let (res_tx, res_rx) = bounded::<(usize, Result<R>, usize)>(opts.workers * 4);

    std::thread::scope(|s| {
        for _ in 0..opts.readers {
            let raw_tx = raw_tx.clone();
            let (next, stop, whole) = (&next, &stop, &whole);
            s.spawn(move || {
                set_priority(opts.priority);
                while !stop.load(Ordering::Relaxed) && ctl.checkpoint() {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(i) else { break };
                    let bytes = if whole(i) { std::fs::read(long_path(path)) } else { Ok(Vec::new()) };
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
                for (i, bytes) in raw_rx {
                    let (out, len) = match bytes {
                        Ok(b) => (process(&mut state, i, &b), b.len()),
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
                stop.store(true, Ordering::Relaxed);
                return Err(out.err().unwrap().context("worker setup"));
            }
            if !sink(i, out, len)? || ctl.stop_requested() {
                stop.store(true, Ordering::Relaxed);
                break;
            }
        }
        Ok(())
    })
}
