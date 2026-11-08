//! The library (the folders the user added) and the background work that keeps the catalog and the files in step: the
//! job that scans, imports, makes thumbnails and saves cropped copies, and the writer that puts ratings and turns into
//! the files. Both run in `pg_app` (`App::library`), which announces what changed on one bus. A thread here subscribes
//! to that bus and hands the changes to the Qt thread in passes: one pass is queued at a time, and it handles what came
//! in meanwhile, each kind once. The Qt thread reads the new state and updates the properties and emits `scanned`,
//! `imported` and `filesChanged`. QML doesn't poll.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QStringList, roots, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(bool, busy, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(bool, paused, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(QString, status, READ, WRITE, NOTIFY, FINAL)]
        /// Done ÷ total of the current pass; -1 when the total isn't known (scanning).
        #[qproperty(f64, progress, READ, WRITE, NOTIFY, FINAL)]
        /// The latest error or note from the job, until the next one.
        #[qproperty(QString, message, READ, WRITE, NOTIFY, FINAL)]
        /// Whether `message` is an error or a warning (otherwise news: a job done).
        #[qproperty(bool, message_warning, cxx_name = "messageWarning", READ, WRITE, NOTIFY, FINAL)]
        /// Ratings and turns still to be written into the files, and the latest failure; empty when none.
        #[qproperty(QString, writes, cxx_name = "writeStatus", READ, WRITE, NOTIFY, FINAL)]
        /// Whether the latest write failed (the status says why).
        #[qproperty(bool, write_failed, cxx_name = "writeFailed", READ, WRITE, NOTIFY, FINAL)]
        type Library = super::LibraryRust;
    }

    impl cxx_qt::Threading for Library {}

    unsafe extern "RustQt" {
        #[qsignal]
        fn scanned(self: Pin<&mut Library>);

        #[qsignal]
        fn imported(self: Pin<&mut Library>);

        /// Files were turned (rewritten or turned in the catalog): their images and sizes changed.
        #[qsignal]
        #[cxx_name = "filesChanged"]
        fn files_changed(self: Pin<&mut Library>);

        /// Start the background job and the writer; they first finish whatever is pending.
        #[qinvokable]
        fn start(self: Pin<&mut Library>, thumb_height: i32);

        /// Add the folder the picker chose for adding a folder (`FolderPicker`, purpose 0). The choice is taken here, not
        /// passed through QML.
        #[qinvokable]
        #[cxx_name = "addPicked"]
        fn add_picked(self: Pin<&mut Library>);

        /// Show `text` as the library's message, as a warning: a folder dialog's failure (the picker's `failed`).
        #[qinvokable]
        #[cxx_name = "showWarning"]
        fn show_warning(self: Pin<&mut Library>, text: &QString);

        /// Remove the library folder at `row` (its row in `roots`).
        #[qinvokable]
        #[cxx_name = "removeFolder"]
        fn remove_folder(self: Pin<&mut Library>, row: i32);

        /// Scan every library folder again (new, changed, moved and deleted files).
        #[qinvokable]
        fn rescan(self: Pin<&mut Library>);

        /// The grid's thumbnail height in device pixels; the job makes thumbnails at this one.
        #[qinvokable]
        #[cxx_name = "setThumbHeight"]
        fn set_thumb_height(self: &Library, height: i32);

        #[qinvokable]
        #[cxx_name = "togglePause"]
        fn toggle_pause(self: Pin<&mut Library>);
    }
}

use core::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use std::path::Path;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use pg_app::reexport::{Cause, Change, FileError, Notice, Phase, Progress, WriteError};
use pg_client::text;
use pg_model::ids::RootId;
use pg_model::rules::PathRules;

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
    notice: bool,
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
            Change::Notice => self.notice = true,
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

#[derive(Default)]
pub struct LibraryRust {
    roots: QStringList,
    /// The catalog's id of each folder in `roots`, row for row: what removing a row removes.
    root_ids: Vec<RootId>,
    busy: bool,
    paused: bool,
    status: QString,
    progress: f64,
    message: QString,
    message_warning: bool,
    writes: QString,
    write_failed: bool,
    /// The writer's count of dropped writes (`WriteStatus::dropped`) already shown: a note is shown when it grows.
    dropped_seen: usize,
}

