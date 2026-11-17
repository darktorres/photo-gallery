//! The app's background job: one thread that saves the cropped copies asked for, scans the folders
//! asked for, imports whatever is pending, makes thumbnails at the grid's current height, makes the duplicate hashes the
//! import left out, pairs the duplicates, and looks for the borders of what it imported (and of everything after a new
//! detector), then sleeps until there is something new to do. The round's order is that one; the thumbnails come before
//! the duplicates and the borders, which are the long ones.
//!
//! The thread is a `pg_jobs::service::Service`, and a round is a schedule of passes over the queue of requests. A new
//! request (a folder added, a rescan) stops the pass in progress at the next file; a new thumbnail height or a feature
//! turned on or off does not stop a round, but the pass it concerns ends after its batch (each watches the value it
//! runs for). It commits what it has and the next round starts with the new work. Every pass is resumable from the
//! catalog, so stopping costs nothing but the batch in flight.

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
use pg_model::features::Features;
use pg_model::ids::FileId;
use pg_model::notice::{ERROR_CAP, FileFailure, FilePass, Key, Notice, Op, Trigger, tidy};
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
/// once a height is set. A change wakes the job, and the thumbnail pass in progress ends after its batch (it watches the
/// height it runs for, `run`); a round is not stopped.
#[derive(Clone, Default)]
pub struct HeightCell(Arc<Mutex<HeightState>>);

/// The background features the owner has on (`Features`, from the preferences), held here so a change reaches the job
/// without a restart. A change wakes the job; it stops nothing. Each pass a feature gates reads it when it starts, and
/// ends after its batch when its feature is turned off (`still` in each pass).
#[derive(Clone)]
pub struct FeatureCell(Arc<Mutex<FeatureState>>);

