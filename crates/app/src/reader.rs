//! The window's catalog reads off its thread: the grid's files, the folder tree and the filter bar's counts (FINDINGS
//! "Startup"), and the inbox's own reads: a ledger entry's files (or folders) when it is expanded, what Copy puts on the
//! clipboard, and the folder a Reveal opens, so the inbox reads nothing on the window's thread. One thread with its own
//! catalog connection runs them, one at a time, oldest request first. The inbox's reads are the user's, so waiting
//! behind a listing that is running is acceptable: each has a kind of its own, so none cancels another.
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
use pg_catalog::ledger::{Counts, FailedFile, FailedFolder};
use pg_model::details::Details;
use pg_model::grid::Entry;
use pg_model::ids::FileId;
use pg_model::notice::LedgerKind;
use pg_model::query::{Query, Sort};
use pg_model::rules::{Merged, Path as RulePath};
use pg_model::tree::FolderTree;

use crate::catalogs::Catalogs;
use crate::error::{Error, ReadError, Result, failed};

/// What a request is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Grid,
    Tree,
    Facets,
    /// An expanded ledger entry's list ([`Session::failed_files`]).
    FailedFiles,
    /// What the inbox's Copy puts on the clipboard ([`Session::copy_details`]).
    CopyDetails,
    /// The folder a Reveal opens ([`Session::reveal_folder`]).
    RevealFolder,
}

/// What a ledger entry lists: a pass's failed files, or the folders the scans couldn't take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailedItems {
    /// Newest first (`pg_catalog::Catalog::failed_files`).
    Files(Vec<FailedFile>),
    /// Grouped by the library folder that holds them (`pg_catalog::Catalog::failed_folders`).
    Folders(Vec<FailedFolder>),
}

