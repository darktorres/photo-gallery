//! The app's background job (PLAN §4, §6): one thread that saves the cropped copies asked for, scans the folders
//! asked for, imports whatever is pending, looks for the borders of what it imported (and of everything after a
//! new detector), then makes thumbnails at the grid's current height, and sleeps until there is something new to
//! do.
//!
//! A new request (a folder added, a rescan, a new thumbnail height) stops the pass in progress at the next file;
//! it commits what it has and the next round starts with the new work. Every pass is resumable from the catalog,
//! so stopping costs nothing but the batch in flight.

use pg_model::ids::FileId;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::Result;
use pg_catalog::Catalog;
use pg_model::path::is_under;
use pg_platform::priority::Priority;
use pg_thumbcache::ThumbCache;

use crate::control::{Control, Progress};
use crate::pipeline::Options;
use crate::{borders, crop, import, scan, thumbs};

#[derive(Default)]
struct Queue {
    /// Folders to scan, normalized.
    scans: Vec<String>,
    /// Cropped copies to save: the files, and the folder.
    crops: Vec<(Vec<FileId>, PathBuf)>,
    /// Something to do: start a round.
    wake: bool,
}

struct Inner {
    catalog_path: PathBuf,
    thumbs: &'static ThumbCache,
    /// Set by `stop`: the job ends at its next wait.
    quit: AtomicBool,
    ctl: Control,
    queue: Mutex<Queue>,
    wake: Condvar,
    thumb_height: AtomicU32,
    scans_done: AtomicU64,
    imports_done: AtomicU64,
    messages: Mutex<Messages>,
    on_round_done: Box<dyn Fn() + Send + Sync>,
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
    /// The job thread, and the signal it sends as it ends (`stop` waits for that, within a bound).
    thread: Mutex<Option<JoinHandle<()>>>,
    exited: Mutex<Option<mpsc::Receiver<()>>>,
}

impl Runner {
    /// Start the job thread. It begins by importing whatever is pending and making the thumbnails still missing
    /// at `thumb_height`. `on_round_done` runs on the job's thread after every round, however it ended (the app
    /// marks it as a stage for measuring, docs/PROFILING.md).
    pub fn start(
        catalog_path: PathBuf,
        thumbs: &'static ThumbCache,
        thumb_height: u32,
        on_round_done: impl Fn() + Send + Sync + 'static,
    ) -> Runner {
        let inner = Arc::new(Inner {
            catalog_path,
            thumbs,
            quit: AtomicBool::new(false),
            ctl: Control::new(false),
            queue: Mutex::new(Queue { scans: Vec::new(), crops: Vec::new(), wake: true }),
            wake: Condvar::new(),
            thumb_height: AtomicU32::new(thumb_height),
            scans_done: AtomicU64::new(0),
            imports_done: AtomicU64::new(0),
            messages: Mutex::new(Messages::default()),
            on_round_done: Box::new(on_round_done),
        });
        let job = inner.clone();
        let (exit, exited) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("background job".into())
            .spawn(move || {
                job.run();
                let _ = exit.send(());
            })
            .expect("spawn job");
        Runner { inner, thread: Mutex::new(Some(thread)), exited: Mutex::new(Some(exited)) }
    }

    /// Stop the job after the file (or the batch) in progress, as a new request does, and wait for its thread to end,
    /// for at most `timeout`: its connections close with it. `false` if it is still running then; it ends with the
    /// process, and the WAL it left is merged at the next open.
    pub fn stop(&self, timeout: Duration) -> bool {
        {
            let mut q = self.inner.queue.lock().unwrap();
            self.inner.quit.store(true, Ordering::Relaxed);
            q.wake = true;
        }
        self.inner.ctl.request_stop();
        self.inner.wake.notify_all();
        let exited = self.exited.lock().unwrap().take();
        let finished = match exited {
            Some(exited) => {
                matches!(exited.recv_timeout(timeout), Ok(()) | Err(RecvTimeoutError::Disconnected))
            }
            // The first stop took the channel, so the thread's own state answers for a later one.
            None => self.thread.lock().unwrap().as_ref().is_none_or(JoinHandle::is_finished),
        };
        if finished && let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
        finished
    }

    /// Scan these folders (normalized), then import and thumbnail what they hold.
    pub fn scan(&self, roots: Vec<String>) {
        let mut q = self.inner.queue.lock().unwrap();
        for r in roots {
            if !q.scans.contains(&r) {
                q.scans.push(r);
            }
        }
        self.inner.wake_up(q);
    }

    /// Save cropped copies of these files' borders into `folder` (`crop::save`), before anything else.
    pub fn save_crops(&self, ids: Vec<FileId>, folder: PathBuf) {
        let mut q = self.inner.queue.lock().unwrap();
        q.crops.push((ids, folder));
        self.inner.wake_up(q);
    }

    /// The catalog has new work outside a scan (a file turned in place needs its analysis redone): start a round.
    pub fn wake(&self) {
        self.inner.wake_up(self.inner.queue.lock().unwrap());
    }

