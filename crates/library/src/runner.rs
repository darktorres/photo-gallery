//! The app's background job (PLAN §4, §6): one thread that saves the cropped copies asked for, scans the folders
//! asked for, imports whatever is pending, looks for the borders of what it imported (and of everything after a
//! new detector), then makes thumbnails at the grid's current height, and sleeps until there is something new to
//! do.
//!
//! The thread is a `pg_jobs::service::Service`, and a round is a schedule of passes over the queue of requests. A new
//! request (a folder added, a rescan, a new thumbnail height) stops the pass in progress at the next file; it commits
//! what it has and the next round starts with the new work. Every pass is resumable from the catalog, so stopping
//! costs nothing but the batch in flight.

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use anyhow::{Context, Result};
use pg_catalog::Catalog;
use pg_jobs::control::Control;
use pg_jobs::events::{Bus, Change};
use pg_jobs::failure::{Cause, FileError};
use pg_jobs::panics;
use pg_jobs::pipeline::Options;
use pg_jobs::schedule::{self, Pass, Step};
use pg_jobs::service::{Service, Waker, Woke};
use pg_model::ids::FileId;
use pg_model::notice::{ERROR_CAP, FileFailure, FilePass, Key, KeyBuf, Notice, Part, tidy};
use pg_model::progress::Progress;
use pg_model::rules::Path as RulePath;
use pg_platform::priority::Priority;
use pg_scan::ScanError;
use pg_thumbcache::ThumbCache;

use crate::scans;

#[derive(Default)]
struct Queue {
    /// Folders to scan, normalized.
    scans: VecDeque<RulePath>,
    /// Cropped copies to save: the files, and the folder.
    crops: VecDeque<(Vec<FileId>, PathBuf)>,
}

struct Inner {
    catalog_path: PathBuf,
    thumbs: &'static ThumbCache,
    ctl: Control,
    queue: Mutex<Queue>,
    /// The folder a scan is walking now ([`Runner::stop_scans_under`]).
    scanning: Mutex<Option<RulePath>>,
    thumb_height: AtomicU32,
    scans_done: AtomicU64,
    imports_done: AtomicU64,
    /// Rounds finished, however they ended: the app marks each one as a stage for measuring (docs/PROFILING.md).
    rounds: AtomicU64,
    notices: Arc<dyn NoticeSink>,
    /// Told what changed (`Change`), so the owner reads the new state instead of polling for it.
    changes: Arc<Bus<Change>>,
}

/// Where the job's notices go (the app's inbox, a test's list): what it ran into, how a job asked for went, and which of
/// its kept notices no longer hold. Called on the job's thread: an implementation queues, it doesn't wait.
pub trait NoticeSink: Send + Sync {
    /// Tell the user `notice`.
    fn post(&self, notice: Notice);
    /// The kept notices of `scope`'s kind and folder (and part, if it has one) are now `notices`: the others no longer
    /// hold (`pg_catalog::Catalog::replace_notices`).
    fn replace(&self, scope: KeyBuf, notices: Vec<Notice>);
    /// The kept notices `key` names no longer hold (a key with no part names every part).
    fn clear(&self, key: KeyBuf) {
        self.replace(key, Vec::new());
    }
}

/// The notice of a crop job's file failure.
fn file_failure(error: FileError) -> FileFailure {
    let (error_text, panicked) = match error.cause {
        Cause::Error(text) => (text, false),
        Cause::Panicked(message) => (message, true),
    };
    FileFailure { path: error.path, error: tidy(&error_text, ERROR_CAP), panicked }
}

/// A count as a notice holds it.
fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// An error's text as a notice holds it: its causes after it, on one line, capped.
fn error_text(error: &anyhow::Error) -> String {
    tidy(&format!("{error:#}"), ERROR_CAP)
}
/// What the job has done so far, for the app to refresh its views.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Generation {
    /// Folder scans that changed the catalog: files new, moved, renamed, rewritten, back or gone (the folder tree
    /// and the lists changed). A scan that found the files as they were counts for nothing: the app would read its
    /// tree and its list again for no change.
    pub scans: u64,
    /// Import passes that analyzed something (sizes and placeholders changed).
    pub imports: u64,
}

pub struct Runner {
    inner: Arc<Inner>,
    service: Service,
}

