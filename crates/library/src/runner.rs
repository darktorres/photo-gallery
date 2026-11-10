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
use pg_jobs::control::{Control, Progress};
use pg_jobs::events::{Bus, Change};
use pg_jobs::failure::FileError;
use pg_jobs::panics;
use pg_jobs::pipeline::Options;
use pg_jobs::schedule::{self, Pass, Step};
use pg_jobs::service::{Service, Waker, Woke};
use pg_model::ids::FileId;
use pg_model::rules::{Path as RulePath, PathRules};
use pg_platform::priority::Priority;
use pg_thumbcache::ThumbCache;

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
    thumb_height: AtomicU32,
    scans_done: AtomicU64,
    imports_done: AtomicU64,
    /// Rounds finished, however they ended: the app marks each one as a stage for measuring (docs/PROFILING.md).
    rounds: AtomicU64,
    messages: Mutex<Messages>,
    /// Told what changed (`Change`), so the owner reads the new state instead of polling for it.
    changes: Arc<Bus<Change>>,
}

/// What a scan counted (`Notice::ScanNote`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanNote {
    /// Files skipped because another path (a hard link) or another folder of the library holds them.
    Duplicates,
    /// Reparse points (links, cloud placeholders) indexed without being followed.
    SkippedReparse,
    /// Files whose old row could not be told apart from another (nothing was bound).
    Ambiguous,
    /// Rows of files that went missing, kept with their ratings, whose new file wasn't found.
    UnpairedWithData,
    /// Rows whose file was replaced by another with the same name: their content was reset.
    Replaced,
    /// Rows whose file was found again under a new identity (a renumbered drive, or a move while closed).
    Rebound,
    /// Volumes with no file system UUID: their files are identified by a device number, which a restart can renumber.
    VolumeFallback,
}

impl ScanNote {
    /// The kind the app words the note by.
    pub fn kind(self) -> &'static str {
        match self {
            ScanNote::Duplicates => "duplicates",
            ScanNote::SkippedReparse => "reparse",
            ScanNote::Ambiguous => "ambiguous",
            ScanNote::UnpairedWithData => "unpaired",
            ScanNote::Replaced => "replaced",
            ScanNote::Rebound => "rebound",
            ScanNote::VolumeFallback => "volume-id",
        }
    }
}

/// Something the job tells the user: a problem it ran into, or how a job asked for went. The job says what happened
/// in values; the app words it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// Folders of a scan that couldn't be read.
    UnreadableFolders { root: String, count: u64 },
    /// A scan that failed. Its batches before the error are in.
    ScanFailed { root: String, error: String },
    /// A folder the scanner refuses (decisions 6 and 13); it stays in the library until it is removed.
    RootRefused { root: String, reason: String },
    /// A folder under a scanned root, on another file system, that the scanner refuses (a network or pseudo file system
    /// mounted there). Its files are not in the scan; `mount` is the folder.
    MountRefused { root: String, mount: String, reason: String },
    /// What a scan of a folder counted that the user may want to know, `count` of them (S-17.7 decision 12).
    ScanNote { root: String, note: ScanNote, count: u64 },
    /// A round that failed.
    RoundFailed { error: String },
    /// A round that panicked (caught): the job goes on with the next request.
    RoundPanicked { message: String },
    /// The thumbnail cache is full: `made` thumbnails at `height` px weren't made.
    CacheFull { height: u32, made: usize },
    /// A crop job that ran to the end of its request. `refused` is the first refusal (`pg_crop::Stats::refused`).
    Cropped {
        folder: PathBuf,
        saved: usize,
        skipped: usize,
        failed: usize,
        first_error: Option<FileError>,
        /// Boxed: a refusal carries two strings, and a notice travels in the `Err` of a round, which clippy's
        /// `result_large_err` bounds.
        refused: Option<Box<pg_crop::CropError>>,
    },
    /// A crop job that failed before it could save anything.
    CropFailed { error: String },
}