    /// The grid's thumbnail height changed: make the missing ones at the new height instead.
    pub fn set_thumb_height(&self, height: u32) {
        if self.inner.thumb_height.swap(height, Ordering::Relaxed) != height {
            self.inner.wake_up(self.inner.queue.lock().unwrap());
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

    /// Errors and notes since the last call (a folder that couldn't be scanned, a full cache).
    pub fn take_messages(&self) -> Messages {
        std::mem::take(&mut *self.inner.messages.lock().unwrap())
    }
}

impl Inner {
    fn wake_up(&self, mut q: std::sync::MutexGuard<Queue>) {
        q.wake = true;
        self.ctl.request_stop();
        self.wake.notify_one();
    }

    fn note(&self, text: String) {
        self.messages.lock().unwrap().push(Message { text, warning: false });
    }

    fn warn(&self, text: String) {
        self.messages.lock().unwrap().push(Message { text, warning: true });
    }

    fn run(&self) {
        // One connection for every round; after a round that failed, a new one.
        let mut catalog: Option<Catalog> = None;
        loop {
            let roots = {
                let mut q = self.queue.lock().unwrap();
                while !q.wake {
                    q = self.wake.wait(q).unwrap();
                }
                if self.quit.load(Ordering::Relaxed) {
                    return;
                }
                q.wake = false;
                self.ctl.clear_stop();
                (std::mem::take(&mut q.scans), std::mem::take(&mut q.crops))
            };
            let done = catalog.take().map_or_else(|| Catalog::open(&self.catalog_path), Ok).and_then(|mut c| {
                self.round(&mut c, roots.0, roots.1)?;
                catalog = Some(c);
                Ok(())
            });
            if let Err(e) = done {
                self.warn(format!("{e:#}"));
            }
            (self.on_round_done)();
        }
    }

    /// Put back folders a stopped round didn't finish.
    fn requeue(&self, roots: &[String]) {
        let mut q = self.queue.lock().unwrap();
        let rest: Vec<String> = roots.iter().filter(|r| !q.scans.contains(r)).cloned().collect();
        q.scans.splice(0..0, rest);
        q.wake = true;
    }

    fn round(&self, catalog: &mut Catalog, roots: Vec<String>, crops: Vec<(Vec<FileId>, PathBuf)>) -> Result<()> {
        for (i, (ids, folder)) in crops.iter().enumerate() {
            match crop::save(catalog, ids, folder, &Options::new(Priority::BelowNormal), &self.ctl) {
                Ok(s) if !s.remaining.is_empty() => {
                    // Stopped: what is left of it, and of the round, goes first in the next one.
                    let rest = std::iter::once((s.remaining, folder.clone())).chain(crops[i + 1..].iter().cloned());
                    self.queue.lock().unwrap().crops.splice(0..0, rest);
                    self.requeue(&roots);
                    return Ok(());
                }
                Ok(s) if s.failed > 0 => self.warn(crop_note(&s, folder)),
                Ok(s) => self.note(crop_note(&s, folder)),
                Err(e) => self.warn(format!("{e:#}")),
            }
        }

        let library: Vec<String> = catalog.roots()?.into_iter().map(|r| r.path).collect();
        for (i, root) in roots.iter().enumerate() {
            // Removed from the library since it was asked for.
            if !library.iter().any(|l| is_under(root, l)) {
                continue;
            }
            match scan::scan(catalog, root, &self.ctl) {
                Ok(s) => {
                    // A scan stopped early has committed what it recorded so far.
                    if s.changed > 0 || s.missing > 0 {
                        self.scans_done.fetch_add(1, Ordering::Relaxed);
                    }
                    if !s.complete {
                        self.requeue(&roots[i..]);
                        return Ok(());
                    }
                    if s.errors > 0 {
                        self.warn(format!("{root}: {} folders couldn't be read", s.errors));
                    }
                }
                Err(e) => {
                    self.warn(format!("{root}: {e:#}"));
                    // Its batches before the error are in.
                    self.scans_done.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        let s = import::import(catalog, &Options::new(Priority::BelowNormal), false, &self.ctl)?;
        if s.files > 0 {
            self.imports_done.fetch_add(1, Ordering::Relaxed);
        }
        if self.ctl.stop_requested() {
            return Ok(());
        }

        // Its own pass, after the import: the import's decode is reduced, the detector wants full size.
        let s = borders::find(catalog, &Options::new(Priority::BelowNormal), &self.ctl)?;
        if s.files > 0 {
            self.imports_done.fetch_add(1, Ordering::Relaxed);
        }
        if self.ctl.stop_requested() {
            return Ok(());
        }

        let height = self.thumb_height.load(Ordering::Relaxed);
        let opts = Options::new(Priority::Idle);
        let s = thumbs::run(self.thumbs, &self.catalog_path, height, &opts, &self.ctl)?;
        if s.cache_full {
            self.warn(format!("The thumbnail cache is full: {} thumbnails at {height} px weren't made", s.made));
        }
        Ok(())
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