impl Runner {
    /// Start the job thread. It begins by importing whatever is pending and making the thumbnails still missing
    /// at `thumb_height`. What it changes is published on `changes` (`Change`), and what it tells the user goes to
    /// `notices`. An error if the thread can't be spawned.
    pub fn start(
        catalog_path: PathBuf,
        thumbs: &'static ThumbCache,
        thumb_height: u32,
        changes: Arc<Bus<Change>>,
        notices: Arc<dyn NoticeSink>,
    ) -> Result<Runner> {
        let inner = Arc::new(Inner {
            catalog_path,
            thumbs,
            ctl: Control::new(),
            queue: Mutex::new(Queue::default()),
            scanning: Mutex::new(None),
            thumb_height: AtomicU32::new(thumb_height),
            scans_done: AtomicU64::new(0),
            imports_done: AtomicU64::new(0),
            rounds: AtomicU64::new(0),
            notices,
            changes,
        });
        let announce = inner.changes.clone();
        inner.ctl.on_change(move || announce.publish(Change::Progress));
        let job = inner.clone();
        let service =
            Service::start("background job", move |waker| job.run(waker)).context("start the background job")?;
        Ok(Runner { inner, service })
    }

    /// Stop the job after the file (or the batch) in progress, as a new request does, and wait for its thread to end,
    /// for at most `timeout`: its connections close with it. `false` if it is still running then; it ends with the
    /// process, and the WAL it left is merged at the next open.
    pub fn stop(&self, timeout: Duration) -> bool {
        // The quit first: the job clears the last request's stop and then looks for a quit, so a stop asked after
        // the clear is one it sees.
        self.service.quit();
        self.inner.ctl.request_stop();
        self.service.stop_within(timeout)
    }

    /// Scan these folders (normalized), then import and thumbnail what they hold.
    pub fn scan(&self, roots: Vec<RulePath>) {
        {
            let mut q = self.inner.queue();
            for r in roots {
                if !q.scans.contains(&r) {
                    q.scans.push_back(r);
                }
            }
        }
        self.wake_up();
    }

    /// A library folder was removed: the scans asked for under `root` are dropped, and one walking a folder under it now
    /// stops at its next batch (what it recorded is committed; it isn't asked for again, as the folder is no longer in
    /// the library).
    pub fn stop_scans_under(&self, root: &RulePath) {
        let rules = pg_platform::native::PATH_RULES;
        self.inner.queue().scans.retain(|path| !rules.is_under(path, root));
        let running = self.inner.scanning.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if running.is_some_and(|path| rules.is_under(&path, root)) {
            self.wake_up();
        }
    }

    /// Save cropped copies of these files' borders into `folder` (`pg_crop::save`), before anything else.
    pub fn save_crops(&self, ids: Vec<FileId>, folder: PathBuf) {
        self.inner.queue().crops.push_back((ids, folder));
        self.wake_up();
    }

    /// The catalog has new work outside a scan (a file turned in place needs its analysis redone): start a round.
    pub fn wake(&self) {
        self.wake_up();
    }

    /// The grid's thumbnail height changed: make the missing ones at the new height instead.
    pub fn set_thumb_height(&self, height: u32) {
        if self.inner.thumb_height.swap(height, Ordering::Relaxed) != height {
            self.wake_up();
        }
    }

    pub fn set_paused(&self, paused: bool) {
        self.inner.ctl.set_paused(paused);
    }

    pub fn is_paused(&self) -> bool {
        self.inner.ctl.is_paused()
    }

    pub fn progress(&self) -> Progress {
        self.inner.ctl.snapshot()
    }

    pub fn generation(&self) -> Generation {
        Generation {
            scans: self.inner.scans_done.load(Ordering::Relaxed),
            imports: self.inner.imports_done.load(Ordering::Relaxed),
        }
    }

    /// Rounds finished so far, for the app to count them as stages.
    pub fn rounds(&self) -> u64 {
        self.inner.rounds.load(Ordering::Relaxed)
    }

    /// The queue has a new request: stop the round in progress at its next file, and start one.
    fn wake_up(&self) {
        self.inner.ctl.request_stop();
        self.service.wake();
    }
}

impl Inner {
    /// The queue of requests. A holder that panicked left it whole (a request is pushed or popped, or not), so it is
    /// used as it is.
    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn notify(&self, notice: Notice) {
        self.notices.post(notice);
    }

    fn clear(&self, key: Key<'_>) {
        self.notices.clear(key.to_buf());
    }

    /// A pass couldn't read `unreadable` of its files (they are tried again by its next pass): what its kept notice says,
    /// gone once a complete pass read them all.
    fn files_read(&self, pass: FilePass, unreadable: usize, complete: bool) {
        if unreadable > 0 {
            self.notify(Notice::UnreadableFiles { pass, count: count(unreadable) });
        } else if complete {
            self.clear(Key { code: "unreadable-files", root: None, part: Some(Part::Code(pass.code())) });
        }
    }