/// The status line's text for a pass's progress; empty when idle.
fn progress_status(p: &Progress) -> String {
    let pass = match &p.phase {
        Phase::Idle => text::Pass::Idle,
        Phase::Scan { root } => text::Pass::Scan { root: root.as_str() },
        Phase::Import => text::Pass::Import,
        Phase::Borders => text::Pass::Borders,
        Phase::Crop => text::Pass::Crop,
        Phase::Thumbs { height } => text::Pass::Thumbs { height: *height },
    };
    let counts = text::Counts { done: p.done, total: p.total, failed: p.failed, rate: p.rate(), eta_secs: p.eta() };
    text::pass_status(pass, &counts)
}

/// The words for a notice from the job; the wording is `pg_client::text`'s.
fn notice_text(notice: &Notice) -> String {
    match notice {
        Notice::UnreadableFolders { root, count } => text::unreadable_folders(root, *count),
        Notice::ScanFailed { root, error } => text::scan_failed(root, error),
        Notice::RootRefused { root, reason } => text::root_refused(root, reason),
        Notice::ScanNote { root, note, count } => text::scan_note(root, note.kind(), *count),
        Notice::RoundFailed { error } => text::round_failed(error),
        Notice::CropFailed { error } => text::crop_failed(error),
        Notice::RoundPanicked { message } => text::round_panicked(message),
        Notice::CacheFull { height, made } => text::cache_full(*height, *made),
        Notice::Cropped { folder, saved, skipped, failed, first_error } => {
            let first_error =
                first_error.as_ref().map(|FileError { path, cause }| format!("{path}: {}", cause_text(cause)));
            text::cropped(*saved, &folder.display().to_string(), *skipped, *failed, first_error.as_deref())
        }
    }
}

/// Why a job failed on a file: the error's text, or the words for a panic the job caught on it.
fn cause_text(cause: &Cause) -> String {
    match cause {
        Cause::Error(error) => error.clone(),
        Cause::Panicked(message) => text::file_panicked(message),
    }
}

/// Why the writer's latest write failed, in words: the job's cause, a copy into a file that may be incomplete, or a
/// write refused for its folder's name.
fn write_error_text(error: &WriteError) -> String {
    match error {
        WriteError::Failed(cause) => cause_text(cause),
        WriteError::CopyIncomplete { file, temp, cause } => text::write_incomplete(file, temp, &cause_text(cause)),
        WriteError::FolderRefused { file, folder } => text::write_folder_refused(file, folder),
    }
}

impl qobject::Library {
    fn show(mut self: Pin<&mut Self>, text: String, warning: bool) {
        self.as_mut().set_message_warning(warning);
        self.set_message(QString::from(&text));
    }

    fn show_warning(self: Pin<&mut Self>, text: &QString) {
        self.show(String::from(text), true);
    }

    /// The library's folders, as the catalog has them. When they can't be read, none is listed and the panel says why.
    fn load_roots(mut self: Pin<&mut Self>) {
        let roots = match app::get().roots() {
            Ok(roots) => roots,
            Err(e) => {
                crate::panic_log::note(&format!("library: the folders couldn't be read: {e}"));
                self.as_mut().show(text::folders_unreadable(&e.to_string()), true);
                Vec::new()
            }
        };
        let texts: QStringList = roots.iter().map(|r| QString::from(&PathRules::NATIVE.display(&r.path))).collect();
        self.as_mut().rust_mut().root_ids = roots.iter().map(|r| r.id).collect();
        self.set_roots(texts);
    }