impl Notice {
    /// Whether it is a problem (shown as a warning), not a job that went as asked.
    pub fn is_warning(&self) -> bool {
        !matches!(self, Notice::Cropped { failed: 0, .. })
    }

    /// The file and its folder of a crop's refusal, for the app to word (`pg_client::text`): the app names this crate's
    /// notices, not the crop's error type.
    pub fn crop_refusal(&self) -> Option<(&str, &str)> {
        match self {
            Notice::Cropped { refused: Some(refused), .. } => match &**refused {
                pg_crop::CropError::FolderRefused { file, folder } => Some((file.as_str(), folder.as_str())),
            },
            _ => None,
        }
    }
}

/// The most messages kept until the UI takes them.
const MESSAGE_CAP: usize = 100;

/// Messages for the UI, oldest first. Past `MESSAGE_CAP` the oldest are dropped and counted in `not_kept`, so the
/// newest is always there and whoever takes them can say how many it did not see.
#[derive(Default)]
pub struct Messages {
    pub kept: VecDeque<Notice>,
    pub not_kept: usize,
}

impl Messages {
    fn push(&mut self, notice: Notice) {
        if self.kept.len() == MESSAGE_CAP {
            self.kept.pop_front();
            self.not_kept += 1;
        }
        self.kept.push_back(notice);
    }
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
    /// at `thumb_height`. What it changes is published on `changes` (`Change`). An error if the thread can't be
    /// spawned.
    pub fn start(
        catalog_path: PathBuf,
        thumbs: &'static ThumbCache,
        thumb_height: u32,
        changes: Arc<Bus<Change>>,
    ) -> Result<Runner> {
        let inner = Arc::new(Inner {
            catalog_path,
            thumbs,
            ctl: Control::new(),
            queue: Mutex::new(Queue::default()),
            thumb_height: AtomicU32::new(thumb_height),
            scans_done: AtomicU64::new(0),
            imports_done: AtomicU64::new(0),
            rounds: AtomicU64::new(0),
            messages: Mutex::new(Messages::default()),
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

    /// Notices since the last call (a folder that couldn't be scanned, a full cache).
    pub fn take_messages(&self) -> Messages {
        std::mem::take(&mut *self.inner.messages.lock().unwrap_or_else(PoisonError::into_inner))
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
        self.messages.lock().unwrap_or_else(PoisonError::into_inner).push(notice);
        self.changes.publish(Change::Notice);
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
                Ok(Step::Done) => {}
                Ok(Step::Preempted) => again = true,
                Err(notice) => self.notify(notice),
            }
            self.rounds.fetch_add(1, Ordering::Relaxed);
            self.changes.publish(Change::Round);
        }
    }

    /// A round, with a panic in it caught: the job reports it (the status line, and the log through the panic hook)
    /// and goes on with the next request.
    fn round_caught(&self, catalog: &mut Option<Catalog>) -> std::result::Result<Step, Notice> {
        match panics::catch(AssertUnwindSafe(|| self.round(catalog))) {
            Ok(result) => result.map_err(|e| Notice::RoundFailed { error: format!("{e:#}") }),
            Err(panic) => {
                // The connection a panicking pass held is not trusted: the next round opens a new one.
                *catalog = None;
                Err(Notice::RoundPanicked { message: panics::message(&*panic).to_string() })
            }
        }
    }

    /// One round: the passes in order, on the connection the job keeps between rounds.
    fn round(&self, conn: &mut Option<Catalog>) -> Result<Step> {
        let mut catalog = match conn.take() {
            Some(c) => c,
            None => Catalog::open(&self.catalog_path, PathRules::NATIVE)?,
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
                    folder,
                    saved: s.saved,
                    skipped: s.skipped,
                    failed: s.failed,
                    first_error: s.error,
                    refused: s.refused.map(Box::new),
                }),
                Err(e) => cx.inner.notify(Notice::CropFailed { error: format!("{e:#}") }),
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
            match pg_scan::scan(cx.catalog, &path, &cx.inner.ctl) {
                Ok(s) => {
                    let root = rules.display(&path);
                    // A scan stopped early has committed what it recorded so far.
                    if s.changed > 0 || s.missing > 0 || s.rebound > 0 || s.replaced > 0 {
                        cx.inner.scanned();
                    }
                    // What the pairing replaced loses its thumbnails and the temp files of its queued writes (decision
                    // 7). Both come after the catalog's commit, which the scan made.
                    for id in &s.replaced_ids {
                        if let Err(e) = cx.inner.thumbs.forget(*id) {
                            cx.inner.notify(Notice::ScanFailed { root: root.clone(), error: format!("{e:#}") });
                        }
                    }
                    for temp in &s.temps {
                        let disk = pg_platform::names::path_buf(&cx.catalog.rules().join(temp));
                        match std::fs::remove_file(&disk) {
                            Ok(()) => {}
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            Err(e) => cx.inner.notify(Notice::ScanFailed {
                                root: root.clone(),
                                error: format!("can't delete {}: {e}", cx.catalog.rules().display(temp)),
                            }),
                        }
                    }
                    if !s.complete {
                        cx.inner.queue().scans.push_front(path);
                        return Ok(Step::Preempted);
                    }
                    if s.errors > 0 {
                        cx.inner.notify(Notice::UnreadableFolders { root: root.clone(), count: s.errors });
                    }
                    for refused in &s.refused_mount_list {
                        cx.inner.notify(Notice::MountRefused {
                            root: root.clone(),
                            mount: refused.path.clone(),
                            reason: refused.error.clone(),
                        });
                    }
                    let counts = [
                        (ScanNote::Duplicates, s.duplicates),
                        (ScanNote::SkippedReparse, s.skipped_reparse),
                        (ScanNote::Ambiguous, u64::try_from(s.ambiguous).unwrap_or(u64::MAX)),
                        (ScanNote::UnpairedWithData, u64::try_from(s.unpaired_with_data).unwrap_or(u64::MAX)),
                        (ScanNote::Replaced, u64::try_from(s.replaced).unwrap_or(u64::MAX)),
                        (ScanNote::Rebound, u64::try_from(s.rebound).unwrap_or(u64::MAX)),
                        (ScanNote::VolumeFallback, s.volume_fallbacks),
                    ];
                    for (note, count) in counts {
                        if count > 0 {
                            cx.inner.notify(Notice::ScanNote { root: root.clone(), note, count });
                        }
                    }
                }
                Err(e) => {
                    let root = rules.display(&path);
                    let notice = match e.downcast_ref::<pg_platform::dir::Refused>() {
                        Some(refused) => Notice::RootRefused { root, reason: refused.reason.clone() },
                        None => Notice::ScanFailed { root, error: format!("{e:#}") },
                    };
                    cx.inner.notify(notice);
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
            cx.inner.notify(Notice::CacheFull { height, made: s.made });
        }
        Ok(Step::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_messages_are_kept_and_the_rest_counted() {
        let mut messages = Messages::default();
        for i in 0..MESSAGE_CAP + 5 {
            messages.push(Notice::CacheFull { height: 0, made: i });
        }
        assert_eq!(messages.kept.len(), MESSAGE_CAP);
        assert_eq!(messages.not_kept, 5);
        assert_eq!(messages.kept.front(), Some(&Notice::CacheFull { height: 0, made: 5 }));
        assert_eq!(messages.kept.back(), Some(&Notice::CacheFull { height: 0, made: MESSAGE_CAP + 4 }));
    }

    #[test]
    fn only_a_crop_with_failures_is_a_warning() {
        let folder = PathBuf::from("out");
        let done = Notice::Cropped {
            folder: folder.clone(),
            saved: 2,
            skipped: 0,
            failed: 0,
            first_error: None,
            refused: None,
        };
        assert!(!done.is_warning());
        let some_failed = Notice::Cropped { folder, saved: 1, skipped: 0, failed: 1, first_error: None, refused: None };
        assert!(some_failed.is_warning());
        assert!(Notice::RoundFailed { error: String::new() }.is_warning());
    }
}
