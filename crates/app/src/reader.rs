//! The window's catalog reads off its thread: the grid's files, the folder tree and the filter bar's counts (FINDINGS
//! "Startup"). One thread with its own catalog connection runs them, one at a time, oldest request first.
//!
//! A `Session` is one client's end of it (the desktop has one). Each of its kinds keeps only its newest request: a
//! request replaces that kind's request not yet started (and goes to the back), so typing in a filter or clicking
//! through folders never queues stale queries, and a stream of one kind never holds back the others. A newer request
//! also cancels the same kind's running one (its result would be dropped anyway): the connection's statements fail
//! until the thread takes the next request (`Catalog::cancel_when`).
//!
//! A request is a read of the catalog and a delivery of its result; the owner delivers it to its own thread. Every
//! request is delivered, as a result or as an error (the connection couldn't be opened, the read failed or panicked),
//! except one cut short by a newer one or by `stop`.

use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;

use pg_catalog::Catalog;
use pg_catalog::browse::read::facets::Facets;
use pg_model::details::Details;
use pg_model::grid::Entry;
use pg_model::ids::FileId;
use pg_model::query::{Query, Sort};
use pg_model::rules::Merged;
use pg_model::tree::FolderTree;

use crate::catalogs::Catalogs;
use crate::error::{Error, ReadError, Result, failed};

/// What a request is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Grid,
    Tree,
    Facets,
}

/// A request, given the reader's connection (or why there is none).
type Job = Box<dyn FnOnce(pg_catalog::Result<&Catalog, &pg_catalog::Error>) + Send>;

#[derive(Default)]
struct Slots {
    /// Each session's and kind's waiting request, with its place in line.
    jobs: BTreeMap<(u64, Kind), (u64, Job)>,
    next: u64,
    running: Option<(u64, Kind)>,
    stop: bool,
}

/// The reader and its thread, started by the first session.
pub(crate) struct Reader {
    slots: Mutex<Slots>,
    wake: Condvar,
    /// Set to cancel the running request; cleared, under `slots`, when the next one is taken.
    cancel: Arc<AtomicBool>,
    catalog_path: PathBuf,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Reader {
    /// Start the reader's thread; an error if it can't be started.
    pub(crate) fn start(catalog_path: PathBuf) -> Result<Arc<Reader>> {
        let reader = Arc::new(Reader {
            slots: Mutex::new(Slots::default()),
            wake: Condvar::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            catalog_path,
            thread: Mutex::new(None),
        });
        let worker = reader.clone();
        let thread = std::thread::Builder::new()
            .name("catalog reader".into())
            .spawn(move || worker.run())
            .map_err(|e| failed(format!("can't start the catalog reader: {e}")))?;
        *reader.thread.lock().unwrap_or_else(PoisonError::into_inner) = Some(thread);
        Ok(reader)
    }

    /// Stop the thread: a running query is interrupted, waiting requests are dropped. Waits for the thread to end.
    pub(crate) fn stop(&self) {
        {
            let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            slots.stop = true;
            self.cancel.store(true, Ordering::Relaxed);
        }
        self.wake.notify_one();
        if let Some(thread) = self.thread.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = thread.join();
        }
    }

    fn run(&self) {
        let mut catalog: Option<Catalog> = None;
        loop {
            let (key, job) = {
                let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
                loop {
                    if slots.stop {
                        return;
                    }
                    let oldest = slots.jobs.iter().min_by_key(|(_, (place, _))| *place).map(|(key, _)| *key);
                    if let Some((key, (_, job))) = oldest.and_then(|key| slots.jobs.remove_entry(&key)) {
                        slots.running = Some(key);
                        self.cancel.store(false, Ordering::Relaxed);
                        break (key, job);
                    }
                    slots = self.wake.wait(slots).unwrap_or_else(PoisonError::into_inner);
                }
            };
            // Opened by the first request, and again by the next one if that failed.
            let opened = match catalog.take() {
                Some(c) => Ok(c),
                None => Catalog::open(&self.catalog_path, pg_platform::native::PATH_RULES).and_then(|mut c| {
                    c.cancel_when(self.cancel.clone())?;
                    Ok(c)
                }),
            };
            job(opened.as_ref());
            // The pages a job read aren't kept for the next: listing the whole library (189,518 files) fills the page cache
            // with 120 MB, which the connection would otherwise hold for as long as the app runs, a second copy of what the
            // file cache holds. Listing it again then takes ~100 ms more (360-430 ms against 210-310 ms, measured on the
            // pinterest catalog). A failure to release them leaves them held, which costs memory and nothing else.
            if let Ok(c) = &opened {
                let _ = c.release_memory();
            }
            catalog = opened.ok();
            let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            if slots.running == Some(key) {
                slots.running = None;
            }
        }
    }
}