struct FeatureState {
    features: Features,
    /// Run on a change, once a job listens (`FeatureCell::listen`).
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl FeatureCell {
    pub fn new(features: Features) -> FeatureCell {
        FeatureCell(Arc::new(Mutex::new(FeatureState { features, wake: None })))
    }

    /// The features, as they are now.
    pub fn get(&self) -> Features {
        self.state().features
    }

    /// Set the features. A change wakes the job, if one listens.
    pub fn set(&self, features: Features) {
        let wake = {
            let mut state = self.state();
            if state.features == features {
                return;
            }
            state.features = features;
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

    fn state(&self) -> MutexGuard<'_, FeatureState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

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
    /// Borders to cut off the originals: the files. Queued only once confirmed (`Runner::cut_borders`).
    cuts: VecDeque<Vec<FileId>>,
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
    features: FeatureCell,
    scans_done: AtomicU64,
    imports_done: AtomicU64,
    /// Rounds finished, however they ended: the app marks each one as a stage for measuring.
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
    /// height `heights` holds (none until one is set). `features` says which background work is on (the borders, the
    /// duplicates). What it changes is published on `changes` (`Change`), and what it tells the user goes to `notices`,
    /// which is poked whenever a pass changes the failure ledger (`Control::ledger_changed`). An error if the thread
    /// can't be spawned.
    pub fn start(
        catalog_path: PathBuf,
        thumbs: &'static ThumbCache,
        heights: HeightCell,
        features: FeatureCell,
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
            features: features.clone(),
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
        // A new height or a change of features wakes the job; the passes they concern end after their batch, and the next
        // round reads the new values. Weak references: the cells (held by the library) must not keep the job alive.
        let service_weak: Weak<Service> = Arc::downgrade(&service);
        let wake_service = service_weak.clone();
        heights.listen(move || {
            if let Some(service) = wake_service.upgrade() {
                service.wake();
            }
        });
        features.listen(move || {
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

    /// A library folder was removed: the scans asked for under `root` are dropped, and one walking a folder under it is
    /// woken. That walk writes nothing more: the removal took its lease (`Catalog::remove_root`), so its next write
    /// ends it as `ScanError::Removed`, and it isn't asked for again, as the folder is no longer in the library.
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

    /// Cut the borders off the originals of these files, in place (`pg_crop::cut`), after the copies asked for. Nothing
    /// is queued unless `confirmed`: the originals change, and only the user's confirmation says they may.
    pub fn cut_borders(&self, ids: Vec<FileId>, confirmed: bool) {
        if !confirmed {
            return;
        }
        self.inner.queue().cuts.push_back(ids);
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

    /// What happened that clears the kept notices its trigger names (see [`Trigger`]).
    fn resolve(&self, trigger: Trigger) {
        self.notices.resolve(trigger);
    }

    /// A pass couldn't read `unreadable` of its files (they are tried again by its next pass): what its kept notice says,
    /// gone once a complete pass read them all.
    fn files_read(&self, pass: FilePass, unreadable: usize, complete: bool) {
        if unreadable > 0 {
            self.notify(Notice::UnreadableFiles { pass, count: count(unreadable) });
        } else if complete {
            self.resolve(Trigger::Pass(pass));
        }
    }

    /// What a pass's writes to the failure ledger came to: a failure is said; a write that went through clears what an
    /// earlier failure said; a pass that wrote nothing says nothing (it is no success of the ledger's write).
    fn ledger_written(&self, outcome: LedgerOutcome) {
        match outcome {
            LedgerOutcome::Failed(error) => {
                self.notify(Notice::LedgerNotWritten { error: tidy(&error, ERROR_CAP) });
            }
            LedgerOutcome::Written => self.ledger_not_written_gone(),
            LedgerOutcome::Untouched => {}
        }
    }

    /// The thumbnails' ledger was written: a `ledger-not-written` kept for an earlier failure is no longer true.
    fn ledger_not_written_gone(&self) {
        self.resolve(Trigger::Op(Op::LedgerWrite(FilePass::Thumbs)));
    }

    /// Originals were cut in place: their thumbnails are forgotten (they show the uncut pixels), and the import that
    /// follows analyzes them again. The app is told the files changed.
    fn cut_done(&self, cut: &[FileId]) {
        if cut.is_empty() {
            return;
        }
        for (_, e) in scans::forget_replaced(self.thumbs, cut) {
            self.notify(Notice::CutFailed { error: error_text(&e) });
        }
        self.imported();
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
                    self.resolve(Trigger::Op(Op::Round));
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
        // The round's span in the perf log: the passes it runs are its children, and the catalog's open is timed apart.
        let _round = pg_perf::span("round");
        let open = pg_perf::span("round.open");
        let mut catalog = match conn.take() {
            Some(c) => c,
            None => Catalog::open(&self.catalog_path, pg_platform::native::PATH_RULES)?,
        };
        drop(open);
        let step = {
            let mut cx = Cx { inner: self, catalog: &mut catalog };
            schedule::round(
                &mut [
                    &mut Crops,
                    &mut Cuts,
                    &mut Scans,
                    &mut Import,
                    &mut Thumbs,
                    &mut Placeholders,
                    &mut Hashes,
                    &mut Pairing,
                    &mut Borders,
                ],
                &mut cx,
                |cx| cx.inner.ctl.stop_requested(),
            )?
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
                    failures: s.failures.into_iter().map(file_failure).collect(),
                }),
                Err(e) => cx.inner.notify(Notice::CropFailed { error: error_text(&e) }),
            }
        }
    }
}

/// Cuts the borders off the originals asked for, one request at a time (`pg_crop::cut`). A stopped cut keeps its rest at
/// the front of the queue. Its notice is told when the request is done.
struct Cuts;

impl Pass<Cx<'_>> for Cuts {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        loop {
            let Some(ids) = cx.inner.queue().cuts.pop_front() else {
                return Ok(Step::Done);
            };
            // Queued only once confirmed (`Runner::cut_borders`).
            match pg_crop::cut(cx.catalog, &ids, true, &Options::new(Priority::BelowNormal), &cx.inner.ctl) {
                Ok(s) => {
                    cx.inner.cut_done(&s.cut);
                    if !s.remaining.is_empty() {
                        cx.inner.queue().cuts.push_front(s.remaining);
                        return Ok(Step::Preempted);
                    }
                    cx.inner.notify(Notice::BordersCut {
                        saved: count(s.saved),
                        skipped: count(s.skipped),
                        unsupported: count(s.unsupported),
                        failed: count(s.failed),
                        failures: s.failures.into_iter().map(file_failure).collect(),
                    });
                }
                Err(e) => cx.inner.notify(Notice::CutFailed { error: error_text(&e) }),
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
            // What a scan leaves to the runner after the scan's phase has closed: the replaced thumbnails, the temp files, the
            // notices.
            let _after = pg_perf::span("scan.after");
            match scanned {
                Ok(s) => {
                    // A scan stopped early has committed what it recorded so far.
                    if s.changed > 0 || s.missing > 0 || s.pairing.rebound > 0 || s.pairing.replaced > 0 {
                        cx.inner.scanned();
                    }
                    // What the pairing replaced loses its thumbnails and the temp files of its queued writes.
                    // Both come after the catalog's commit, which the scan made.
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
                // The folder was removed from the library while it was walked (here, or by another process): the
                // removal marked its files missing and took the walk's lease, so the walk wrote nothing after it.
                // Nothing failed, and the folder isn't asked for again.
                Err(ScanError::Removed { .. }) => {}
                Err(ScanError::Failed(e)) => {
                    cx.inner.notify(Notice::ScanFailed { root: path, error: error_text(&e) });
                    // Its batches before the error are in.
                    cx.inner.scanned();
                }
            }
        }
    }
}

/// Analyzes what the scans found, from each file's header alone (`pg_import::import_while`): no hash, no pixel. The
/// duplicates are the Hashes pass's, after the thumbnails.
struct Import;

impl Pass<Cx<'_>> for Import {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        let s =
            pg_import::import_while(cx.catalog, &Options::new(Priority::BelowNormal), false, &|| true, &cx.inner.ctl)?;
        if s.files > 0 {
            cx.inner.imported();
        }
        Ok(Step::Done)
    }
}

/// Makes the duplicate hashes the import stored without them, while duplicates detection was off (`pg_import::hashes`).
/// Its pass ends after a batch when duplicates detection is turned off; what is left stays owed (`hashes_owed`).
struct Hashes;

impl Pass<Cx<'_>> for Hashes {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        if !cx.inner.features.get().duplicates {
            return Ok(Step::Done);
        }
        let features = cx.inner.features.clone();
        let s = pg_import::hashes::hash(
            cx.catalog,
            &Options::new(Priority::BelowNormal),
            &|| features.get().duplicates,
            &cx.inner.ctl,
        )?;
        // Its stored hashes are new pictures for the pairing pass, and the views list the duplicates again.
        if s.stored > 0 {
            cx.inner.imported();
        }
        // A file that couldn't be read is said, as the borders' are; one that didn't decode is an import failure, which
        // the import's ledger holds.
        cx.inner.files_read(FilePass::Import, s.unreadable, s.complete);
        Ok(Step::Done)
    }
}

