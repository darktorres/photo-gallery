//! The library (the folders the user added) and the background work that keeps the catalog and the files in step: the
//! job that scans, imports, makes thumbnails and saves cropped copies, and the writer that puts ratings and turns into
//! the files. Both run in `pg_app` (`App::library`), which announces what changed on one bus. A thread here subscribes
//! to that bus and hands the changes to the Qt thread in passes: one pass is queued at a time, and it handles what came
//! in meanwhile, each kind once. The Qt thread reads the new state and updates the properties and emits `scanned`,
//! `imported` and `filesChanged`. QML doesn't poll.

pub(crate) mod state;

use crate::library_bridge::qobject;

use core::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use std::path::Path;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use pg_app::reexport::{Cause, Change, WriteError};
use pg_client::text;
use pg_model::notice::{FilePass, Notice};
use pg_model::progress::Phase;

use crate::app;
use crate::picker::{self, Purpose};

/// The library, once `start` has started it: what the change pump and the folder operations read.
static LIBRARY: OnceLock<&'static pg_app::Library> = OnceLock::new();

/// The changes the pump has taken off the bus and the Qt thread hasn't handled, and whether a pass is queued for them.
/// At most one pass is queued at a time, so the Qt thread runs one per event-loop turn at most.
#[derive(Default)]
struct Pending {
    changes: Changes,
    queued: bool,
}

/// What the pending changes come to: each kind is handled once per pass, however many came in. The rounds are counted.
#[derive(Default)]
struct Changes {
    progress: bool,
    writes: bool,
    turned: bool,
    scanned: bool,
    imported: bool,
    rounds: u32,
}

impl Changes {
    fn add(&mut self, change: Change) {
        match change {
            Change::Progress => self.progress = true,
            Change::Writes => self.writes = true,
            Change::Turned => self.turned = true,
            Change::Scanned => self.scanned = true,
            Change::Imported => self.imported = true,
            Change::Round => self.rounds += 1,
        }
    }
}

fn lock(shared: &Mutex<Pending>) -> MutexGuard<'_, Pending> {
    // A panic on the other side leaves the changes as they were: the next pass handles them.
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why a job failed on a file: the error's text, or the words for a panic the job caught on it.
fn cause_text(cause: &Cause) -> String {
    match cause {
        Cause::Error(error) => error.clone(),
        Cause::Panicked(message) => text::file_panicked(message),
    }
}

/// The notice of the writer's latest failure: its cause, and for a copy that may be incomplete, the file and its copy.
fn write_failed(error: &WriteError) -> Notice {
    let (cause, file, temp) = match error {
        WriteError::Failed(cause) => (cause, None, None),
        WriteError::CopyIncomplete { file, temp, cause } => (cause, Some(file.clone()), Some(temp.clone())),
    };
    let (error, panicked) = match cause {
        Cause::Error(text) => (app::error_text(text), false),
        Cause::Panicked(message) => (app::error_text(message), true),
    };
    Notice::WriteFailed { error, panicked, file, temp }
}
/// Why a folder's path couldn't be added, in the client's words.
fn path_words(e: &pg_platform::names::NormalizeError) -> String {
    use pg_platform::names::{NormalizeError, shown};
    match e {
        NormalizeError::Unreadable { path, error } => text::path_unreadable(&shown(path), &error.to_string()),
        NormalizeError::NotAPath { path } => text::not_a_path(&shown(path)),
    }
}

impl qobject::Library {
    /// The library's folders, as the catalog has them. When they can't be read, none is listed and the inbox says why.
    fn load_roots(mut self: Pin<&mut Self>) {
        let roots = match app::get().library().roots() {
            Ok(roots) => roots,
            Err(e) => {
                app::post(Notice::FoldersUnreadable { error: app::error_text(&e) });
                Vec::new()
            }
        };
        let texts: QStringList =
            roots.iter().map(|r| QString::from(&pg_platform::native::PATH_RULES.display(&r.path))).collect();
        self.as_mut().rust_mut().root_ids = roots.iter().map(|r| r.id).collect();
        self.set_roots(texts);
    }

