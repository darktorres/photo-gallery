//! The window's catalog reads off the GUI thread: the grid's files, the folder tree and the filter bar's counts
//! (FINDINGS "Startup"). One thread with its own catalog connection runs them, one at a time, oldest request first.
//! Each kind keeps only its newest request: a request replaces one of its kind not yet started (and goes to the
//! back), so typing in a filter or clicking through folders never queues stale queries, and a stream of one kind
//! (a refresh after every scan) never holds back the others.
//!
//! A newer request also cancels one of its kind running (its result would be dropped anyway): the connection's
//! statements fail until the thread takes the next request (`Catalog::cancel_when`).
//!
//! A request is a read of the catalog and a delivery of its result, which hands it to its QObject on the GUI thread
//! (`cxx_qt::Threading`); the object drops results older than its latest request. Every request is delivered, as
//! a result or an error (the connection couldn't be opened, the read failed or panicked), except one cut short by
//! a newer one or by `stop`.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;

use anyhow::{Result, anyhow};
use pg_core::catalog::{self, Catalog};

use crate::state;

/// What a request is for.
#[derive(Clone, Copy)]
pub enum Kind {
    Grid = 0,
    Tree = 1,
    Facets = 2,
}

const KINDS: usize = 3;

/// A request, given the reader's connection (or why there is none).
type Job = Box<dyn FnOnce(Result<&Catalog, &anyhow::Error>) + Send>;

#[derive(Default)]
struct Slots {
    /// Each kind's waiting request, with its place in line.
    jobs: [Option<(u64, Job)>; KINDS],
    next: u64,
    running: Option<usize>,
    stop: bool,
}

#[derive(Default)]
struct Reader {
    slots: Mutex<Slots>,
    wake: Condvar,
    /// Set to cancel the running request; cleared, under `slots`, when the next one is taken.
    cancel: Arc<AtomicBool>,
}

/// The reader and its thread, started by the first request.
struct Service {
    reader: Arc<Reader>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

static SERVICE: OnceLock<Service> = OnceLock::new();

fn reader() -> &'static Arc<Reader> {
    &SERVICE
        .get_or_init(|| {
            let reader = Arc::new(Reader::default());
            let worker = reader.clone();
            let thread = std::thread::Builder::new()
                .name("catalog reader".into())
                .spawn(move || worker.run())
                .expect("spawn the catalog reader");
            Service { reader, thread: Mutex::new(Some(thread)) }
        })
        .reader
}

/// Run `read` on the reader thread and give `deliver` its result there, replacing any `kind` request not yet
/// started, and interrupting one running. `deliver` is not called when the read was cut short (by a newer one,
/// by `stop`).
pub fn submit<T: 'static>(
    kind: Kind,
    read: impl FnOnce(&Catalog) -> Result<T> + Send + 'static,
    deliver: impl FnOnce(Result<T>) + Send + 'static,
) {
    let job: Job = Box::new(move |catalog| {
        let result = match catalog {
            // A panic is reported by the panic hook; the thread goes on serving the others.
            Ok(c) => std::panic::catch_unwind(AssertUnwindSafe(|| read(c)))
                .unwrap_or_else(|_| Err(anyhow!("the catalog read failed (an internal error)"))),
            Err(e) => Err(anyhow!("the catalog can't be opened: {e:#}")),
        };
        if !matches!(&result, Err(e) if catalog::interrupted(e)) {
            deliver(result);
        }
    });
    let reader = reader();
    let mut slots = reader.slots.lock().unwrap();
    let place = slots.next;
    slots.next += 1;
    slots.jobs[kind as usize] = Some((place, job));
    if slots.running == Some(kind as usize) {
        reader.cancel.store(true, Ordering::Relaxed);
    }
    reader.wake.notify_one();
}

/// Stop the thread (after the event loop): a running query is interrupted, waiting requests are dropped.
pub fn stop() {
    let Some(Service { reader, thread }) = SERVICE.get() else { return };
    {
        let mut slots = reader.slots.lock().unwrap();
        slots.stop = true;
        reader.cancel.store(true, Ordering::Relaxed);
    }
    reader.wake.notify_one();
    if let Some(thread) = thread.lock().unwrap().take() {
        let _ = thread.join();
    }
}

impl Reader {
    fn run(&self) {
        let mut catalog: Option<Catalog> = None;
        loop {
            let (kind, job) = {
                let mut slots = self.slots.lock().unwrap();
                loop {
                    if slots.stop {
                        return;
                    }
                    let oldest = (0..KINDS).filter_map(|k| slots.jobs[k].as_ref().map(|(place, _)| (*place, k))).min();
                    if let Some((_, kind)) = oldest {
                        let (_, job) = slots.jobs[kind].take().unwrap();
                        slots.running = Some(kind);
                        self.cancel.store(false, Ordering::Relaxed);
                        break (kind, job);
                    }
                    slots = self.wake.wait(slots).unwrap();
                }
            };
            // Opened by the first request, and again by the next one if that failed.
            let opened = match catalog.take() {
                Some(c) => Ok(c),
                None => {
                    Catalog::open_current(&state::get().catalog_path).inspect(|c| c.cancel_when(self.cancel.clone()))
                }
            };
            job(opened.as_ref());
            // The pages a job read aren't kept for the next: listing the whole library (189,518 files) fills the
            // page cache with 120 MB, which the connection would otherwise hold for as long as the app runs, a
            // second copy of what the file cache holds. Listing it again then takes ~100 ms more (360-430 ms
            // against 210-310 ms, measured on the pinterest catalog).
            if let Ok(c) = &opened
                && let Err(e) = c.conn.release_memory()
            {
                eprintln!("loader: {e:#}");
            }
            catalog = opened.ok();
            let mut slots = self.slots.lock().unwrap();
            if slots.running == Some(kind) {
                slots.running = None;
            }
        }
    }
}
