//! The library (the folders the user added) and the background work that keeps the catalog and the files in step: the
//! job that scans, imports, makes thumbnails and saves cropped copies, and the writer that puts ratings and turns into
//! the files. Both run in `pg_app` (`App::library`), which announces what changed on one bus. A thread here subscribes
//! to that bus and queues each change onto the Qt thread, which reads the new state and updates the properties and
//! emits `scanned`, `imported` and `filesChanged`. QML doesn't poll.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
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

        #[qinvokable]
        #[cxx_name = "addFolder"]
        fn add_folder(self: Pin<&mut Library>, url: &QUrl);

        #[qinvokable]
        #[cxx_name = "removeFolder"]
        fn remove_folder(self: Pin<&mut Library>, path: &QString);

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
use std::sync::OnceLock;

use cxx_qt::Threading;
use cxx_qt_lib::{QString, QStringList, QUrl};
use pg_client::text;
use pg_jobs::control::{Phase, Progress};
use pg_library::Change;
use pg_library::runner::Notice;

use crate::app;

/// The library, once `start` has started it: what the change pump and the folder operations read.
static LIBRARY: OnceLock<&'static pg_app::Library> = OnceLock::new();

#[derive(Default)]
pub struct LibraryRust {
    roots: QStringList,
    busy: bool,
    paused: bool,
    status: QString,
    progress: f64,
    message: QString,
    message_warning: bool,
    writes: QString,
    write_failed: bool,
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
        Notice::RoundFailed { error } | Notice::CropFailed { error } => error.clone(),
        Notice::RoundPanicked { message } => text::round_panicked(message),
        Notice::CacheFull { height, made } => text::cache_full(*height, *made),
        Notice::Cropped { folder, saved, skipped, failed, first_error } => {
            text::cropped(*saved, &folder.display().to_string(), *skipped, *failed, first_error.as_deref())
        }
    }
}

impl qobject::Library {
    fn show(mut self: Pin<&mut Self>, text: String, warning: bool) {
        self.as_mut().set_message_warning(warning);
        self.set_message(QString::from(&text));
    }

    fn load_roots(self: Pin<&mut Self>) {
        let roots = app::get().roots().unwrap_or_else(|e| {
            eprintln!("library: {e}");
            Vec::new()
        });
        self.set_roots(roots.iter().map(QString::from).collect());
    }

    fn start(mut self: Pin<&mut Self>, thumb_height: i32) {
        if LIBRARY.get().is_some() {
            return;
        }
        self.as_mut().load_roots();
        let library = app::get().library(thumb_height.max(1) as u32);
        // Subscribed before the pump starts, so no change is missed between the start and the first one it handles.
        let changes = library.changes().subscribe();
        let _ = LIBRARY.set(library);
        let thread = self.qt_thread();
        std::thread::Builder::new()
            .name("library changes".into())
            .spawn(move || {
                while let Some(change) = changes.recv() {
                    // Fails only when the window is gone: the pump ends with it.
                    if thread.queue(move |library| library.on_change(change)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn the library's change pump");
        // The state as it is now, for what was announced before the pump.
        self.as_mut().refresh_progress();
        self.as_mut().refresh_writes();
    }

    /// One change from the job or the writer, on the Qt thread.
    fn on_change(mut self: Pin<&mut Self>, change: Change) {
        match change {
            Change::Progress => self.as_mut().refresh_progress(),
            Change::Notice => self.as_mut().show_notices(),
            Change::Writes => self.as_mut().refresh_writes(),
            Change::Turned => {
                // A turned file's analysis is stale: the job re-analyzes it, and the views reload it.
                if let Some(library) = LIBRARY.get() {
                    library.wake_job();
                }
                self.as_mut().files_changed();
            }
            Change::Scanned => self.as_mut().scanned(),
            Change::Imported => self.as_mut().imported(),
            // The job's rounds, counted as stages for measuring (docs/PROFILING.md).
            Change::Round => pg_trace::pg_stage_round_done(),
        }
    }

    /// The job's pass and its pause: the status line, the busy state and the progress.
    fn refresh_progress(mut self: Pin<&mut Self>) {
        let Some(library) = LIBRARY.get() else { return };
        let status = library.status();
        let p = &status.progress;
        let line = match (status.paused, progress_status(p)) {
            (true, s) if s.is_empty() => "Background work paused".to_string(),
            (true, s) => format!("Paused · {s}"),
            (false, s) => s,
        };
        self.as_mut().set_busy(p.phase != Phase::Idle);
        self.as_mut().set_paused(status.paused);
        self.as_mut().set_status(QString::from(&line));
        self.as_mut().set_progress(if p.total > 0 { p.fraction() } else { -1.0 });
    }

    /// The writer's queue and its latest failure.
    fn refresh_writes(mut self: Pin<&mut Self>) {
        let Some(library) = LIBRARY.get() else { return };
        let s = library.status().writes;
        let writes = text::write_status(s.queued, s.error.as_deref());
        if self.writes.to_string() != writes {
            self.as_mut().set_writes(QString::from(&writes));
        }
        self.as_mut().set_write_failed(s.error.is_some());
    }

    /// The newest notice from the job, with how many others came in since the last one: the job keeps the newest and
    /// counts what it dropped, so none goes unannounced.
    fn show_notices(self: Pin<&mut Self>) {
        let Some(library) = LIBRARY.get() else { return };
        let taken = library.take_messages();
        if let Some(last) = taken.kept.back() {
            let more = taken.kept.len() - 1 + taken.not_kept;
            let text = notice_text(last);
            let text = if more == 0 { text } else { format!("{text} (and {more} more)") };
            self.show(text, last.is_warning());
        }
    }

    fn add_folder(mut self: Pin<&mut Self>, url: &QUrl) {
        let path = url.to_local_file().map_or_else(|| url.to_qstring().to_string(), |p| p.to_string());
        match app::get().add_folder(&path) {
            Ok((root, added)) => {
                // A folder already in the library is rescanned (the app scans it either way).
                if !added {
                    self.as_mut().show(format!("{root} is already in the library: rescanning it"), false);
                }
                self.as_mut().load_roots();
            }
            Err(e) => self.show(format!("{e}"), true),
        }
    }

    fn remove_folder(mut self: Pin<&mut Self>, path: &QString) {
        match app::get().remove_folder(&path.to_string()) {
            Ok(()) => {
                self.as_mut().load_roots();
                self.scanned();
            }
            Err(e) => self.show(format!("{e}"), true),
        }
    }

    fn rescan(self: Pin<&mut Self>) {
        if let Err(e) = app::get().rescan() {
            self.show(format!("{e}"), true);
        }
    }

    fn set_thumb_height(&self, height: i32) {
        if let Some(library) = LIBRARY.get() {
            library.set_thumb_height(height.max(1) as u32);
        }
    }

    fn toggle_pause(mut self: Pin<&mut Self>) {
        let Some(library) = LIBRARY.get() else { return };
        library.set_paused(!library.is_paused());
        self.as_mut().refresh_progress();
    }
}