    fn start(mut self: Pin<&mut Self>, thumb_height: i32) {
        if LIBRARY.get().is_some() {
            return;
        }
        self.as_mut().load_roots();
        let library = match app::get().library(u32::try_from(thumb_height.max(1)).unwrap_or(1)) {
            Ok(library) => library,
            Err(e) => {
                // Nothing is scanned, imported or written: the panel says so, not only the log.
                crate::panic_log::note(&format!("library: {e}"));
                self.show(text::library_unavailable(&e.to_string()), true);
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
            crate::panic_log::note(&format!("library: the change pump didn't start: {e}"));
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
        if taken.notice {
            self.as_mut().show_notices();
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
        // The job's rounds, counted as stages for measuring (docs/PROFILING.md).
        for _ in 0..taken.rounds {
            pg_trace::pg_stage_round_done();
        }
    }

    /// The job's pass and its pause: the status line, the busy state and the progress.
    fn refresh_progress(mut self: Pin<&mut Self>) {
        let Some(library) = LIBRARY.get() else { return };
        let status = library.status();
        let p = &status.progress;
        let pass = progress_status(p);
        let line = if status.paused { text::paused_status(&pass) } else { pass };
        self.as_mut().set_busy(p.phase != Phase::Idle);
        self.as_mut().set_paused(status.paused);
        self.as_mut().set_status(QString::from(&line));
        self.as_mut().set_progress(if p.total > 0 { p.fraction() } else { -1.0 });
    }

    /// The writer's queue and its latest failure; and a note, once, for the writes dropped since the last one (a drop
    /// is not a failure: it doesn't light `write_failed`).
    fn refresh_writes(mut self: Pin<&mut Self>) {
        let Some(library) = LIBRARY.get() else { return };
        let s = library.status().writes;
        let error = s.error.as_ref().map(write_error_text);
        let writes = text::write_status(s.queued, error.as_deref());
        if self.writes.to_string() != writes {
            self.as_mut().set_writes(QString::from(&writes));
        }
        self.as_mut().set_write_failed(s.error.is_some());
        if s.dropped > self.dropped_seen {
            let new = s.dropped.saturating_sub(self.dropped_seen);
            self.as_mut().rust_mut().dropped_seen = s.dropped;
            if let Some(kind) = s.last_dropped {
                self.as_mut().show(text::write_dropped(kind, new), true);
            }
        }
    }

    /// The newest notice from the job, with how many others came in since the last one: the job keeps the newest and
    /// counts what it dropped, so none goes unannounced.
    fn show_notices(self: Pin<&mut Self>) {
        let Some(library) = LIBRARY.get() else { return };
        let taken = library.take_messages();
        if let Some(last) = taken.kept.back() {
            let more = taken.kept.len() - 1 + taken.not_kept;
            self.show(text::notice_more(notice_text(last), more), last.is_warning());
        }
    }

    fn add_picked(mut self: Pin<&mut Self>) {
        if let Some(path) = picker::take(Purpose::AddFolder) {
            self.as_mut().add_path(&path);
        }
    }

    /// Add the folder at `path` (exact), and say so when it was already there.
    fn add_path(mut self: Pin<&mut Self>, path: &Path) {
        match app::get().add_folder(path) {
            Ok((root, added)) => {
                // A folder already in the library is rescanned (the app scans it either way).
                if !added {
                    self.as_mut().show(text::folder_already_added(&root), false);
                }
                self.as_mut().load_roots();
            }
            Err(pg_app::Error::FolderRefused { folder, reason }) => {
                self.show(text::folder_refused(&folder, &reason), true);
            }
            Err(e) => self.show(text::folder_not_added(&e.to_string()), true),
        }
    }

    fn remove_folder(mut self: Pin<&mut Self>, row: i32) {
        let Some(id) = usize::try_from(row).ok().and_then(|r| self.rust().root_ids.get(r).copied()) else {
            return;
        };
        match app::get().remove_folder(id) {
            Ok(()) => {
                self.as_mut().load_roots();
                self.scanned();
            }
            Err(e) => self.show(text::folder_not_removed(&e.to_string()), true),
        }
    }

    fn rescan(self: Pin<&mut Self>) {
        if let Err(e) = app::get().rescan() {
            self.show(text::rescan_failed(&e.to_string()), true);
        }
    }

    fn set_thumb_height(&self, height: i32) {
        if let Some(library) = LIBRARY.get() {
            library.set_thumb_height(u32::try_from(height.max(1)).unwrap_or(1));
        }
    }

    fn toggle_pause(mut self: Pin<&mut Self>) {
        let Some(library) = LIBRARY.get() else { return };
        library.set_paused(!library.is_paused());
        self.as_mut().refresh_progress();
    }
}
