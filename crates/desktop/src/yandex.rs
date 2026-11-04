//! Reverse image search on Yandex, from the viewer: the Qt glue over `pg_app::yandex`. A thread subscribes to the search's
//! changes and queues a `poll()` onto the Qt thread for each one, as the library's change pump does; QML doesn't poll.
//! The results are items by index, as the grid's are (`GalleryModel`): the other sizes first (`otherCount` of them),
//! then the similar images (`similarCount`). The picks to save are kept here too. The words the panel shows are
//! formatted here (`pg_client::text` and the QML's own), from the typed outcomes the app returns.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, busy, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(QString, status, READ, WRITE, NOTIFY, FINAL)]
        /// Results of the latest search: the other sizes, then the similar images.
        #[qproperty(i32, other_count, cxx_name = "otherCount", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, similar_count, cxx_name = "similarCount", READ, WRITE, NOTIFY, FINAL)]
        /// Results picked to be saved.
        #[qproperty(i32, picked_count, cxx_name = "pickedCount", READ, WRITE, NOTIFY, FINAL)]
        /// Changes whenever the results or the picks do: for bindings that call the item functions.
        #[qproperty(i32, revision, READ, WRITE, NOTIFY, FINAL)]
        type YandexSearch = super::YandexSearchRust;
    }

    impl cxx_qt::Threading for YandexSearch {}

    unsafe extern "RustQt" {
        /// Upload the photo at the local file `path` and read its other sizes and similar images.
        #[qinvokable]
        fn search(self: Pin<&mut YandexSearch>, path: &QString);

        /// A result's full-size file.
        #[qinvokable]
        fn url(self: &YandexSearch, item: i32) -> QString;

        /// A picture of the result to show: Yandex's preview for a similar image, the file itself for another size.
        #[qinvokable]
        fn thumb(self: &YandexSearch, item: i32) -> QString;

        /// Its size in pixels, as Yandex gives it; 0 when it doesn't.
        #[qinvokable]
        fn width(self: &YandexSearch, item: i32) -> i32;

        #[qinvokable]
        fn height(self: &YandexSearch, item: i32) -> i32;

        #[qinvokable]
        #[cxx_name = "isPicked"]
        fn is_picked(self: &YandexSearch, item: i32) -> bool;

        /// Pick a result to be saved, or unpick it. It becomes the anchor of the next `pickTo`.
        #[qinvokable]
        #[cxx_name = "togglePick"]
        fn toggle_pick(self: Pin<&mut YandexSearch>, item: i32);

        /// Shift-click: every result from the anchor (the last one toggled) to `item` takes the anchor's state, picked
        /// or not. Without an anchor, toggles `item`.
        #[qinvokable]
        #[cxx_name = "pickTo"]
        fn pick_to(self: Pin<&mut YandexSearch>, item: i32);

        /// Pick (or unpick) the `count` results from `first`: a group's Select all and Select none.
        #[qinvokable]
        #[cxx_name = "setPicked"]
        fn set_picked(self: Pin<&mut YandexSearch>, first: i32, count: i32, picked: bool);

        /// How many of the `count` results from `first` are picked.
        #[qinvokable]
        #[cxx_name = "pickedIn"]
        fn picked_in(self: &YandexSearch, first: i32, count: i32) -> i32;

        /// Where the save dialog opens: the folder saved into last, while it exists, or else the photo's folder.
        #[qinvokable]
        #[cxx_name = "startFolder"]
        fn start_folder(self: &YandexSearch) -> QUrl;

        /// Save the picked results into `folder`, a file URL.
        #[qinvokable]
        fn download(self: Pin<&mut YandexSearch>, folder: &QUrl);
    }
}

use std::path::{Path, PathBuf};
use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QUrl};
use pg_app::yandex::{Hit, Phase, SaveFailure, Saved, Search, SearchError, SimilarNote, Status};
use pg_client::picks::Picks;
use pg_client::text;
use pg_yandex::YandexError;

/// The status line when a search or a save stops at an internal error (the panic is in the log).
const INTERNAL_STOP: &str = "Stopped by an internal error; the details are in gallery.log.";