    /// A pass wrote its failures to the ledger, or couldn't (`error`).
    fn ledger_written(&self, pass: FilePass, error: Option<String>) {
        match error {
            Some(error) => self.notify(Notice::LedgerNotWritten { pass, error: tidy(&error, ERROR_CAP) }),
            None => self.clear(Key { code: "ledger-not-written", root: None, part: Some(Part::Code(pass.code())) }),
        }
    }

    fn scanned(&self) {
        self.scans_done.fetch_add(1, Ordering::Relaxed);
        self.changes.publish(Change::Scanned);
    }

    fn imported(&self) {
        self.imports_done.fetch_add(1, Ordering::Relaxed);
        self.changes.publish(Change::Imported);
    }

    fn run(&self, waker: &Waker<'_>) {
        // One connection for every round; after a round that failed, a new one.
        let mut catalog: Option<Catalog> = None;
        // A round stopped by a new request starts the next one at once: the request is already queued.
        let mut again = false;
        loop {
            if !again && waker.wait(None) != Woke::Work {
                return;
            }
            again = false;
            // The stop the last request set is cleared here, then the quit is looked for: a quit asked after this
            // check comes with a stop after the clear, so the round stops.
            self.ctl.clear_stop();
            if waker.quitting() {
                return;
            }
            match self.round_caught(&mut catalog) {
                Ok(step) => {
                    // A round that ran is the end of a failed one's notice.
                    self.clear(Key::all("round-failed", None));
                    again = matches!(step, Step::Preempted);
                }
                Err(notice) => self.notify(notice),
            }
            self.rounds.fetch_add(1, Ordering::Relaxed);
            self.changes.publish(Change::Round);
        }
    }

    /// A round, with a panic in it caught: the job reports it (the inbox, and the log through the panic hook)
    /// and goes on with the next request.
    fn round_caught(&self, catalog: &mut Option<Catalog>) -> std::result::Result<Step, Notice> {
        match panics::catch(AssertUnwindSafe(|| self.round(catalog))) {
            Ok(result) => result.map_err(|e| Notice::RoundFailed { error: error_text(&e) }),
            Err(panic) => {
                // The connection a panicking pass held is not trusted: the next round opens a new one.
                *catalog = None;
                Err(Notice::RoundPanicked { message: tidy(panics::message(&*panic), ERROR_CAP) })
            }
        }
    }

    /// One round: the passes in order, on the connection the job keeps between rounds.
    fn round(&self, conn: &mut Option<Catalog>) -> Result<Step> {
        let mut catalog = match conn.take() {
            Some(c) => c,
            None => Catalog::open(&self.catalog_path, pg_platform::native::PATH_RULES)?,
        };
        let step = {
            let mut cx = Cx { inner: self, catalog: &mut catalog };
            schedule::round(&mut [&mut Crops, &mut Scans, &mut Import, &mut Borders, &mut Thumbs], &mut cx, |cx| {
                cx.inner.ctl.stop_requested()
            })?
        };
        *conn = Some(catalog);
        Ok(step)
    }
}

/// What the passes of a round work with: the job, and the round's connection.
struct Cx<'a> {
    inner: &'a Inner,
    catalog: &'a mut Catalog,
}

/// Saves the cropped copies asked for, one request at a time. A stopped save keeps its rest at the front of the queue.
struct Crops;

impl Pass<Cx<'_>> for Crops {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        loop {
            let Some((ids, folder)) = cx.inner.queue().crops.pop_front() else {
                return Ok(Step::Done);
            };
            match pg_crop::save(cx.catalog, &ids, &folder, &Options::new(Priority::BelowNormal), &cx.inner.ctl) {
                Ok(s) if !s.remaining.is_empty() => {
                    cx.inner.queue().crops.push_front((s.remaining, folder));
                    return Ok(Step::Preempted);
                }
                Ok(s) => cx.inner.notify(Notice::Cropped {
                    folder: folder.display().to_string(),
                    saved: count(s.saved),
                    skipped: count(s.skipped),
                    failed: count(s.failed),
                    first_error: s.error.map(file_failure),
                }),
                Err(e) => cx.inner.notify(Notice::CropFailed { error: error_text(&e) }),
            }
        }
    }
}

/// Scans the folders asked for, skipping those no longer in the library. A scan stopped early goes back to the front.
struct Scans;

