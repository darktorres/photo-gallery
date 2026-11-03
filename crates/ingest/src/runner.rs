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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use pg_catalog::Catalog;
use pg_jobs::control::{Control, Progress};
use pg_jobs::panics;
use pg_jobs::pipeline::Options;
use pg_jobs::schedule::{self, Pass, Step};
use pg_jobs::service::{Service, Waker, Woke};
use pg_model::ids::FileId;
use pg_model::path::is_under;
use pg_platform::priority::Priority;
use pg_thumbcache::ThumbCache;

use crate::{borders, crop, import, scan, thumbs};

#[derive(Default)]
struct Queue {
    /// Folders to scan, normalized.
    scans: VecDeque<String>,
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
}

/// Something the job tells the user: an error or a warning, or how a job asked for went.
pub struct Message {
    pub text: String,
    pub warning: bool,
}

/// The most messages kept until the UI takes them.
const MESSAGE_CAP: usize = 100;

/// Messages for the UI, oldest first. Past `MESSAGE_CAP` the oldest are dropped and counted in `not_kept`, so the
/// newest is always there and whoever takes them can say how many it did not see.
#[derive(Default)]
pub struct Messages {
    pub kept: VecDeque<Message>,
    pub not_kept: usize,
}

impl Messages {
    fn push(&mut self, message: Message) {
        if self.kept.len() == MESSAGE_CAP {
            self.kept.pop_front();
            self.not_kept += 1;
        }
        self.kept.push_back(message);
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
    /// at `thumb_height`.
    pub fn start(catalog_path: PathBuf, thumbs: &'static ThumbCache, thumb_height: u32) -> Runner {
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
        });
        let job = inner.clone();
        let service = Service::start("background job", move |waker| job.run(waker));
        Runner { inner, service }
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
    pub fn scan(&self, roots: Vec<String>) {
        {
            let mut q = self.inner.queue.lock().unwrap();
            for r in roots {
                if !q.scans.contains(&r) {
                    q.scans.push_back(r);
                }
            }
        }
        self.wake_up();
    }

    /// Save cropped copies of these files' borders into `folder` (`crop::save`), before anything else.
    pub fn save_crops(&self, ids: Vec<FileId>, folder: PathBuf) {
        self.inner.queue.lock().unwrap().crops.push_back((ids, folder));
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

    /// Errors and notes since the last call (a folder that couldn't be scanned, a full cache).
    pub fn take_messages(&self) -> Messages {
        std::mem::take(&mut *self.inner.messages.lock().unwrap())
    }

    /// The queue has a new request: stop the round in progress at its next file, and start one.
    fn wake_up(&self) {
        self.inner.ctl.request_stop();
        self.service.wake();
    }
}

impl Inner {
    fn note(&self, text: String) {
        self.messages.lock().unwrap().push(Message { text, warning: false });
    }

    fn warn(&self, text: String) {
        self.messages.lock().unwrap().push(Message { text, warning: true });
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
                Err(e) => self.warn(format!("{e:#}")),
            }
            self.rounds.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// A round, with a panic in it caught: the job reports it (the status line, and the log through the panic hook)
    /// and goes on with the next request.
    fn round_caught(&self, catalog: &mut Option<Catalog>) -> Result<Step> {
        match panics::catch(AssertUnwindSafe(|| self.round(catalog))) {
            Ok(result) => result,
            Err(panic) => {
                // The connection a panicking pass held is not trusted: the next round opens a new one.
                *catalog = None;
                Err(anyhow!("internal error in the background job: {}", panics::message(&*panic)))
            }
        }
    }