#[derive(Default)]
pub struct YandexSearchRust {
    busy: bool,
    status: QString,
    other_count: i32,
    similar_count: i32,
    picked_count: i32,
    revision: i32,
    search: Search,
    /// Whether the change pump runs: it starts with the first search or save, which subscribes before it starts one.
    pump: bool,
    /// The other sizes, then the similar images, as the latest search found them.
    hits: Vec<Hit>,
    picks: Picks,
}

impl YandexSearchRust {
    fn hit(&self, item: i32) -> Option<&Hit> {
        usize::try_from(item).ok().and_then(|i| self.hits.get(i))
    }

    /// The results `first..first + count` that exist.
    fn range(&self, first: i32, count: i32) -> std::ops::Range<usize> {
        self.picks.range(usize::try_from(first).unwrap_or(0), usize::try_from(count).unwrap_or(0))
    }
}

/// What the user reads when a request fails.
fn explain(e: &YandexError) -> String {
    match e {
        YandexError::Captcha => text::YANDEX_CAPTCHA.to_string(),
        other => other.to_string(),
    }
}

fn phase_text(phase: Phase) -> String {
    match phase {
        Phase::Starting => "Starting…".to_string(),
        Phase::Uploading => text::YANDEX_UPLOADING.to_string(),
        Phase::OtherSizes => text::YANDEX_OTHER_SIZES.to_string(),
        Phase::Similar => text::YANDEX_SIMILAR.to_string(),
        Phase::Saving { files } => format!("Saving {files} files…"),
        Phase::SavingFile { n, of } => format!("Saving {n} of {of}…"),
    }
}

fn search_error_text(e: SearchError) -> String {
    match e {
        SearchError::NotStill => text::YANDEX_NOT_STILL.to_string(),
        SearchError::Unreadable(e) => text::yandex_unreadable_photo(&e.to_string()),
        SearchError::Request(e) => explain(&e),
    }
}

fn similar_note_text(note: SimilarNote) -> String {
    match note {
        SimilarNote::Stopped { shown, error } => text::yandex_similar_stopped(shown, &explain(&error)),
        SimilarNote::Failed(error) => text::yandex_similar_failed(&explain(&error)),
    }
}

fn failure_text(failure: SaveFailure) -> String {
    match failure {
        SaveFailure::Client(e) => explain(&e),
        SaveFailure::Download { url, error } => format!("{url}: {error}"),
        SaveFailure::Write { url, path, error } => format!("{url}: {}: {error}", path.display()),
    }
}

fn saved_text(saved: Saved) -> String {
    let Saved { folder, written, failed, first_error } = saved;
    match first_error {
        None => format!("Saved {written} file{} to {}", if written == 1 { "" } else { "s" }, folder.display()),
        Some(first) => format!("Saved {written}, {failed} failed. First failure: {}", failure_text(first)),
    }
}

fn status_text(status: Status) -> String {
    match status {
        Status::Working(phase) => phase_text(phase),
        Status::Searched(note) => note.map_or_else(String::new, similar_note_text),
        Status::SearchFailed(e) => search_error_text(e),
        Status::Saved(saved) => saved_text(saved),
        Status::Stopped => INTERNAL_STOP.to_string(),
    }
}

/// `file:///C:/a/b`: a local path as a URL, for the folder dialog. `%`, `#` and `?` are escaped, or the URL would read
/// them as an escape, a fragment and a query.
fn file_url(path: &Path) -> String {
    let mut url = "file:///".to_string();
    for c in path.to_string_lossy().chars() {
        match c {
            '\\' => url.push('/'),
            '%' => url.push_str("%25"),
            '#' => url.push_str("%23"),
            '?' => url.push_str("%3F"),
            c => url.push(c),
        }
    }
    url
}

impl qobject::YandexSearch {
    fn search(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        self.as_mut().ensure_pump();
        if !self.search.start(Path::new(&path)) {
            return;
        }
        // The pump's poll reads `busy` with the rest of the state; set it here too, so the panel doesn't wait for it.
        self.as_mut().set_busy(true);
        self.as_mut().set_status(QString::from(&phase_text(Phase::Starting)));
        self.as_mut().show(Vec::new(), Vec::new());
    }

