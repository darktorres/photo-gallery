//! The library (the folders the user added) and the background work that keeps the catalog and the files in step:
//! the job that saves cropped copies, scans, imports and makes thumbnails (`pg_library::runner`), and the writer
//! that puts ratings and turns into the files (`pg_library::writeback`). QML polls `poll()` for progress, and
//! reloads its views on `scanned` (the folder tree changed), `imported` (sizes and placeholders changed) and
//! `filesChanged` (files were turned).

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

        #[qinvokable]
        fn poll(self: Pin<&mut Library>);
    }
}

use core::pin::Pin;
use std::sync::OnceLock;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList, QUrl};
use pg_jobs::control::{Phase, Progress};
use pg_library::runner::{Generation, Notice, Runner};
use pg_library::scan::normalize_path;
use pg_library::writeback::Writer;
use pg_model::ids::FileId;

use crate::state;

static RUNNER: OnceLock<Runner> = OnceLock::new();
static WRITER: OnceLock<Writer> = OnceLock::new();

/// Something was queued for the files (a rating, a turn).
pub fn wake_writer() {
    if let Some(w) = WRITER.get() {
        w.wake();
    }
}

/// Save cropped copies of these files' borders into `folder`, first thing in the background job.
pub fn save_crops(ids: Vec<FileId>, folder: std::path::PathBuf) {
    if let Some(r) = RUNNER.get() {
        r.save_crops(ids, folder);
    }
}

/// After the event loop, before the thumbnail cache closes: let the file write in progress finish, with its
/// thumbnails retagged. Then the background job stops after the file (or batch) it has, and its thread ends, so its
/// connections close. What it stops in the middle is redone at the next start. The wait is bounded: a file that
/// takes longer to read than `JOB_STOP_WAIT` is left to end with the process, as before.
pub fn stop() {
    if let Some(w) = WRITER.get() {
        w.stop();
    }
    if let Some(r) = RUNNER.get()
        && !r.stop(JOB_STOP_WAIT)
    {
        eprintln!("background job still running at exit: its last changes are merged at the next open");
    }
}

const JOB_STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

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
    seen: Generation,
    seen_turned: u64,
    seen_rounds: u64,
}

fn library_roots() -> Vec<String> {
    state::with_catalog(|c| Ok(c.roots()?.into_iter().map(|r| r.path).collect())).unwrap_or_else(|e| {
        eprintln!("library: {e:#}");
        Vec::new()
    })
}

/// The status line's text for a pass's progress; empty when idle.
fn progress_status(p: &Progress) -> String {
    match &p.phase {
        Phase::Idle => String::new(),
        Phase::Scan { root } => format!("Scanning {root} · {} images found", group_digits(p.done)),
        Phase::Import => working("Importing", p),
        Phase::Borders => working("Finding borders", p),
        Phase::Crop => working("Saving cropped copies", p),
        Phase::Thumbs { height } => working(&format!("Thumbnails at {height} px"), p),
    }
}

/// A pass that counts its files: how far it is, its rate, and what is left.
fn working(what: &str, p: &Progress) -> String {
    let failed = if p.failed > 0 { format!(" · {} failed", group_digits(p.failed)) } else { String::new() };
    format!(
        "{what} {} of {} · {:.0}/s · {}{failed}",
        group_digits(p.done),
        group_digits(p.total),
        p.rate(),
        duration(p.eta())
    )
}