    /// One round: the passes in order, on the connection the job keeps between rounds.
    fn round(&self, conn: &mut Option<Catalog>) -> Result<Step> {
        let mut catalog = match conn.take() {
            Some(c) => c,
            None => Catalog::open(&self.catalog_path)?,
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
            let Some((ids, folder)) = cx.inner.queue.lock().unwrap().crops.pop_front() else {
                return Ok(Step::Done);
            };
            match crop::save(cx.catalog, &ids, &folder, &Options::new(Priority::BelowNormal), &cx.inner.ctl) {
                Ok(s) if !s.remaining.is_empty() => {
                    cx.inner.queue.lock().unwrap().crops.push_front((s.remaining, folder));
                    return Ok(Step::Preempted);
                }
                Ok(s) if s.failed > 0 => cx.inner.warn(crop_note(&s, &folder)),
                Ok(s) => cx.inner.note(crop_note(&s, &folder)),
                Err(e) => cx.inner.warn(format!("{e:#}")),
            }
        }
    }
}

/// Scans the folders asked for, skipping those no longer in the library. A scan stopped early goes back to the front.
struct Scans;

impl Pass<Cx<'_>> for Scans {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        let library: Vec<String> = cx.catalog.roots()?.into_iter().map(|r| r.path).collect();
        loop {
            let Some(root) = cx.inner.queue.lock().unwrap().scans.pop_front() else {
                return Ok(Step::Done);
            };
            if !library.iter().any(|l| is_under(&root, l)) {
                continue;
            }
            match scan::scan(cx.catalog, &root, &cx.inner.ctl) {
                Ok(s) => {
                    // A scan stopped early has committed what it recorded so far.
                    if s.changed > 0 || s.missing > 0 {
                        cx.inner.scans_done.fetch_add(1, Ordering::Relaxed);
                    }
                    if !s.complete {
                        cx.inner.queue.lock().unwrap().scans.push_front(root);
                        return Ok(Step::Preempted);
                    }
                    if s.errors > 0 {
                        cx.inner.warn(format!("{root}: {} folders couldn't be read", s.errors));
                    }
                }
                Err(e) => {
                    cx.inner.warn(format!("{root}: {e:#}"));
                    // Its batches before the error are in.
                    cx.inner.scans_done.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
}

/// Analyzes what the scans found.
struct Import;

impl Pass<Cx<'_>> for Import {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        let s = import::import(cx.catalog, &Options::new(Priority::BelowNormal), false, &cx.inner.ctl)?;
        if s.files > 0 {
            cx.inner.imports_done.fetch_add(1, Ordering::Relaxed);
        }
        Ok(Step::Done)
    }
}

/// Looks for the borders of what was imported. Its own pass, after the import: the import's decode is reduced, the
/// detector wants full size.
struct Borders;

impl Pass<Cx<'_>> for Borders {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        let s = borders::find(cx.catalog, &Options::new(Priority::BelowNormal), &cx.inner.ctl)?;
        if s.files > 0 {
            cx.inner.imports_done.fetch_add(1, Ordering::Relaxed);
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
        let s = thumbs::run(cx.inner.thumbs, &cx.inner.catalog_path, height, &opts, &cx.inner.ctl)?;
        if s.cache_full {
            cx.inner.warn(format!("The thumbnail cache is full: {} thumbnails at {height} px weren't made", s.made));
        }
        Ok(Step::Done)
    }
}

/// What a crop job did, for the status line.
fn crop_note(s: &crop::Stats, folder: &Path) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let mut note =
        format!("Saved {} cropped cop{} in {}", s.saved, if s.saved == 1 { "y" } else { "ies" }, folder.display());
    if s.skipped > 0 {
        note += &format!("; {} file{} had nothing to crop any more", s.skipped, plural(s.skipped));
    }
    if let Some(e) = &s.error {
        note += &format!("; {} failed, the first: {e}", s.failed);
    }
    note
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_messages_are_kept_and_the_rest_counted() {
        let mut messages = Messages::default();
        for i in 0..MESSAGE_CAP + 5 {
            messages.push(Message { text: i.to_string(), warning: false });
        }
        assert_eq!(messages.kept.len(), MESSAGE_CAP);
        assert_eq!(messages.not_kept, 5);
        assert_eq!(messages.kept.front().unwrap().text, "5");
        assert_eq!(messages.kept.back().unwrap().text, (MESSAGE_CAP + 4).to_string());
    }
}
