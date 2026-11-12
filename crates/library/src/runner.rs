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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use anyhow::{Context, Result};
use pg_catalog::Catalog;
use pg_jobs::NoticeSink;
use pg_jobs::control::Control;
use pg_jobs::events::{Bus, Change};
use pg_jobs::panics;
use pg_jobs::pipeline::Options;
use pg_jobs::schedule::{self, Pass, Step};
use pg_jobs::service::{Service, Waker, Woke};
use pg_model::ids::FileId;
use pg_model::notice::{ERROR_CAP, FileFailure, FilePass, Key, Notice, Part, tidy};
use pg_model::progress::Progress;
use pg_model::rules::Path as RulePath;
use pg_model::size::ThumbHeight;
use pg_platform::priority::Priority;
use pg_scan::ScanError;
use pg_thumbcache::ThumbCache;
use pg_thumbs::LedgerOutcome;

use crate::scans;

/// How long a walk refused by another walk's lease (an overlapping folder, in this process or another) waits before it
/// is tried again, with nothing else to wake the job: a lease ends with its walk, or expires in
/// `pg_catalog::SCAN_LEASE_SECS`.
const BUSY_RETRY: Duration = Duration::from_secs(5);

/// The thumbnail height the grid asks for. It is held here, not given to the job at its start, so the height may be set
/// before the job starts (the window's scale arrives after the library is made). The thumbnail pass makes thumbnails only
/// once a height is set, and a change stops the pass in progress and wakes the job.
#[derive(Clone, Default)]
pub struct HeightCell(Arc<Mutex<HeightState>>);