/// One client's end of the reads: its requests replace its own waiting requests of a kind, and no other client's.
#[derive(Clone)]
pub struct Session {
    id: u64,
    reader: Arc<Reader>,
}

impl Session {
    pub(crate) fn new(id: u64, reader: Arc<Reader>) -> Session {
        Session { id, reader }
    }

    /// Run `read` on the reader thread and give `deliver` its result there, replacing this session's `kind` request not
    /// yet started, and interrupting its `kind` request running. `deliver` is not called when the read was cut short (by
    /// a newer one, by `stop`).
    pub(crate) fn submit<T: 'static>(
        &self,
        kind: Kind,
        read: impl FnOnce(&Catalog) -> pg_catalog::Result<T> + Send + 'static,
        deliver: impl FnOnce(Result<T, ReadError>) + Send + 'static,
    ) {
        let job: Job = Box::new(move |catalog| {
            let result = match catalog {
                // A panic is logged by the panic hook and reported here; the thread goes on serving the others.
                Ok(c) => pg_jobs::panics::catch(AssertUnwindSafe(|| read(c)))
                    .map_err(|_| ReadError::Panicked)
                    .and_then(|read| read.map_err(ReadError::from)),
                Err(e) => Err(ReadError::Unopened(format!("{e:#}"))),
            };
            // A read that the next request (or `stop`) cut short is not delivered.
            if !matches!(&result, Err(ReadError::Catalog(pg_catalog::Error::Interrupted))) {
                deliver(result);
            }
        });
        let key = (self.id, kind);
        let reader = &self.reader;
        let mut slots = reader.slots.lock().unwrap_or_else(PoisonError::into_inner);
        let place = slots.next;
        slots.next += 1;
        slots.jobs.insert(key, (place, job));
        if slots.running == Some(key) {
            reader.cancel.store(true, Ordering::Relaxed);
        }
        reader.wake.notify_one();
    }

    /// The folder tree, merged across drives (its kind's newest request, as with `submit`).
    pub fn tree(&self, deliver: impl FnOnce(Result<FolderTree>) + Send + 'static) {
        self.submit(Kind::Tree, pg_catalog::Catalog::folder_tree, move |r| deliver(r.map_err(Error::from)));
    }

    /// The files of `folder` (merged). The empty folder is every file with `recursive`, else the drive roots' own
    /// files. Filtered by `query` and sorted by `sort`: what the grid shows.
    pub fn grid(
        &self,
        folder: Merged,
        recursive: bool,
        query: Query,
        sort: Sort,
        deliver: impl FnOnce(Result<Vec<Entry>>) + Send + 'static,
    ) {
        self.submit(
            Kind::Grid,
            move |c| c.list_grid(&folder, recursive, &query, sort),
            move |r| deliver(r.map_err(Error::from)),
        );
    }

    /// The filter bar's counts: the files of `folder` in all, and the ones `query` matches.
    pub fn facets(
        &self,
        folder: Merged,
        recursive: bool,
        query: Query,
        deliver: impl FnOnce(Result<Facets>) + Send + 'static,
    ) {
        self.submit(
            Kind::Facets,
            move |c| c.facets(&folder, recursive, &query),
            move |r| deliver(r.map_err(Error::from)),
        );
    }
}

/// The catalog reads: the reader thread's session (the grid, the folder tree, the filter bar's counts), and the reads a
/// front end makes on its own thread (a file's details and its grid entry).
pub struct Reading {
    reader: Arc<Reader>,
    session: Session,
    catalogs: Arc<Catalogs>,
}

impl Reading {
    /// Start the reader thread on the catalog at `catalog_path`; an error if the thread can't be started.
    pub(crate) fn start(catalog_path: PathBuf, catalogs: Arc<Catalogs>) -> Result<Reading> {
        let reader = Reader::start(catalog_path)?;
        let session = Session::new(1, reader.clone());
        Ok(Reading { reader, session, catalogs })
    }

    /// The catalog reader's session (the front end's one).
    pub fn session(&self) -> Session {
        self.session.clone()
    }

    /// Stop the reader thread (see [`Reader::stop`]).
    pub(crate) fn stop(&self) {
        self.reader.stop();
    }

    /// A file's details, read now; `None` when the file is gone from the catalog.
    pub fn details(&self, file_id: FileId) -> Result<Option<Details>> {
        self.catalogs.with_catalog(|c| c.details(file_id))
    }

    /// A file's grid entry as the catalog has it now; `None` when it is gone.
    pub fn entry(&self, file_id: FileId) -> Result<Option<Entry>> {
        self.catalogs.with_catalog(|c| c.grid_entry(file_id))
    }
}