/// Pairs what the import and the hashes left owed (`pg_import::pairs`): the pictures analyzed since the last pass, against
/// every picture hashed. Skipped while duplicates detection is off: the owed pairs wait (`pairs_owed`) for it to come
/// back on. Its join runs to the end once started, as a pass of seconds to minutes.
struct Pairing;

impl Pass<Cx<'_>> for Pairing {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        if !cx.inner.features.get().duplicates {
            return Ok(Step::Done);
        }
        let s = pg_import::pairs::pair(cx.catalog, &Options::new(Priority::BelowNormal), &cx.inner.ctl)?;
        // The duplicates the views list change with the pairs stored.
        if s.pairs > 0 || s.files > 0 {
            cx.inner.imported();
        }
        Ok(Step::Done)
    }
}

/// Looks for the borders of what was imported. Its own pass, after the import: the import's decode is reduced, the
/// detector wants full size. Skipped while border detection is off, and its due files stay due (`borders_due`), so
/// turning it back on finds them; it ends after a batch when it is turned off.
struct Borders;

impl Pass<Cx<'_>> for Borders {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        let features = cx.inner.features.clone();
        if !features.get().borders {
            // Nothing is looked for, so nothing is unreadable: a notice from an earlier pass is no longer true.
            cx.inner.files_read(FilePass::Borders, 0, true);
            return Ok(Step::Done);
        }
        let s = pg_borders::find_while(
            cx.catalog,
            &Options::new(Priority::BelowNormal),
            &|| features.get().borders,
            &cx.inner.ctl,
        )?;
        // Only what went into the catalog changes what the app shows: files that couldn't be read stored nothing.
        if s.stored > 0 {
            cx.inner.imported();
        }
        cx.inner.files_read(FilePass::Borders, s.unreadable, s.complete);
        // The borders' ledger rows are written in the transaction of the borders they go with (`pg_borders::find`): a
        // failure to write them fails the batch, which is the round's failure, not a ledger's of its own.
        Ok(Step::Done)
    }
}