    pub(crate) fn start(mut self: Pin<&mut Self>, thumb_height: i32) {
        if LIBRARY.get().is_some() {
            return;
        }
        self.as_mut().load_roots();
        let library = match app::get().library().start(u32::try_from(thumb_height.max(1)).unwrap_or(1)) {
            Ok(library) => library,
            Err(e) => {
                // Nothing is scanned, imported or written: the inbox says so, not only the log.
                app::post(Notice::LibraryUnavailable { error: app::error_text(&e) });
                return;
            }
        };
        // Subscribed before the pump starts, so no change is missed between the start and the first one it handles.
        let changes = library.changes().subscribe();
        let _ = LIBRARY.set(library);
        let shared: Arc<Mutex<Pending>> = Arc::default();
        let thread = self.qt_thread();
        // Without its thread the window isn't told the library's changes; the log says why.
        let pump = std::thread::Builder::new().name("library changes".into()).spawn(move || {
            while let Some(change) = changes.recv() {
                // Queued only when no pass is: a pass that is queued reads what this change is added to.
                let queue = {
                    let mut pending = lock(&shared);
                    pending.changes.add(change);
                    !std::mem::replace(&mut pending.queued, true)
                };
                if queue {
                    let pass = Arc::clone(&shared);
                    // Fails only when the window is gone: the pump ends with it.
                    if thread.queue(move |library| library.on_pass(pass)).is_err() {
                        break;
                    }
                }
            }
        });
        if let Err(e) = pump {
            pg_log::warn("library", &format!("the change pump didn't start: {e}"));
        }
        // The state as it is now, for what was announced before the pump.
        self.as_mut().refresh_progress();
        self.as_mut().refresh_writes();
    }

    /// One pass on the Qt thread: the changes that came in since the last one, each kind handled once, in this order.
    fn on_pass(mut self: Pin<&mut Self>, shared: Arc<Mutex<Pending>>) {
        let taken = {
            let mut pending = lock(&shared);
            pending.queued = false;
            std::mem::take(&mut pending.changes)
        };
        if taken.progress {
            self.as_mut().refresh_progress();
        }
        if taken.writes {
            self.as_mut().refresh_writes();
        }
        if taken.turned {
            // A turned file's analysis is stale: the job re-analyzes it, and the views reload it.
            if let Some(library) = LIBRARY.get() {
                library.wake_job();
            }
            self.as_mut().files_changed();
        }
        if taken.scanned {
            self.as_mut().scanned();
        }
        if taken.imported {
            self.as_mut().imported();
        }
        // The job's rounds, counted as stages for measuring (docs/PROFILING.md). A round's passes write on their own
        // connections: the inbox looks for what they changed.
        for _ in 0..taken.rounds {
            pg_trace::pg_stage_round_done();
        }
        if taken.rounds > 0 {
            app::get().inbox().poke();
        }
    }

    /// The job's pass and its pause: the status line, the busy state and the progress.
    fn refresh_progress(mut self: Pin<&mut Self>) {
        let Some(library) = LIBRARY.get() else { return };
        let status = library.status();
        let p = &status.progress;
        // A scanned folder is shown under the catalog's rules, this machine's.
        let pass = text::pass_status(p, pg_platform::native::PATH_RULES);
        let line = if status.paused { text::paused_status(&pass) } else { pass };
        self.as_mut().set_busy(p.phase != Phase::Idle);
        self.as_mut().set_paused(status.paused);
        self.as_mut().set_status(QString::from(&line));
        self.as_mut().set_progress(if p.total > 0 { p.fraction() } else { -1.0 });
        let failed = QString::from(&text::pass_failed(p));
        if self.failed != failed {
            self.as_mut().set_failed(failed);
        }
        let pass = match p.phase {
            Phase::Thumbs { .. } => Some(FilePass::Thumbs),
            Phase::Borders => Some(FilePass::Borders),
            Phase::Import => Some(FilePass::Import),
            Phase::Idle | Phase::Scan { .. } | Phase::Crop => None,
        };
        self.as_mut().set_failed_pass(crate::failed_files::pass_code(pass));
    }