#[derive(Default)]
struct HeightState {
    height: Option<ThumbHeight>,
    /// Run on a change, once a job listens (`HeightCell::listen`).
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl HeightCell {
    /// The height, once one has been set.
    pub fn get(&self) -> Option<ThumbHeight> {
        self.state().height
    }

    /// Set the height. A change to a different height wakes the job, if one listens.
    pub fn set(&self, height: ThumbHeight) {
        let wake = {
            let mut state = self.state();
            if state.height == Some(height) {
                return;
            }
            state.height = Some(height);
            state.wake.clone()
        };
        if let Some(wake) = wake {
            wake();
        }
    }

    /// Run `wake` on each change from now on (the job's own, installed at its start).
    fn listen(&self, wake: impl Fn() + Send + Sync + 'static) {
        self.state().wake = Some(Arc::new(wake));
    }

    fn state(&self) -> MutexGuard<'_, HeightState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

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
    /// The scans refused because another walk of an overlapping folder held its lease: back in the queue at the next
    /// round, which comes by [`BUSY_RETRY`] at the latest.
    busy: Mutex<Vec<RulePath>>,
    heights: HeightCell,
    scans_done: AtomicU64,
    imports_done: AtomicU64,
    /// Rounds finished, however they ended: the app marks each one as a stage for measuring (docs/PROFILING.md).
    rounds: AtomicU64,
    notices: Arc<dyn NoticeSink>,
    /// Told what changed (`Change`), so the owner reads the new state instead of polling for it.
    changes: Arc<Bus<Change>>,
}

/// The notice of a crop job's file failure: its error capped as a notice holds it.
fn file_failure(failure: FileFailure) -> FileFailure {
    FileFailure { error: tidy(&failure.error, ERROR_CAP), ..failure }
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
    service: Arc<Service>,
}

impl Runner {
    /// Start the job thread. It begins by importing whatever is pending, and makes the thumbnails still missing at the
    /// height `heights` holds (none until one is set). What it changes is published on `changes` (`Change`), and what it
    /// tells the user goes to `notices`, which is poked whenever a pass changes the failure ledger
    /// (`Control::ledger_changed`). An error if the thread can't be spawned.
    pub fn start(
        catalog_path: PathBuf,
        thumbs: &'static ThumbCache,
        heights: HeightCell,
        changes: Arc<Bus<Change>>,
        notices: Arc<dyn NoticeSink>,
    ) -> Result<Runner> {
        let inner = Arc::new(Inner {
            catalog_path,
            thumbs,
            ctl: Control::new(),
            queue: Mutex::new(Queue::default()),
            scanning: Mutex::new(None),
            busy: Mutex::new(Vec::new()),
            heights: heights.clone(),
            scans_done: AtomicU64::new(0),
            imports_done: AtomicU64::new(0),
            rounds: AtomicU64::new(0),
            notices,
            changes,
        });
        let announce = inner.changes.clone();
        inner.ctl.on_change(move || announce.publish(Change::Progress));
        // The inbox reads the ledger again when a pass changed it, within the pass, not only at the round's end.
        let ledger = inner.notices.clone();
        inner.ctl.on_ledger_change(move || ledger.poke());
        let job = inner.clone();
        let service = Arc::new(
            Service::start("background job", move |waker| job.run(waker)).context("start the background job")?,
        );
        // A new height stops the thumbnail pass in progress (it makes the rest at the new height) and wakes the job. Weak
        // references: the cell (held by the library) must not keep the job alive.
        let (inner_weak, service_weak): (Weak<Inner>, Weak<Service>) =
            (Arc::downgrade(&inner), Arc::downgrade(&service));
        heights.listen(move || {
            if let Some(inner) = inner_weak.upgrade() {
                inner.ctl.request_stop();
            }
            if let Some(service) = service_weak.upgrade() {
                service.wake();
            }
        });
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
        self.inner.busy().retain(|path| !rules.is_under(path, root));
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

    /// The scans refused by another walk's lease, waiting for their retry.
    fn busy(&self) -> MutexGuard<'_, Vec<RulePath>> {
        self.busy.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The walk of `path` was refused by another walk's lease: it is tried again later ([`BUSY_RETRY`]).
    fn walk_busy(&self, path: RulePath) {
        let mut busy = self.busy();
        if !busy.contains(&path) {
            busy.push(path);
        }
    }

    /// The scans refused by a lease go back to the end of the queue, for this round to try again.
    fn requeue_busy(&self) {
        let busy = std::mem::take(&mut *self.busy());
        let mut q = self.queue();
        for path in busy {
            if !q.scans.contains(&path) {
                q.scans.push_back(path);
            }
        }
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

    /// What a pass's writes to the failure ledger came to: a failure is said; a write that went through clears what an
    /// earlier failure said; a pass that wrote nothing says nothing (it is no success of the ledger's write).
    fn ledger_written(&self, pass: FilePass, outcome: LedgerOutcome) {
        match outcome {
            LedgerOutcome::Failed(error) => {
                self.notify(Notice::LedgerNotWritten { pass, error: tidy(&error, ERROR_CAP) });
            }
            LedgerOutcome::Written => self.ledger_not_written_gone(pass),
            LedgerOutcome::Untouched => {}
        }
    }

    /// `pass`'s ledger was written: its kept `ledger-not-written` goes.
    fn ledger_not_written_gone(&self, pass: FilePass) {
        self.clear(Key { code: "ledger-not-written", root: None, part: Some(Part::Code(pass.code())) });
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
            if !again {
                // A walk refused by another's lease is tried again after a while, with nothing else to wake the job.
                let idle = (!self.busy().is_empty()).then_some(BUSY_RETRY);
                if waker.wait(idle) == Woke::Quit {
                    return;
                }
            }
            again = false;
            // The stop the last request set is cleared here, then the quit is looked for: a quit asked after this
            // check comes with a stop after the clear, so the round stops.
            self.ctl.clear_stop();
            if waker.quitting() {
                return;
            }
            self.requeue_busy();
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
                    if s.changed > 0 || s.missing > 0 || s.pairing.rebound > 0 || s.pairing.replaced > 0 {
                        cx.inner.scanned();
                    }
                    // What the pairing replaced loses its thumbnails and the temp files of its queued writes (decision
                    // 7). Both come after the catalog's commit, which the scan made.
                    for (_, e) in scans::forget_replaced(cx.inner.thumbs, &s.pairing.replaced_ids) {
                        cx.inner.notify(Notice::ScanFailed { root: path.clone(), error: error_text(&e) });
                    }
                    for (temp, e) in pg_writeback::discard_temps(rules, &s.pairing.temps) {
                        cx.inner.notify(Notice::ScanFailed {
                            root: path.clone(),
                            error: tidy(&format!("can't delete {}: {e}", rules.display(&temp)), ERROR_CAP),
                        });
                    }
                    let found = scans::scan_notices(&path, &s);
                    // What the walk did is said whatever folder it walked, and stopped or not: the pairing's binds and
                    // replacements are in, and the walk that resumes doesn't do them again.
                    for notice in found.events {
                        cx.inner.notify(notice);
                    }
                    if !s.complete {
                        cx.inner.queue().scans.push_front(path);
                        return Ok(Step::Preempted);
                    }
                    // A complete walk of a whole library folder is what holds for it now: its kept notes (the folder's
                    // conditions) are replaced by the ones it found. A walk of a folder under it looked at part of the
                    // folder only, so it changes none of them. The folders it couldn't read are the folder ledger's,
                    // which the walk itself recorded and swept.
                    if library.contains(&path) {
                        cx.inner.notices.replace(Key::all("scan-note", Some(&path)).to_buf(), found.conditions);
                    }
                }
                // Refused before any walk: the refusal is in the folder ledger (`pg_scan::scan` recorded it, and poked
                // the inbox), until a walk of the folder can run.
                Err(ScanError::Refused(_)) => {}
                // Another walk of an overlapping folder holds its lease (in this process or another): nothing was
                // written, and the scan is tried again once that walk can have ended.
                Err(ScanError::Busy { .. }) => cx.inner.walk_busy(path),
                Err(ScanError::Failed(e)) => {
                    cx.inner.notify(Notice::ScanFailed { root: path, error: error_text(&e) });
                    // Its batches before the error are in.
                    cx.inner.scanned();
                }
            }
        }
    }
}

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
        // The borders' ledger rows are written in the transaction of the borders they go with (`pg_borders::find`): a
        // failure to write them fails the batch, which is the round's failure, not a ledger's of its own. A batch that
        // went in wrote its rows, which resolves a `ledger-not-written` an earlier build kept for the borders.
        if s.files > 0 {
            cx.inner.ledger_not_written_gone(FilePass::Borders);
        }
        Ok(Step::Done)
    }
}