/// Makes the thumbnails still missing at the grid's current height. It runs before the duplicates and the borders: the
/// grid shows them first. It ends after a batch when the grid asks for another height; the next round makes that one.
struct Thumbs;

impl Pass<Cx<'_>> for Thumbs {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        // No height yet: the grid has not said which thumbnails it wants, so there is nothing to make.
        let Some(height) = cx.inner.heights.get() else {
            return Ok(Step::Done);
        };
        let heights = cx.inner.heights.clone();
        let opts = Options::new(Priority::Idle);
        let s = match pg_thumbs::run_while(
            cx.inner.thumbs,
            &cx.inner.catalog_path,
            height,
            &opts,
            &|| heights.get() == Some(height),
            &cx.inner.ctl,
        ) {
            Ok(s) => s,
            // A pass that fails is said, and the round goes on to the duplicates and the borders, which are after it.
            Err(e) => {
                cx.inner.notify(Notice::CacheNotStored { error: error_text(&e) });
                return Ok(Step::Done);
            }
        };
        if s.cache_full {
            cx.inner.notify(Notice::CacheFull { height, made: count(s.made) });
        } else if s.complete {
            cx.inner.resolve(Trigger::Pass(FilePass::Thumbs));
        }
        cx.inner.files_read(FilePass::Thumbs, s.unreadable, s.complete);
        cx.inner.ledger_written(s.ledger);
        Ok(Step::Done)
    }
}

/// Makes the ThumbHash placeholders the thumbnails left owing (`pg_thumbs::placeholders`), after them. It is not tied to
/// the grid's height: a placeholder is read from a cached thumbnail or from the original, whatever the grid shows.
struct Placeholders;