impl FailedItems {
    pub fn len(&self) -> usize {
        match self {
            FailedItems::Files(files) => files.len(),
            FailedItems::Folders(folders) => folders.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A ledger entry's detail: what it lists (at most the limit asked for) and its counts over all its rows, read together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failed {
    pub kind: LedgerKind,
    pub items: FailedItems,
    pub counts: Counts,
}

/// The detail of `kind`'s ledger, at most `limit` items, new past `seen_to` (the mark the window kept when the inbox
/// opened; `None`: the acknowledgement as it is now).
fn read_failed(c: &Catalog, kind: LedgerKind, limit: usize, seen_to: Option<i64>) -> pg_catalog::Result<Failed> {
    let items = match kind {
        LedgerKind::File(pass) => FailedItems::Files(c.failed_files_since(pass, limit, seen_to)?),
        LedgerKind::ScanFolders => FailedItems::Folders(c.failed_folders_since(limit, seen_to)?),
    };
    let mut ack = c.ledger_ack(kind)?;
    if let Some(seen_to) = seen_to {
        ack.seen_to = seen_to;
    }
    let counts = c.ledger_counts(kind, &ack)?;
    Ok(Failed { kind, items, counts })
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

    /// What `kind`'s expanded entry lists, at most `limit`, with its counts (its kind's newest request, as with
    /// `submit`): the list shows "loading" until it is delivered.
    /// New is past `seen_to` (see `read_failed`).
    pub fn failed_files(
        &self,
        kind: LedgerKind,
        limit: usize,
        seen_to: Option<i64>,
        deliver: impl FnOnce(Result<Failed>) + Send + 'static,
    ) {
        self.submit(
            Kind::FailedFiles,
            move |c| read_failed(c, kind, limit, seen_to),
            move |r| deliver(r.map_err(Error::from)),
        );
    }

    /// What the inbox's Copy of `kind`'s entry adds to its words: its files (or folders), at most `limit`. Its own kind,
    /// so a Copy doesn't cancel the expanded list's read, nor that one the Copy.
    pub fn copy_details(
        &self,
        kind: LedgerKind,
        limit: usize,
        seen_to: Option<i64>,
        deliver: impl FnOnce(Result<Failed>) + Send + 'static,
    ) {
        self.submit(
            Kind::CopyDetails,
            move |c| read_failed(c, kind, limit, seen_to),
            move |r| deliver(r.map_err(Error::from)),
        );
    }

    /// The folder of `file_id` where a Reveal opens it, if the library shows the file (present); `None` for one gone or
    /// missing (the front end says the file is no longer in the library). A newer Reveal supersedes it: its delivery is
    /// dropped.
    pub fn reveal_folder(&self, file_id: FileId, deliver: impl FnOnce(Result<Option<RulePath>>) + Send + 'static) {
        self.submit(
            Kind::RevealFolder,
            move |c| c.present_file_folder(file_id),
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

#[cfg(test)]
mod tests {
    use super::*;
    use pg_catalog::fixture::{self, File};
    use pg_catalog::ledger::Failure;
    use pg_model::ids::FolderId;
    use pg_model::notice::FilePass;
    use pg_model::turns::Turns;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::time::Duration;

    const THUMBS: LedgerKind = LedgerKind::File(FilePass::Thumbs);

    /// A catalog in a test folder holding files 1 and 2, both with a thumbnail failure, and the folder's path.
    fn catalog(name: &str) -> (PathBuf, PathBuf) {
        let dir = pg_testkit::test_folder("scratch_app", name).unwrap();
        let layout = pg_library::layout::Layout::in_home(&dir);
        std::fs::create_dir_all(layout.catalog_dir()).unwrap();
        let c = pg_library::open_catalog(&layout).unwrap();
        let folder = dir.to_str().unwrap();
        fixture::state::add_scan(&c, 1, folder).unwrap();
        fixture::state::add_folder(&c, 1, folder).unwrap();
        let files = [
            File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 0),
            File::new(FileId::new(2), FolderId::new(1), "b.jpg", 10, 0),
        ];
        fixture::insert(&c, &files).unwrap();
        let failure = |id| Failure {
            file_id: FileId::new(id),
            mtime: 0,
            size: 10,
            rotation: Turns::NONE,
            error: "x".into(),
            retry: false,
        };
        c.record_failures(FilePass::Thumbs, &[failure(1), failure(2)], 1).unwrap();
        drop(c);
        (layout.catalog_path(), dir)
    }

    /// Hold the reader's thread until the sender is dropped (or sends), so the requests made meanwhile wait.
    fn hold(session: &Session) -> Sender<()> {
        let (release, held) = channel::<()>();
        session.submit(
            Kind::Grid,
            move |_| {
                let _ = held.recv();
                Ok(())
            },
            |_: Result<(), ReadError>| {},
        );
        release
    }

    fn got<T>(deliveries: &Receiver<T>) -> T {
        deliveries.recv_timeout(Duration::from_secs(20)).expect("delivered")
    }

    /// Every request made before this one has been run (the reader runs them oldest first).
    fn settle(session: &Session) {
        let (done, wait) = channel();
        session.submit(Kind::Tree, |_| Ok(()), move |_: Result<(), ReadError>| done.send(()).unwrap());
        got(&wait);
    }

    // A newer request for a ledger's list replaces the waiting one: only the newer is delivered.
    #[test]
    fn a_superseded_failed_files_request_is_not_delivered() {
        let (path, dir) = catalog("reader-failed-superseded");
        let reader = Reader::start(path).unwrap();
        let session = Session::new(1, reader.clone());
        let (deliver, deliveries) = channel();
        let release = hold(&session);
        for limit in [1, 10] {
            let deliver = deliver.clone();
            session.failed_files(THUMBS, limit, None, move |r| deliver.send(r.map(|f| f.items.len())).unwrap());
        }
        drop(release);
        assert_eq!(got(&deliveries).unwrap(), 2, "the newer request's limit");
        settle(&session);
        assert!(deliveries.try_recv().is_err(), "the older one is not delivered");
        reader.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The inbox's three kinds don't cancel each other: each is delivered, with what it read. The list's counts are over
    // every row, whatever the limit.
    #[test]
    fn the_inboxs_kinds_dont_cancel_each_other() {
        let (path, dir) = catalog("reader-three-kinds");
        let reader = Reader::start(path).unwrap();
        let session = Session::new(1, reader.clone());
        let (failed, failed_got) = channel();
        let (copied, copied_got) = channel();
        let (revealed, revealed_got) = channel();
        let release = hold(&session);
        session.failed_files(THUMBS, 1, None, move |r| failed.send(r).unwrap());
        session.copy_details(THUMBS, 10, None, move |r| copied.send(r).unwrap());
        session.reveal_folder(FileId::new(1), move |r| revealed.send(r).unwrap());
        drop(release);
        let list = got(&failed_got).unwrap();
        assert_eq!((list.kind, list.items.len(), list.counts.total, list.counts.new), (THUMBS, 1, 2, 2));
        let FailedItems::Files(files) = got(&copied_got).unwrap().items else { panic!("files") };
        let mut ids: Vec<i64> = files.iter().map(|f| f.file_id.get()).collect();
        ids.sort_unstable();
        assert_eq!(ids, [1, 2]);
        let folder = got(&revealed_got).unwrap().expect("a present file's folder");
        assert_eq!(folder, pg_platform::native::PATH_RULES.split_file_name(&files[0].path).unwrap().0);
        // The scans' ledger is listed by its folders (none here).
        let (folders, folders_got) = channel();
        session.failed_files(LedgerKind::ScanFolders, 10, None, move |r| folders.send(r).unwrap());
        let list = got(&folders_got).unwrap();
        assert!(matches!(list.items, FailedItems::Folders(ref f) if f.is_empty()));
        assert_eq!(list.counts.total, 0);
        reader.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A newer Reveal supersedes the waiting one, whose delivery is dropped; a file the catalog doesn't have is `None`.
    #[test]
    fn a_superseded_reveal_is_dropped() {
        let (path, dir) = catalog("reader-reveal");
        let reader = Reader::start(path).unwrap();
        let session = Session::new(1, reader.clone());
        let (deliver, deliveries) = channel();
        let release = hold(&session);
        for file in [1, 99] {
            let deliver = deliver.clone();
            session.reveal_folder(FileId::new(file), move |r| deliver.send((file, r.unwrap())).unwrap());
        }
        drop(release);
        assert_eq!(got(&deliveries), (99, None));
        settle(&session);
        assert!(deliveries.try_recv().is_err());
        reader.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A read that can't be made is delivered as an error, for every kind.
    #[test]
    fn an_error_is_delivered_as_an_error() {
        let dir = pg_testkit::test_folder("scratch_app", "reader-error").unwrap();
        let reader = Reader::start(dir.join("no-such-catalog.sqlite")).unwrap();
        let session = Session::new(1, reader.clone());
        let (failed, failed_got) = channel();
        let (copied, copied_got) = channel();
        let (revealed, revealed_got) = channel();
        session.failed_files(THUMBS, 10, None, move |r| failed.send(r.map(|_| ())).unwrap());
        session.copy_details(THUMBS, 10, None, move |r| copied.send(r.map(|_| ())).unwrap());
        session.reveal_folder(FileId::new(1), move |r| revealed.send(r.map(|_| ())).unwrap());
        for result in [got(&failed_got), got(&copied_got), got(&revealed_got)] {
            let e = result.unwrap_err();
            assert!(e.to_string().contains("can't be opened"), "{e}");
        }
        reader.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