/// Makes the thumbnails still missing at the grid's current height.
struct Thumbs;

impl Pass<Cx<'_>> for Thumbs {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        // No height yet: the grid has not said which thumbnails it wants, so there is nothing to make.
        let Some(height) = cx.inner.heights.get() else {
            return Ok(Step::Done);
        };
        let height = height.get();
        let opts = Options::new(Priority::Idle);
        let s = pg_thumbs::run(cx.inner.thumbs, &cx.inner.catalog_path, height, &opts, &cx.inner.ctl)?;
        if s.cache_full {
            cx.inner.notify(Notice::CacheFull { height, made: count(s.made) });
        } else if s.complete {
            cx.inner.clear(Key::all("cache-full", None));
        }
        cx.inner.files_read(FilePass::Thumbs, s.unreadable, s.complete);
        cx.inner.ledger_written(FilePass::Thumbs, s.ledger);
        Ok(Step::Done)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Instant;

    use pg_jobs_testkit::{Call, Quiet, RecordingSink};
    use pg_model::notice::{KeyBuf, ScanNote};
    use pg_platform::native::PATH_RULES;

    use super::*;

    /// A height cell set to 64 pixels, as the grid asks at the start.
    fn height_64() -> HeightCell {
        let heights = HeightCell::default();
        heights.set(ThumbHeight::new(64).expect("a height in range"));
        heights
    }

    /// A height is held before anything listens, and each change to a different height wakes the job once; the same height
    /// again wakes nothing.
    #[test]
    fn a_height_is_held_and_each_change_wakes_the_job_once() {
        let heights = HeightCell::default();
        assert_eq!(heights.get(), None, "no height until the grid asks");
        let woke = Arc::new(AtomicU64::new(0));
        let counter = woke.clone();
        heights.listen(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let at = |px| ThumbHeight::new(px).expect("a height in range");
        heights.set(at(220));
        heights.set(at(220));
        heights.set(at(440));
        assert_eq!(heights.get(), Some(at(440)));
        assert_eq!(woke.load(Ordering::Relaxed), 2);
    }

    /// A library folder `images` in a run folder, holding `names` (small PNGs), scanned and imported into the catalog
    /// beside it: the run folder, the catalog's path, and the library folder.
    fn library(prefix: &str, names: &[&str]) -> (PathBuf, PathBuf, RulePath) {
        let dir = pg_testkit::run_folder_with("scratch_runner", prefix, &["images"]).unwrap();
        for name in names {
            std::fs::write(dir.join("images").join(name), pg_testkit::png_rgb(8, 8, &[128; 192]).unwrap()).unwrap();
        }
        let catalog_path = dir.join("catalog.sqlite");
        let mut c = Catalog::prepare(&catalog_path, crate::versions(), PATH_RULES).unwrap();
        let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
        assert!(c.add_root(&root).unwrap());
        pg_scan::scan(&mut c, &root, &Control::new()).unwrap();
        pg_import::import(&mut c, &Options::new(Priority::Normal), false, &Control::new()).unwrap();
        (dir, catalog_path, root)
    }

    /// The thumbnail cache beside the catalog, holding at most `cap` bytes (`None`: the default), leaked as the job's
    /// is, with the watch on its events.
    fn thumbs(dir: &Path, catalog_path: &Path, cap: Option<i64>) -> (&'static ThumbCache, Quiet<pg_thumbcache::Event>) {
        let id = Catalog::open(catalog_path, PATH_RULES).unwrap().catalog_id().unwrap();
        let (events, quiet) = Quiet::bus();
        let cap = cap.and_then(pg_thumbcache::Cap::from_bytes);
        let cache = ThumbCache::open(&dir.join("thumbs.sqlite"), &id, cap, pg_render::THUMBS_VERSION, events).unwrap();
        (Box::leak(Box::new(cache)), quiet)
    }

    /// The job's state without its thread: the test runs its passes one by one.
    fn job(catalog_path: &Path, thumbs: &'static ThumbCache, sink: Arc<RecordingSink>) -> Inner {
        Inner {
            catalog_path: catalog_path.to_path_buf(),
            thumbs,
            ctl: Control::new(),
            queue: Mutex::new(Queue::default()),
            scanning: Mutex::new(None),
            busy: Mutex::new(Vec::new()),
            heights: height_64(),
            scans_done: AtomicU64::new(0),
            imports_done: AtomicU64::new(0),
            rounds: AtomicU64::new(0),
            notices: sink,
            changes: Arc::new(Bus::new(16)),
        }
    }

    /// Run `pass` once on a connection of its own, as a round does.
    fn run(pass: &mut dyn for<'a> Pass<Cx<'a>>, inner: &Inner) -> Step {
        let mut catalog = Catalog::open(&inner.catalog_path, PATH_RULES).unwrap();
        pass.run(&mut Cx { inner, catalog: &mut catalog }).unwrap()
    }

    /// Wait (30 s at most) until `done`.
    fn wait_for(what: &str, done: impl Fn() -> bool) {
        let began = Instant::now();
        while !done() {
            assert!(began.elapsed() < Duration::from_secs(30), "waited 30 s for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn kept(code: &'static str, part: Option<&'static str>) -> KeyBuf {
        KeyBuf { code, root: None, part: part.map(Part::Code) }
    }

    /// A file that can't be read (here, gone since the import) is said, not recorded in the ledger; once nothing is
    /// left to read, the next complete pass (its early return) clears what was said.
    #[test]
    fn an_offline_file_is_said_not_recorded_and_a_complete_pass_clears_it() {
        let (dir, catalog_path, root) = library("runner-offline", &["a.png", "b.png"]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache, sink.clone());
        std::fs::remove_file(dir.join("images").join("b.png")).unwrap();
        run(&mut Thumbs, &inner);
        assert_eq!(sink.posted(), [Notice::UnreadableFiles { pass: FilePass::Thumbs, count: 1 }]);
        let c = Catalog::open(&catalog_path, PATH_RULES).unwrap();
        assert!(c.failed_files(FilePass::Thumbs, 10).unwrap().is_empty(), "an unreadable file is in no ledger");
        // The rescan finds it missing: the next pass has nothing to make, and is complete.
        pg_scan::scan(&mut Catalog::open(&catalog_path, PATH_RULES).unwrap(), &root, &Control::new()).unwrap();
        sink.take();
        run(&mut Thumbs, &inner);
        assert!(sink.posted().is_empty(), "{:?}", sink.calls());
        assert!(sink.calls().contains(&Call::Clear(kept("unreadable-files", Some("thumbs")))), "{:?}", sink.calls());
    }

    /// A cache too small for one thumbnail is said; a complete pass (here, once its file left the library) clears it.
    #[test]
    fn a_full_cache_is_said_and_a_complete_pass_clears_it() {
        let (dir, catalog_path, root) = library("runner-full", &["a.png"]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, Some(1));
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache, sink.clone());
        run(&mut Thumbs, &inner);
        assert_eq!(sink.posted(), [Notice::CacheFull { height: 64, made: 0 }]);
        std::fs::remove_file(dir.join("images").join("a.png")).unwrap();
        pg_scan::scan(&mut Catalog::open(&catalog_path, PATH_RULES).unwrap(), &root, &Control::new()).unwrap();
        sink.take();
        run(&mut Thumbs, &inner);
        assert!(sink.calls().contains(&Call::Clear(kept("cache-full", None))), "{:?}", sink.calls());
    }

    /// A round that fails (its catalog can't be opened) is said; the next round that runs clears it.
    #[test]
    fn a_failed_round_is_said_and_the_next_round_clears_it() {
        let (dir, catalog_path, _root) = library("runner-round", &[]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let aside = dir.join("aside.sqlite");
        std::fs::rename(&catalog_path, &aside).unwrap();
        let sink = RecordingSink::new();
        let runner =
            Runner::start(catalog_path.clone(), cache, height_64(), Arc::new(Bus::new(16)), sink.clone()).unwrap();
        wait_for("the round's failure", || sink.posted().iter().any(|n| matches!(n, Notice::RoundFailed { .. })));
        std::fs::rename(&aside, &catalog_path).unwrap();
        runner.wake();
        wait_for("the failure cleared", || sink.calls().contains(&Call::Clear(kept("round-failed", None))));
        assert!(runner.stop(Duration::from_secs(30)));
    }

    /// The scan notes split by what they describe (`ScanNote::lifecycle`): a complete walk of the library folder
    /// replaces the folder's conditions with what it found (here, a second path to a file: a hard link); a walk of a
    /// folder inside it says nothing of them.
    #[test]
    fn only_a_walk_of_the_whole_library_folder_replaces_its_conditions() {
        let (dir, catalog_path, root) = library("runner-notes", &["a.png"]);
        let images = dir.join("images");
        std::fs::create_dir_all(images.join("sub")).unwrap();
        if std::fs::hard_link(images.join("a.png"), images.join("sub").join("b.png")).is_err() {
            pg_testkit::skip("this file system makes no hard links");
            return;
        }
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache, sink.clone());
        let sub = pg_platform::names::normalize_path(&images.join("sub")).unwrap();
        inner.queue().scans.extend([root.clone(), sub]);
        assert_eq!(run(&mut Scans, &inner), Step::Done);
        let duplicates = Notice::ScanNote { root: root.clone(), note: ScanNote::Duplicates, count: 1 };
        assert_eq!(sink.take(), [Call::Replace(Key::all("scan-note", Some(&root)).to_buf(), vec![duplicates])]);
    }

    /// A walk refused by another's lease posts nothing, waits apart, and runs once it is put back after the lease ends.
    #[test]
    fn a_walk_refused_by_a_lease_is_tried_again_after_it() {
        let (dir, catalog_path, root) = library("runner-busy", &["a.png"]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache, sink.clone());
        let mut holder = Catalog::open(&catalog_path, PATH_RULES).unwrap();
        let lease = holder.begin_scan(&root).unwrap();
        inner.queue().scans.push_back(root.clone());
        assert_eq!(run(&mut Scans, &inner), Step::Done);
        assert_eq!(*inner.busy(), std::slice::from_ref(&root));
        assert!(sink.calls().is_empty(), "{:?}", sink.calls());
        drop(lease);
        inner.requeue_busy();
        assert!(inner.busy().is_empty());
        assert_eq!(run(&mut Scans, &inner), Step::Done);
        assert_eq!(sink.take(), [Call::Replace(Key::all("scan-note", Some(&root)).to_buf(), Vec::new())]);
    }

    /// A library folder the scan refuses (here, gone since it was added: its facts can't be read) is the folder
    /// ledger's, not a failed scan: nothing is posted, and the refusal is a row of the ledger.
    #[test]
    fn a_refused_folder_is_in_the_folder_ledger_not_a_failed_scan() {
        let (dir, catalog_path, root) = library("runner-refused", &["a.png"]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache, sink.clone());
        std::fs::remove_dir_all(dir.join("images")).unwrap();
        inner.queue().scans.push_back(root.clone());
        assert_eq!(run(&mut Scans, &inner), Step::Done);
        assert!(sink.posted().is_empty(), "{:?}", sink.calls());
        let c = Catalog::open(&catalog_path, PATH_RULES).unwrap();
        let failed = c.failed_folders(10).unwrap();
        assert_eq!(
            failed.iter().map(|f| (&f.path, f.failure)).collect::<Vec<_>>(),
            [(&root, pg_catalog::FolderFailure::Refused)]
        );
    }

    /// The thumbnail pass's ledger outcome: a failure is said, a write clears what was said, and nothing written says
    /// nothing.
    #[test]
    fn a_ledger_outcome_is_said_cleared_or_left() {
        let (dir, catalog_path, _root) = library("runner-ledger", &[]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache, sink.clone());
        inner.ledger_written(FilePass::Thumbs, LedgerOutcome::Untouched);
        assert!(sink.take().is_empty());
        inner.ledger_written(FilePass::Thumbs, LedgerOutcome::Failed("disk full".into()));
        assert_eq!(
            sink.take(),
            [Call::Post(Notice::LedgerNotWritten { pass: FilePass::Thumbs, error: "disk full".into() })]
        );
        inner.ledger_written(FilePass::Thumbs, LedgerOutcome::Written);
        assert_eq!(sink.take(), [Call::Clear(kept("ledger-not-written", Some("thumbs")))]);
    }

    /// The job pokes its sink when a pass says the ledger changed (`Control::ledger_changed`).
    #[test]
    fn a_ledger_change_pokes_the_sink() {
        let (dir, catalog_path, _root) = library("runner-poke", &[]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let runner =
            Runner::start(catalog_path.clone(), cache, height_64(), Arc::new(Bus::new(16)), sink.clone()).unwrap();
        let before = sink.pokes();
        runner.inner.ctl.ledger_changed();
        assert_eq!(sink.pokes(), before + 1);
        assert!(runner.stop(Duration::from_secs(30)));
    }
}