fn group_digits(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn duration(secs: f64) -> String {
    let s = secs.round() as u64;
    match s {
        0..60 => format!("{s} s left"),
        60..3600 => format!("{} min left", s.div_ceil(60)),
        _ => format!("{} h {} min left", s / 3600, s % 3600 / 60),
    }
}

/// The words for a notice from the job. The English is here, in the app, which is where the text is shown; it moves
/// to `pg-client` with the other strings (S-10.8).
fn notice_text(notice: &Notice) -> String {
    match notice {
        Notice::UnreadableFolders { root, count } => format!("{root}: {count} folders couldn't be read"),
        Notice::ScanFailed { root, error } => format!("{root}: {error}"),
        Notice::RoundFailed { error } | Notice::CropFailed { error } => error.clone(),
        Notice::RoundPanicked { message } => format!("internal error in the background job: {message}"),
        Notice::CacheFull { height, made } => {
            format!("The thumbnail cache is full: {made} thumbnails at {height} px weren't made")
        }
        Notice::Cropped { folder, saved, skipped, failed, first_error } => {
            let plural = |n: usize| if n == 1 { "" } else { "s" };
            let mut note =
                format!("Saved {saved} cropped cop{} in {}", if *saved == 1 { "y" } else { "ies" }, folder.display());
            if *skipped > 0 {
                note += &format!("; {skipped} file{} had nothing to crop any more", plural(*skipped));
            }
            if let Some(e) = first_error {
                note += &format!("; {failed} failed, the first: {e}");
            }
            note
        }
    }
}

impl qobject::Library {
    fn show(mut self: Pin<&mut Self>, text: String, warning: bool) {
        self.as_mut().set_message_warning(warning);
        self.set_message(QString::from(&text));
    }

    fn load_roots(self: Pin<&mut Self>) {
        self.set_roots(library_roots().iter().map(QString::from).collect());
    }

    fn start(mut self: Pin<&mut Self>, thumb_height: i32) {
        self.as_mut().load_roots();
        let height = thumb_height.max(1) as u32;
        RUNNER.get_or_init(|| Runner::start(state::get().catalog_path.clone(), state::thumbs(), height));
        WRITER.get_or_init(|| Writer::start(state::get().catalog_path.clone(), state::thumbs()));
    }

    fn add_folder(mut self: Pin<&mut Self>, url: &QUrl) {
        let path = url.to_local_file().map_or_else(|| url.to_qstring().to_string(), |p| p.to_string());
        let root = match normalize_path(&path) {
            Ok(r) => r,
            Err(e) => return self.show(format!("{e:#}"), true),
        };
        match state::with_catalog(|c| c.add_root(&root)) {
            // A folder inside the library is just rescanned.
            Ok(added) => {
                if !added {
                    self.as_mut().show(format!("{root} is already in the library: rescanning it"), false);
                }
            }
            Err(e) => return self.show(format!("{e:#}"), true),
        }
        self.as_mut().load_roots();
        if let Some(r) = RUNNER.get() {
            r.scan(vec![root]);
        }
    }

    fn remove_folder(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        let removed = state::with_catalog(|c| match c.root_id(&path)? {
            Some(id) => c.remove_root(id),
            None => Ok(None),
        });
        if let Err(e) = removed {
            return self.show(format!("{e:#}"), true);
        }
        // A scan of it in progress stops; the job skips folders no longer in the library.
        if let Some(r) = RUNNER.get() {
            r.scan(Vec::new());
        }
        self.as_mut().load_roots();
        self.scanned();
    }

    fn rescan(self: Pin<&mut Self>) {
        if let Some(r) = RUNNER.get() {
            r.scan(library_roots());
        }
    }

    fn set_thumb_height(&self, height: i32) {
        if let Some(r) = RUNNER.get() {
            r.set_thumb_height(height.max(1) as u32);
        }
    }

    fn toggle_pause(mut self: Pin<&mut Self>) {
        let Some(r) = RUNNER.get() else { return };
        r.set_paused(!r.is_paused());
        self.as_mut().poll();
    }

    fn poll(mut self: Pin<&mut Self>) {
        let Some(r) = RUNNER.get() else { return };
        let p = r.progress();
        let paused = r.is_paused();
        let status = match (paused, progress_status(&p)) {
            (true, s) if s.is_empty() => "Background work paused".to_string(),
            (true, s) => format!("Paused · {s}"),
            (false, s) => s,
        };
        self.as_mut().set_busy(p.phase != Phase::Idle);
        self.as_mut().set_paused(paused);
        self.as_mut().set_status(QString::from(&status));
        self.as_mut().set_progress(if p.total > 0 { p.fraction() } else { -1.0 });
        // The latest message, with how many others came in since the last poll: the runner keeps the newest and
        // counts what it dropped, so none goes unannounced.
        let taken = r.take_messages();
        if let Some(last) = taken.kept.back() {
            let more = taken.kept.len() - 1 + taken.not_kept;
            let text = notice_text(last);
            let text = if more == 0 { text } else { format!("{text} (and {more} more)") };
            self.as_mut().show(text, last.is_warning());
        }
        if let Some(w) = WRITER.get() {
            let s = w.status();
            let failed = s.error.is_some();
            let writes = match (s.queued, s.error) {
                (0, None) => String::new(),
                (n, None) => format!("Writing {n} change{} into files", if n == 1 { "" } else { "s" }),
                (n, Some(e)) => format!("{n} change{} not yet in the files: {e}", if n == 1 { "" } else { "s" }),
            };
            if self.writes.to_string() != writes {
                self.as_mut().set_writes(QString::from(&writes));
            }
            self.as_mut().set_write_failed(failed);
            // A turned file's analysis is stale: the job re-analyzes it, and the views reload it.
            let turned = w.turned();
            if std::mem::replace(&mut self.as_mut().rust_mut().seen_turned, turned) != turned {
                r.wake();
                self.as_mut().files_changed();
            }
        }
        let now = r.generation();
        let seen = std::mem::replace(&mut self.as_mut().rust_mut().seen, now);
        if now.scans != seen.scans {
            self.as_mut().scanned();
        } else if now.imports != seen.imports {
            self.as_mut().imported();
        }
        // The job's rounds, counted as stages for measuring (docs/PROFILING.md); a round ends on its own thread, so
        // it is counted here, on the poll that sees it.
        let rounds = r.rounds();
        let seen_rounds = std::mem::replace(&mut self.as_mut().rust_mut().seen_rounds, rounds);
        for _ in seen_rounds..rounds {
            pg_trace::pg_stage_round_done();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1234567), "1,234,567");
        assert_eq!(duration(5.2), "5 s left");
        assert_eq!(duration(61.0), "2 min left");
        assert_eq!(duration(7300.0), "2 h 1 min left");
    }
}
