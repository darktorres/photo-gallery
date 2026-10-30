//! The app's background job (PLAN §4, §6): one thread that scans the folders asked for, imports whatever is
//! pending, then makes thumbnails at the grid's current height, and sleeps until there is something new to do.
//!
//! A new request (a folder added, a rescan, a new thumbnail height) stops the pass in progress at the next file;
//! it commits what it has and the next round starts with the new work. Every pass is resumable from the catalog,
//! so stopping costs nothing but the batch in flight.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use anyhow::Result;
use pg_core::catalog::{Catalog, is_under};
use pg_core::thumbs::ThumbCache;
use windows_sys::Win32::System::Threading::{THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_IDLE};

use crate::control::{Control, Progress};
use crate::pipeline::Options;
use crate::{import, scan, thumbs};

#[derive(Default)]
struct Queue {
    /// Folders to scan, normalized.
    scans: Vec<String>,
    /// Something to do: start a round.
    wake: bool,
}

struct Inner {
    catalog_path: PathBuf,
    thumbs: &'static Mutex<ThumbCache>,
    ctl: Control,
    queue: Mutex<Queue>,
    wake: Condvar,
    thumb_height: AtomicU32,
    scans_done: AtomicU64,
    imports_done: AtomicU64,
    messages: Mutex<Vec<String>>,
}

/// What the job has done so far, for the app to refresh its views.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Generation {
    /// Completed folder scans (the folder tree changed).
    pub scans: u64,
    /// Import passes that analyzed something (sizes and placeholders changed).
    pub imports: u64,
}

pub struct Runner {
    inner: Arc<Inner>,
}

impl Runner {
    /// Start the job thread. It begins by importing whatever is pending and making the thumbnails still missing
    /// at `thumb_height`.
    pub fn start(catalog_path: PathBuf, thumbs: &'static Mutex<ThumbCache>, thumb_height: u32) -> Runner {
        let inner = Arc::new(Inner {
            catalog_path,
            thumbs,
            ctl: Control::new(false),
            queue: Mutex::new(Queue { scans: Vec::new(), wake: true }),
            wake: Condvar::new(),
            thumb_height: AtomicU32::new(thumb_height),
            scans_done: AtomicU64::new(0),
            imports_done: AtomicU64::new(0),
            messages: Mutex::new(Vec::new()),
        });
        let job = inner.clone();
        std::thread::Builder::new().name("background job".into()).spawn(move || job.run()).expect("spawn job");
        Runner { inner }
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
    pub fn take_messages(&self) -> Vec<String> {
        std::mem::take(&mut self.inner.messages.lock().unwrap())
    }
}

impl Inner {
    fn wake_up(&self, mut q: std::sync::MutexGuard<Queue>) {
        q.wake = true;
        self.ctl.request_stop();
        self.wake.notify_one();
    }

    fn note(&self, message: String) {
        self.messages.lock().unwrap().push(message);
    }

    fn run(&self) {
        loop {
            let roots = {
                let mut q = self.queue.lock().unwrap();
                while !q.wake {
                    q = self.wake.wait(q).unwrap();
                }
                q.wake = false;
                self.ctl.clear_stop();
                std::mem::take(&mut q.scans)
            };
            if let Err(e) = self.round(roots) {
                self.note(format!("{e:#}"));
            }
        }
    }

    /// Put back folders a stopped round didn't finish.
    fn requeue(&self, roots: &[String]) {
        let mut q = self.queue.lock().unwrap();
        let rest: Vec<String> = roots.iter().filter(|r| !q.scans.contains(r)).cloned().collect();
        q.scans.splice(0..0, rest);
        q.wake = true;
    }

    fn round(&self, roots: Vec<String>) -> Result<()> {
        let mut catalog = Catalog::open(&self.catalog_path)?;
        let library = catalog.roots()?;
        for (i, root) in roots.iter().enumerate() {
            // Removed from the library since it was asked for.
            if !library.iter().any(|l| is_under(root, l)) {
                continue;
            }
            match scan::scan(&mut catalog, root, &self.ctl) {
                Ok(s) if !s.complete => {
                    self.requeue(&roots[i..]);
                    return Ok(());
                }
                Ok(s) => {
                    if s.errors > 0 {
                        self.note(format!("{root}: {} folders couldn't be read", s.errors));
                    }
                }
                Err(e) => self.note(format!("{root}: {e:#}")),
            }
            self.scans_done.fetch_add(1, Ordering::Relaxed);
        }

        let s = import::import(&mut catalog, &Options::new(THREAD_PRIORITY_BELOW_NORMAL), false, &self.ctl)?;
        if s.files > 0 {
            self.imports_done.fetch_add(1, Ordering::Relaxed);
        }
        if self.ctl.stop_requested() {
            return Ok(());
        }

        let height = self.thumb_height.load(Ordering::Relaxed);
        let opts = Options::new(THREAD_PRIORITY_IDLE);
        let s = thumbs::run(self.thumbs, &self.catalog_path, height, &opts, &self.ctl)?;
        if s.cache_full {
            self.note(format!("The thumbnail cache is full: {} thumbnails at {height} px weren't made", s.made));
        }
        Ok(())
    }
}
