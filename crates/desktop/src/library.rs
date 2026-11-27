//! The library (the folders the user added) and the background work that keeps the catalog and the files in step: the
//! job that scans, imports, makes thumbnails and saves cropped copies, and the writer that puts ratings and turns into
//! the files. Both run in `pg_app` (`App::library`), which announces what changed on one bus. A thread here subscribes
//! to that bus and hands the changes to the Qt thread in passes: one pass is queued at a time, and it handles what came
//! in meanwhile, each kind once. The Qt thread reads the new state and updates the properties and emits `scanned`,
//! `imported` and `filesChanged`. QML doesn't poll.

use crate::bridges::library_bridge::qobject;

use core::pin::Pin;
use std::sync::Mutex;

use std::path::Path;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use pg_model::job::{Cause, Change};
use pg_model::notice::Notice;
use pg_model::progress::Phase;
use pg_model::removal::Begun;
use pg_model::size::ThumbHeight;
use pg_model::write::WriteError;
use pg_wording::Message;

use crate::app;
use crate::bridges::picker_state::Purpose;
use crate::picker;
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
        Cause::Error(error) => pg_wording::render(&Message::FailureDetail { error: error.clone() }),
        Cause::Panicked(message) => pg_wording::render(&Message::FilePanicked { message: message.clone() }),
    }
}

/// Why a folder's path couldn't be added, in the client's words.
fn path_words(e: &pg_platform::names::NormalizeError) -> String {
    use pg_platform::names::{NormalizeError, shown};
    match e {
        NormalizeError::Unreadable { path, error } => {
            pg_wording::render(&Message::PathUnreadable { path: shown(path), error: error.to_string() })
        }
        NormalizeError::NotAPath { path } => pg_wording::render(&Message::NotAPath { path: shown(path) }),
    }
}

impl qobject::Library {
    /// The library's folders, as the catalog has them. When they can't be read, the inbox says why and the list stays as
    /// it was: a read that failed is not taken for an empty library (the grid's "no folders" text, the panel's "no folders
    /// yet", and the row numbers a removal is asked by all follow the list).
    fn load_roots(mut self: Pin<&mut Self>) {
        let roots = match app::get().library().roots() {
            Ok(roots) => roots,
            Err(e) => {
                app::post(Notice::FoldersUnreadable { error: app::error_text(&e) });
                self.as_mut().load_removals();
                return;
            }
        };
        let texts: QStringList =
            roots.iter().map(|r| QString::from(&pg_platform::native::PATH_RULES.display(&r.path))).collect();
        self.as_mut().rust_mut().root_ids = roots.iter().map(|r| r.id).collect();
        self.as_mut().rust_mut().root_paths = roots.iter().map(|r| r.path.clone()).collect();
        self.as_mut().load_removals();
        self.set_roots(texts);
    }

    /// The removals not finished, listed under the folders: each with its state (kept for an unfinished write, or still
    /// removing). When they can't be read, the inbox says why and the list stays as it was.
    fn load_removals(mut self: Pin<&mut Self>) {
        let pending = match app::get().library().pending_removals() {
            Ok(pending) => pending,
            Err(e) => {
                app::post(Notice::FoldersUnreadable { error: app::error_text(&e) });
                return;
            }
        };
        let folders = |kept: bool| -> QStringList {
            pending
                .iter()
                .filter(|(_, deferred)| (*deferred > 0) == kept)
                .map(|(path, _)| QString::from(&pg_platform::native::PATH_RULES.display(path)))
                .collect()
        };
        // The exact paths of the kept removals, row for row with their texts: Discard is asked for by row.
        let kept_paths: Vec<_> =
            pending.iter().filter(|(_, deferred)| *deferred > 0).map(|(path, _)| path.clone()).collect();
        let kept = folders(true);
        let removing = folders(false);
        self.as_mut().rust_mut().kept_removal_paths = kept_paths;
        self.as_mut().set_kept_removals(kept);
        self.set_removing_removals(removing);
    }