    /// The writer's queue and its latest failure, on the status line. The inbox is told a failure once (the same one
    /// again is not news) and that it went once a write has gone through; that the writer stopped, once; and the writes
    /// dropped since the last time (a drop is not a failure: it doesn't light `write_failed`).
    fn refresh_writes(mut self: Pin<&mut Self>) {
        let Some(library) = LIBRARY.get() else { return };
        let status = library.status();
        if status.writer_stopped && !self.writer_stopped_shown {
            self.as_mut().rust_mut().writer_stopped_shown = true;
            app::post(Notice::WriteStopped);
        }
        let s = status.writes;
        let failed = s.error.as_ref().map(write_failed);
        if failed != self.write_failure {
            match &failed {
                Some(notice) => app::post(notice.clone()),
                None => app::resolved("write-failed"),
            }
            self.as_mut().rust_mut().write_failure = failed;
        }
        let error = s.error.as_ref().map(|e| match e {
            WriteError::Failed(cause) | WriteError::CopyIncomplete { cause, .. } => cause_text(cause),
        });
        let writes = text::write_status(s.queued, error.as_deref());
        if self.writes.to_string() != writes {
            self.as_mut().set_writes(QString::from(&writes));
        }
        self.as_mut().set_write_failed(s.error.is_some());
        if s.dropped > self.dropped_seen {
            let new = s.dropped.saturating_sub(self.dropped_seen);
            self.as_mut().rust_mut().dropped_seen = s.dropped;
            if let Some(write) = s.last_dropped {
                app::post(Notice::WriteDropped { write, count: u64::try_from(new).unwrap_or(u64::MAX) });
            }
        }
    }
    pub(crate) fn add_picked(mut self: Pin<&mut Self>) {
        if let Some(path) = picker::take(Purpose::AddFolder) {
            self.as_mut().add_path(&path);
        }
    }

    /// Add the folder at `path` (exact), and say so when it was already there, or why it wasn't added.
    fn add_path(mut self: Pin<&mut Self>, path: &Path) {
        match app::get().library().add_folder(path) {
            Ok((root, added)) => {
                // A folder already in the library is rescanned (the app scans it either way).
                if !added {
                    app::post(Notice::FolderAlreadyAdded { folder: root });
                }
                self.as_mut().load_roots();
            }
            Err(pg_app::Error::FolderRefused { folder, reason }) => app::post(Notice::FolderRefused { folder, reason }),
            Err(pg_app::Error::FolderPath(e)) => app::post(Notice::FolderNotAdded { error: path_words(&e) }),
            Err(e) => app::post(Notice::FolderNotAdded { error: app::error_text(&e) }),
        }
    }

    pub(crate) fn remove_folder(mut self: Pin<&mut Self>, row: i32) {
        let Some(id) = usize::try_from(row).ok().and_then(|r| self.rust().root_ids.get(r).copied()) else {
            return;
        };
        match app::get().library().remove_folder(id) {
            Ok(()) => {
                self.as_mut().load_roots();
                self.scanned();
            }
            Err(e) => app::post(Notice::FolderNotRemoved { error: app::error_text(&e) }),
        }
    }

    pub(crate) fn rescan(self: Pin<&mut Self>) {
        if let Err(e) = app::get().library().rescan() {
            app::post(Notice::RescanFailed { error: app::error_text(&e) });
        }
    }

    pub(crate) fn set_thumb_height(&self, height: i32) {
        if let Some(library) = LIBRARY.get() {
            library.set_thumb_height(u32::try_from(height.max(1)).unwrap_or(1));
        }
    }

    pub(crate) fn toggle_pause(mut self: Pin<&mut Self>) {
        let Some(library) = LIBRARY.get() else { return };
        library.set_paused(!library.is_paused());
        self.as_mut().refresh_progress();
    }
}
