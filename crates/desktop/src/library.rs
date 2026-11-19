//! The library (the folders the user added) and the background work that keeps the catalog and the files in step: the
//! job that scans, imports, makes thumbnails and saves cropped copies, and the writer that puts ratings and turns into
//! the files. Both run in `pg_app` (`App::library`), which announces what changed on one bus. A thread here subscribes
//! to that bus and hands the changes to the Qt thread in passes: one pass is queued at a time, and it handles what came
//! in meanwhile, each kind once. The Qt thread reads the new state and updates the properties and emits `scanned`,
//! `imported` and `filesChanged`. QML doesn't poll.

pub(crate) mod state;

use crate::library_bridge::qobject;

use core::pin::Pin;
use std::sync::Mutex;

use std::path::Path;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use pg_app::reexport::{Cause, Change, WriteError};
use pg_client::text::{self, Message};
use pg_model::notice::Notice;
use pg_model::progress::Phase;
use pg_model::size::ThumbHeight;

use crate::app;
use crate::picker;
use crate::picker::state::Purpose;
use crate::pump::Pending;

/// The library, once `start` has started it: what the change pump, the folder operations and the lane rows read.
pub(crate) fn running() -> Option<&'static pg_app::Library> {
    app::get().library().running()
}

/// The grid's thumbnail height, as the shell reports it on each change. The service holds it from before the job starts,
/// so the job makes the thumbnails at it whenever it runs.
pub(crate) fn set_thumb_height(height: ThumbHeight) {
    app::get().library().set_thumb_height(height);
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

/// Why a job failed on a file: the error's text, or the words for a panic the job caught on it.
fn cause_text(cause: &Cause) -> String {
    match cause {
        Cause::Error(error) => error.clone(),
        Cause::Panicked(message) => text::render(&Message::FilePanicked { message: message.clone() }),
    }
}

/// Why a folder's path couldn't be added, in the client's words.
fn path_words(e: &pg_platform::names::NormalizeError) -> String {
    use pg_platform::names::{NormalizeError, shown};
    match e {
        NormalizeError::Unreadable { path, error } => {
            text::render(&Message::PathUnreadable { path: shown(path), error: error.to_string() })
        }
        NormalizeError::NotAPath { path } => text::render(&Message::NotAPath { path: shown(path) }),
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

    pub(crate) fn start(mut self: Pin<&mut Self>) {
        if running().is_some() {
            return;
        }
        self.as_mut().load_roots();
        let library = match app::get().library().start() {
            Ok(library) => library,
            Err(e) => {
                // Nothing is scanned, imported or written: the inbox says so, not only the log.
                app::post(Notice::LibraryUnavailable { error: app::error_text(&e) });
                return;
            }
        };
        // Subscribed before the pump starts, so no change is missed between the start and the first one it handles.
        let changes = library.changes().subscribe();
        // Without its thread the window isn't told the library's changes; the log says why.
        let pump = crate::pump::start(
            "library changes",
            self.qt_thread(),
            move || changes.recv(),
            Changes::add,
            |library, pending| library.on_pass(&pending),
        );
        if let Err(e) = pump {
            pg_log::warn("library", &format!("the change pump didn't start: {e}"));
        }
        // The state as it is now, for what was announced before the pump.
        self.as_mut().refresh_progress();
        self.as_mut().refresh_writes();
    }

    /// One pass on the Qt thread: the changes that came in since the last one, each kind handled once, in this order.
    fn on_pass(mut self: Pin<&mut Self>, pending: &Mutex<Pending<Changes>>) {
        let taken = Pending::take(pending);
        if taken.progress {
            let failed = self.as_mut().refresh_progress();
            // While a pass has failed files, the inbox's "N failed" link follows the pass: it is poked here, not left to
            // the inbox's poll (30 s) or to the job's next round.
            if failed > 0 {
                app::get().inbox().poke();
            }
        }
        if taken.writes {
            self.as_mut().refresh_writes();
        }
        if taken.turned {
            // A turned file's analysis is stale: the job re-analyzes it, and the views reload it.
            if let Some(library) = running() {
                library.wake_turned();
            }
            self.as_mut().files_changed();
        }
        if taken.scanned {
            self.as_mut().scanned();
        }
        if taken.imported {
            self.as_mut().imported();
        }
        // The job's rounds, counted as stages for measuring. A round's passes write on their own
        // connections: the inbox looks for what they changed.
        for _ in 0..taken.rounds {
            pg_trace::pg_stage_round_done();
        }
        if taken.rounds > 0 {
            app::get().inbox().poke();
        }
    }

    /// The job's busy state, then the lane rows are told (`progressed`), which read the job's rows themselves. Returns the
    /// pass's failed count (0 when no job runs), which `on_pass` pokes the inbox on.
    fn refresh_progress(mut self: Pin<&mut Self>) -> u64 {
        let Some(library) = running() else { return 0 };
        let p = &library.status().progress;
        self.as_mut().set_busy(p.phase != Phase::Idle);
        self.as_mut().progressed();
        p.failed
    }

    /// The writer's queue and its latest failure, on the status line: display only. The inbox hears of the writer from
    /// the writer and the library themselves (`write-failed` and `write-dropped` from the writer as its status changes,
    /// `write-stopped` where its restarts run out), never from here.
    fn refresh_writes(mut self: Pin<&mut Self>) {
        let Some(library) = running() else { return };
        let s = library.status().writes;
        let error = s.error.as_ref().map(|e| match e {
            WriteError::Failed(cause) | WriteError::CopyIncomplete { cause, .. } => cause_text(cause),
        });
        let writes = text::write_status(s.queued, error.as_deref());
        if self.writes.to_string() != writes {
            self.as_mut().set_writes(QString::from(&writes));
        }
        self.as_mut().set_write_failed(s.error.is_some());
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

    /// Pauses (`paused`) or resumes the lane numbered `number`, as its row showed it: a command, not a toggle, so a click on
    /// a row that has since changed sets the state the user asked for. The crop's -1 and a lane that has retired do nothing.
    pub(crate) fn set_lane_paused(mut self: Pin<&mut Self>, number: i32, paused: bool) {
        let Some(library) = running() else { return };
        if let Ok(number) = usize::try_from(number) {
            library.set_lane_paused(number, paused);
        }
        self.as_mut().refresh_progress();
    }
}