    /// Gives up the removal at `row` of `keptRemovals` that waits for unfinished writes: the Discard button, after the window
    /// asks. The removal is found by the exact path the row was read with, in the catalog's pending removals: the list's
    /// text is lossy, and a list read before a change is never acted on by its position alone.
    pub(crate) fn discard_removal(mut self: Pin<&mut Self>, row: i32) {
        let Some(shown) =
            usize::try_from(row).ok().and_then(|index| self.rust().kept_removal_paths.get(index)).cloned()
        else {
            return;
        };
        let library = app::get().library();
        let pending = match library.pending_removals() {
            Ok(pending) => pending,
            Err(e) => {
                app::post(Notice::DiscardFailed { error: app::error_text(&e) });
                return;
            }
        };
        if !pending.iter().any(|(path, _)| *path == shown) {
            // It is gone since the list was read (the job finished it, or it was discarded): nothing is discarded.
            self.as_mut().load_removals();
            return;
        }
        // The list is read again either way: a discard can fail after the catalog has dropped the removal.
        let result = library.discard_removal(&shown);
        self.as_mut().load_roots();
        if let Err(e) = result {
            app::post(Notice::DiscardFailed { error: app::error_text(&e) });
        }
    }

    /// Starts the library for this window's object: the library itself is started once per process, so a window made
    /// again (live reload) finds it running, and still lists its folders and takes its changes (`app::Library::start`
    /// returns the one running).
    pub(crate) fn start(mut self: Pin<&mut Self>) {
        if self.started {
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
        self.as_mut().rust_mut().started = true;
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
            pg_log::warn("library", &format!("the change pump didn't start: {e:#}"));
            // The window would show nothing of the background's changes: the inbox says so, not only the log.
            app::post(Notice::ChangesUnseen { error: app::error_text(&e) });
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
            // The scan asked for has ended: the window need not wait for it any more.
            self.as_mut().set_scan_pending(false);
            // A folder can have left the library since the list was read (a removal finished by the job, or from a command).
            self.as_mut().load_roots();
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
        // The job has shown itself: a scan asked for is no longer waited for on its account (`scanPending`).
        if p.phase != Phase::Idle {
            self.as_mut().set_scan_pending(false);
        }
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
        let writes = pg_wording::write_status(s.queued, error.as_deref());
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
                // A folder already in the library is rescanned (the app scans it either way): a scan is asked for while the
                // library runs.
                if running().is_some() {
                    self.as_mut().set_scan_pending(true);
                }
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

    /// The folder at `row` of `roots`, as the list shows it (`""` for none): the removal's dialog names it.
    pub(crate) fn folder_at(&self, row: i32) -> QString {
        usize::try_from(row)
            .ok()
            .and_then(|index| self.root_paths.get(index))
            .map_or_else(|| QString::from(""), |path| QString::from(&pg_platform::native::PATH_RULES.display(path)))
    }

    pub(crate) fn remove_folder(mut self: Pin<&mut Self>, row: i32) {
        let Some(index) = usize::try_from(row).ok() else { return };
        let (Some(id), Some(path)) =
            (self.rust().root_ids.get(index).copied(), self.rust().root_paths.get(index).cloned())
        else {
            return;
        };
        match app::get().library().remove_folder(id, &path) {
            // Gone from the list at once. A started removal's files stay listed until the job hides them: the job's
            // `Change::Scanned` (published after `hide_pages`, coordinator.rs) is what reads the grid and the tree again
            // (`scanned`, which QML answers with `reloadAll`), so the grid is not read here. A pending removal keeps its
            // previous refresh.
            Ok(Begun::Started { .. }) => self.as_mut().load_roots(),
            Ok(Begun::Pending(_)) => {
                self.as_mut().load_roots();
                self.scanned();
            }
            // The list changed since it was shown: nothing was removed, and the list is read again.
            Ok(Begun::Changed | Begun::NotFound) => {
                self.as_mut().load_roots();
                app::post(Notice::FolderNotRemoved { error: pg_wording::render(&Message::FolderListChanged) });
            }
            // A failure can come after the removal was begun (the catalog's marker is set): the list is read again.
            Err(e) => {
                self.as_mut().load_roots();
                app::post(Notice::FolderNotRemoved { error: app::error_text(&e) });
            }
        }
    }

    pub(crate) fn rescan(mut self: Pin<&mut Self>) {
        match app::get().library().rescan() {
            // The job has been asked to scan: the window waits for it from here (`scanPending`).
            Ok(()) => self.as_mut().set_scan_pending(true),
            Err(e) => app::post(Notice::RescanFailed { error: app::error_text(&e) }),
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