impl Pass<Cx<'_>> for Scans {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        let rules = cx.catalog.rules();
        let library: Vec<RulePath> = cx.catalog.roots()?.into_iter().map(|r| r.path).collect();
        loop {
            let Some(path) = cx.inner.queue().scans.pop_front() else {
                return Ok(Step::Done);
            };
            // A folder asked for that is no longer in the library is skipped.
            if !library.iter().any(|l| rules.is_under(&path, l)) {
                continue;
            }
            *cx.inner.scanning.lock().unwrap_or_else(PoisonError::into_inner) = Some(path.clone());
            let scanned = pg_scan::scan(cx.catalog, &path, &cx.inner.ctl);
            *cx.inner.scanning.lock().unwrap_or_else(PoisonError::into_inner) = None;
            match scanned {
                Ok(s) => {
                    // A scan stopped early has committed what it recorded so far.
                    if s.changed > 0 || s.missing > 0 || s.rebound > 0 || s.replaced > 0 {
                        cx.inner.scanned();
                    }
                    // What the pairing replaced loses its thumbnails and the temp files of its queued writes (decision
                    // 7). Both come after the catalog's commit, which the scan made.
                    for (_, e) in scans::forget_replaced(cx.inner.thumbs, &s.replaced_ids) {
                        cx.inner.notify(Notice::ScanFailed { root: path.clone(), error: error_text(&e) });
                    }
                    for (temp, e) in pg_writeback::discard_temps(rules, &s.temps) {
                        cx.inner.notify(Notice::ScanFailed {
                            root: path.clone(),
                            error: tidy(&format!("can't delete {}: {e}", rules.display(&temp)), ERROR_CAP),
                        });
                    }
                    if !s.complete {
                        cx.inner.queue().scans.push_front(path);
                        return Ok(Step::Preempted);
                    }
                    let found = scans::scan_notices(&path, &s);
                    // A complete walk of a whole library folder is what holds for it now: its kept notices are
                    // replaced by the ones it found. A walk of a folder under it adds its own and clears nothing (the
                    // rest of the library folder wasn't looked at).
                    if library.contains(&path) {
                        for code in SCAN_STATES {
                            let of_code = found.iter().filter(|n| n.code() == code).cloned().collect();
                            cx.inner.notices.replace(Key::all(code, Some(&path)).to_buf(), of_code);
                        }
                    } else {
                        for notice in found {
                            cx.inner.notify(notice);
                        }
                    }
                }
                Err(e) => {
                    let notice = match e {
                        ScanError::Refused(reason) => Notice::RootRefused { root: path, reason },
                        ScanError::Failed(e) => Notice::ScanFailed { root: path, error: error_text(&e) },
                    };
                    cx.inner.notify(notice);
                    // Its batches before the error are in.
                    cx.inner.scanned();
                }
            }
        }
    }
}

/// The kept notices a scan of a library folder posts, which a complete walk of the whole folder replaces.
const SCAN_STATES: [&str; 4] = ["scan-unreadable", "root-refused", "mount-refused", "scan-note"];
/// Analyzes what the scans found.
struct Import;

impl Pass<Cx<'_>> for Import {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        let s = pg_import::import(cx.catalog, &Options::new(Priority::BelowNormal), false, &cx.inner.ctl)?;
        if s.files > 0 {
            cx.inner.imported();
        }
        Ok(Step::Done)
    }
}

/// Looks for the borders of what was imported. Its own pass, after the import: the import's decode is reduced, the
/// detector wants full size.
struct Borders;

impl Pass<Cx<'_>> for Borders {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        let s = pg_borders::find(cx.catalog, &Options::new(Priority::BelowNormal), &cx.inner.ctl)?;
        // Only what went into the catalog changes what the app shows: files that couldn't be read stored nothing.
        if s.stored > 0 {
            cx.inner.imported();
        }
        cx.inner.files_read(FilePass::Borders, s.unreadable, s.complete);
        cx.inner.ledger_written(FilePass::Borders, s.ledger_error);
        Ok(Step::Done)
    }
}

/// Makes the thumbnails still missing at the grid's current height.
struct Thumbs;

impl Pass<Cx<'_>> for Thumbs {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        let height = cx.inner.thumb_height.load(Ordering::Relaxed);
        let opts = Options::new(Priority::Idle);
        let s = pg_thumbs::run(cx.inner.thumbs, &cx.inner.catalog_path, height, &opts, &cx.inner.ctl)?;
        if s.cache_full {
            cx.inner.notify(Notice::CacheFull { height, made: count(s.made) });
        } else if s.complete {
            cx.inner.clear(Key::all("cache-full", None));
        }
        cx.inner.files_read(FilePass::Thumbs, s.unreadable, s.complete);
        cx.inner.ledger_written(FilePass::Thumbs, s.ledger_error);
        Ok(Step::Done)
    }
}