impl Pass<Cx<'_>> for Placeholders {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        // Its failures (the files that don't decode) are the Thumbs ledger's rows, which the pass writes in its batches.
        pg_thumbs::placeholders::run_while(
            cx.catalog,
            cx.inner.thumbs,
            &Options::new(Priority::Idle),
            &|| true,
            &cx.inner.ctl,
        )?;
        Ok(Step::Done)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Instant;

    use pg_jobs_testkit::{Call, Quiet, RecordingSink};
    use pg_model::notice::ScanNote;
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

    /// The features are held as they were set, and each change to other features wakes the job once; the same features
    /// again wake nothing.
    #[test]
    fn features_are_held_and_each_change_wakes_the_job_once() {
        let features = FeatureCell::new(Features::ALL);
        let woke = Arc::new(AtomicU64::new(0));
        let counter = woke.clone();
        features.listen(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let no_duplicates = Features { borders: true, duplicates: false };
        features.set(Features::ALL);
        features.set(no_duplicates);
        features.set(no_duplicates);
        assert_eq!(features.get(), no_duplicates);
        assert_eq!(woke.load(Ordering::Relaxed), 1);
    }

    /// A library folder `images` in a run folder, holding `names` (small PNGs), scanned and imported into the catalog
    /// beside it: the run folder, the catalog's path, and the library folder.
    fn library(prefix: &str, names: &[&str]) -> (PathBuf, PathBuf, RulePath) {
        library_sized(prefix, names, 8)
    }

    /// [`library`], with square pictures of `side` pixels (the border detector looks at those of 64 or more).
    fn library_sized(prefix: &str, names: &[&str], side: u32) -> (PathBuf, PathBuf, RulePath) {
        let dir = pg_testkit::run_folder_with("scratch_runner", prefix, &["images"]).unwrap();
        // A pattern, not a flat colour: a flat picture has too little PDQ quality to owe its pairs.
        let pixels: Vec<u8> =
            (0..side * side * 3).map(|i| u8::try_from(i.wrapping_mul(2_654_435_761) >> 24).unwrap()).collect();
        for name in names {
            std::fs::write(dir.join("images").join(name), pg_testkit::png_rgb(side, side, &pixels).unwrap()).unwrap();
        }
        let catalog_path = dir.join("catalog.sqlite");
        let mut c = Catalog::prepare(&catalog_path, crate::versions(), PATH_RULES).unwrap();
        let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
        assert!(c.add_root(&root).unwrap());
        pg_scan::scan(&mut c, &root, &Control::new()).unwrap();
        pg_import::import_while(&mut c, &Options::new(Priority::Normal), false, &|| true, &Control::new()).unwrap();
        (dir, catalog_path, root)
    }

    /// With a feature off, its passes do nothing and what they owe stays owed: the Hashes and Pairing passes while
    /// duplicates is off, the Borders pass while borders is off. Turned back on, the Hashes pass makes the hashes owed.
    #[test]
    fn a_pass_whose_feature_is_off_does_nothing_and_its_work_stays_owed() {
        let (dir, catalog_path, _root) = library("runner-off", &["a.png"]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache, sink.clone());
        inner.features.set(Features { borders: false, duplicates: false });
        let due = pg_catalog::fixture::state::borders_due(&Catalog::open(&catalog_path, PATH_RULES).unwrap(), "a.png")
            .unwrap();
        assert_eq!(run(&mut Hashes, &inner), Step::Done);
        assert_eq!(run(&mut Pairing, &inner), Step::Done);
        assert_eq!(run(&mut Borders, &inner), Step::Done);
        let c = Catalog::open(&catalog_path, PATH_RULES).unwrap();
        assert_eq!(c.owed_hashes().unwrap(), 1, "the hashes stay owed while duplicates is off");
        assert_eq!(pg_catalog::fixture::state::borders_due(&c, "a.png").unwrap(), due, "the borders are not touched");

        inner.features.set(Features { borders: false, duplicates: true });
        assert_eq!(run(&mut Hashes, &inner), Step::Done);
        assert_eq!(c.owed_hashes().unwrap(), 0, "turned on, the pass makes them");
    }

    /// One round makes the thumbnails, then the placeholders, then the hashes, then the pairs, then the borders: each pass ends its phase
    /// (`Finished`) in that order. A picture imported without its hashes owes all four, at a 300-pixel size the borders
    /// look at.
    #[test]
    fn a_round_makes_the_thumbnails_then_the_hashes_then_the_pairs_then_the_borders() {
        let (dir, catalog_path, _root) = library_sized("runner-order", &["a.png"], 300);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache, sink.clone());
        let ended = inner.ctl.subscribe();
        inner.round(&mut None).unwrap();
        assert_eq!(finished_order(ended.take().items), ["thumbs", "placeholders", "hashes", "pairs", "borders"]);
    }

    /// The passes that ended, in the order they ended, among the four the round's order is about (a pass that ends with
    /// nothing to do ends too, and says so as the others do).
    fn finished_order(events: Vec<pg_jobs::control::Event>) -> Vec<&'static str> {
        events
            .into_iter()
            .filter_map(|event| match event {
                pg_jobs::control::Event::Finished(progress) => match progress.phase {
                    pg_model::progress::Phase::Thumbs { .. } => Some("thumbs"),
                    pg_model::progress::Phase::Placeholders => Some("placeholders"),
                    pg_model::progress::Phase::Hashes => Some("hashes"),
                    pg_model::progress::Phase::Pairing => Some("pairs"),
                    pg_model::progress::Phase::Borders => Some("borders"),
                    _ => None,
                },
                pg_jobs::control::Event::Progress(_) => None,
            })
            .collect()
    }

    /// A thumbnail cache that refuses its batch is said (`CacheNotStored`), and the round goes on: the hashes, the pairs
    /// and the borders still run after the thumbnails. (Before this, the error ended the round, and they never ran.)
    #[test]
    fn a_thumbnail_cache_error_is_said_and_the_round_goes_on() {
        let (dir, catalog_path, _root) = library_sized("runner-cache-error", &["a.png"], 300);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        pg_thumbs::fault::refuse_stores(&dir.join("thumbs.sqlite")).unwrap();
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache, sink.clone());
        let ended = inner.ctl.subscribe();
        inner.round(&mut None).unwrap();
        let posted = sink.posted();
        assert!(
            posted.iter().any(|n| matches!(n, Notice::CacheNotStored { error } if error.contains("refuses"))),
            "{posted:?}"
        );
        assert_eq!(finished_order(ended.take().items), ["thumbs", "placeholders", "hashes", "pairs", "borders"]);
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
            features: FeatureCell::new(Features::ALL),
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
        assert!(sink.calls().contains(&Call::Resolve(Trigger::Pass(FilePass::Thumbs))), "{:?}", sink.calls());
    }

    /// A cache too small for one thumbnail is said; a complete pass (here, once its file left the library) clears it.
    #[test]
    fn a_full_cache_is_said_and_a_complete_pass_clears_it() {
        let (dir, catalog_path, root) = library("runner-full", &["a.png"]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, Some(1));
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache, sink.clone());
        run(&mut Thumbs, &inner);
        assert_eq!(
            sink.posted(),
            [Notice::CacheFull { height: ThumbHeight::new(64).expect("a height in range"), made: 0 }]
        );
        std::fs::remove_file(dir.join("images").join("a.png")).unwrap();
        pg_scan::scan(&mut Catalog::open(&catalog_path, PATH_RULES).unwrap(), &root, &Control::new()).unwrap();
        sink.take();
        run(&mut Thumbs, &inner);
        assert!(sink.calls().contains(&Call::Resolve(Trigger::Pass(FilePass::Thumbs))), "{:?}", sink.calls());
    }

    /// A round that fails (its catalog can't be opened) is said; the next round that runs clears it.
    #[test]
    fn a_failed_round_is_said_and_the_next_round_clears_it() {
        let (dir, catalog_path, _root) = library("runner-round", &[]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let aside = dir.join("aside.sqlite");
        std::fs::rename(&catalog_path, &aside).unwrap();
        let sink = RecordingSink::new();
        let runner = Runner::start(
            catalog_path.clone(),
            cache,
            height_64(),
            FeatureCell::new(Features::ALL),
            Arc::new(Bus::new(16)),
            sink.clone(),
        )
        .unwrap();
        wait_for("the round's failure", || sink.posted().iter().any(|n| matches!(n, Notice::RoundFailed { .. })));
        std::fs::rename(&aside, &catalog_path).unwrap();
        runner.wake();
        wait_for("the failure cleared", || sink.calls().contains(&Call::Resolve(Trigger::Op(Op::Round))));
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
        inner.ledger_written(LedgerOutcome::Untouched);
        assert!(sink.take().is_empty());
        inner.ledger_written(LedgerOutcome::Failed("disk full".into()));
        assert_eq!(sink.take(), [Call::Post(Notice::LedgerNotWritten { error: "disk full".into() })]);
        inner.ledger_written(LedgerOutcome::Written);
        assert_eq!(sink.take(), [Call::Resolve(Trigger::Op(Op::LedgerWrite(FilePass::Thumbs)))]);
    }

    /// The job pokes its sink when a pass says the ledger changed (`Control::ledger_changed`).
    #[test]
    fn a_ledger_change_pokes_the_sink() {
        let (dir, catalog_path, _root) = library("runner-poke", &[]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let runner = Runner::start(
            catalog_path.clone(),
            cache,
            height_64(),
            FeatureCell::new(Features::ALL),
            Arc::new(Bus::new(16)),
            sink.clone(),
        )
        .unwrap();
        let before = sink.pokes();
        runner.inner.ctl.ledger_changed();
        assert_eq!(sink.pokes(), before + 1);
        assert!(runner.stop(Duration::from_secs(30)));
    }

    /// A cut is queued only once confirmed: an unconfirmed one queues nothing. A confirmed one runs its pass, which says
    /// what became of the file (here, a file with no borders to cut: skipped).
    #[test]
    fn a_cut_is_queued_only_when_confirmed_and_is_told() {
        let (dir, catalog_path, _root) = library("runner-cut", &["a.png"]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let runner = Runner::start(
            catalog_path.clone(),
            cache,
            height_64(),
            FeatureCell::new(Features::ALL),
            Arc::new(Bus::new(16)),
            sink.clone(),
        )
        .unwrap();
        let c = Catalog::open(&catalog_path, PATH_RULES).unwrap();
        let id = pg_catalog::fixture::state::present_files(&c).unwrap()[0].0;
        runner.cut_borders(vec![id], false);
        assert!(runner.inner.queue().cuts.is_empty(), "an unconfirmed cut queues nothing");
        runner.cut_borders(vec![id], true);
        wait_for("the cut's notice", || sink.posted().iter().any(|n| matches!(n, Notice::BordersCut { .. })));
        let posted = sink.posted();
        let told = Notice::BordersCut { saved: 0, skipped: 1, unsupported: 0, failed: 0, failures: Vec::new() };
        assert!(posted.contains(&told), "{posted:?}");
        assert!(runner.stop(Duration::from_secs(30)));
    }
}