    /// Start the thread that polls on each change of the search (once). It subscribes before any search or save starts,
    /// so none of their changes is missed.
    fn ensure_pump(mut self: Pin<&mut Self>) {
        if self.pump {
            return;
        }
        self.as_mut().rust_mut().pump = true;
        let changes = self.search.subscribe();
        let thread = self.qt_thread();
        std::thread::Builder::new()
            .name("yandex changes".into())
            .spawn(move || {
                while changes.recv().is_some() {
                    // Each poll reads the whole state, so what came in while this one was queued is read with it.
                    changes.take();
                    // Fails only when the window is gone: the pump ends with it.
                    if thread.queue(|search| search.poll()).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn the yandex change pump");
    }

    /// Show these results, none picked.
    fn show(mut self: Pin<&mut Self>, other: Vec<Hit>, similar: Vec<Hit>) {
        let (other_count, similar_count) = (other.len() as i32, similar.len() as i32);
        {
            let mut this = self.as_mut().rust_mut();
            this.hits = other.into_iter().chain(similar).collect();
            let count = this.hits.len();
            this.picks.reset(count);
        }
        self.as_mut().set_other_count(other_count);
        self.as_mut().set_similar_count(similar_count);
        self.picks_changed();
    }

    /// After the picks change: their count, and a new revision for the bindings.
    fn picks_changed(mut self: Pin<&mut Self>) {
        let picked_count = self.picks.count() as i32;
        self.as_mut().set_picked_count(picked_count);
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn poll(mut self: Pin<&mut Self>) {
        let poll = self.search.poll();
        // The results first: whatever watches `busy` going off finds them in place.
        if let Some(found) = poll.found {
            self.as_mut().show(found.other, found.similar);
        }
        if let Some(status) = poll.status {
            self.as_mut().set_status(QString::from(&status_text(status)));
        }
        self.set_busy(poll.busy);
    }

    fn url(&self, item: i32) -> QString {
        self.hit(item).map_or_else(QString::default, |h| QString::from(&h.url))
    }

    fn thumb(&self, item: i32) -> QString {
        self.hit(item).map_or_else(QString::default, |h| QString::from(&h.thumb))
    }

    fn width(&self, item: i32) -> i32 {
        self.hit(item).map_or(0, |h| h.width)
    }

    fn height(&self, item: i32) -> i32 {
        self.hit(item).map_or(0, |h| h.height)
    }

    fn is_picked(&self, item: i32) -> bool {
        usize::try_from(item).is_ok_and(|i| self.picks.is_picked(i))
    }

    fn toggle_pick(mut self: Pin<&mut Self>, item: i32) {
        let Ok(i) = usize::try_from(item) else { return };
        self.as_mut().rust_mut().picks.toggle(i);
        self.picks_changed();
    }

    fn pick_to(mut self: Pin<&mut Self>, item: i32) {
        let Ok(i) = usize::try_from(item) else { return };
        self.as_mut().rust_mut().picks.pick_to(i);
        self.picks_changed();
    }

    fn set_picked(mut self: Pin<&mut Self>, first: i32, count: i32, picked: bool) {
        let range = self.range(first, count);
        self.as_mut().rust_mut().picks.set(range, picked);
        self.picks_changed();
    }

    fn picked_in(&self, first: i32, count: i32) -> i32 {
        self.picks.picked_in(self.range(first, count)) as i32
    }

    fn start_folder(&self) -> QUrl {
        self.search.start_folder(crate::app::get()).map_or_else(QUrl::default, |f| QUrl::from(&file_url(&f)))
    }

    fn download(mut self: Pin<&mut Self>, folder: &QUrl) {
        let folder = folder.to_local_file().map_or_else(|| folder.to_qstring().to_string(), |p| p.to_string());
        let hits: Vec<Hit> = self.picks.indices().filter_map(|i| self.hits.get(i).cloned()).collect();
        let files = hits.len();
        self.as_mut().ensure_pump();
        // The app refuses an empty pick, a search or a save already running, and remembers the folder when it saves.
        if !self.search.save(crate::app::get(), hits, PathBuf::from(folder)) {
            return;
        }
        self.as_mut().set_busy(true);
        self.as_mut().set_status(QString::from(&status_text(Status::Working(Phase::Saving { files }))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_url_escapes_what_a_url_would_misread() {
        assert_eq!(file_url(Path::new(r"C:\a b\c")), "file:///C:/a b/c");
        assert_eq!(file_url(Path::new(r"D:\100% #1?\x")), "file:///D:/100%25 %231%3F/x");
    }
}
