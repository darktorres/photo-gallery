//! Reverse image search on Yandex, from the viewer: the Qt glue over `pg_app::yandex`. A thread subscribes to the search's
//! changes and queues a `poll()` onto the Qt thread for each one, as the library's change pump does; QML doesn't poll.
//! The results are the model's rows, as the grid's are (`GalleryModel`): the other sizes first (`otherCount` of them),
//! then the similar images (`similarCount`). A row's `picked` role follows the picks to save. The preview shows one
//! result (`previewItem`): its URL, size and pick are properties, set whenever the item or its pick changes. The words
//! the panel shows are formatted here (`pg_client::text` and the QML's own), from the typed outcomes the app returns.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtCore/QAbstractListModel>);
        type QAbstractListModel;

        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qproperty(bool, busy, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(QString, status, READ, WRITE, NOTIFY, FINAL)]
        /// The results are the model's rows: the other sizes first, then the similar images.
        #[qproperty(i32, other_count, cxx_name = "otherCount", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, similar_count, cxx_name = "similarCount", READ, WRITE, NOTIFY, FINAL)]
        /// Results picked to be saved, and how many of each group are (its header's count).
        #[qproperty(i32, picked_count, cxx_name = "pickedCount", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, other_picked, cxx_name = "otherPicked", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, similar_picked, cxx_name = "similarPicked", READ, WRITE, NOTIFY, FINAL)]
        /// The result the preview shows (-1: none). Setting it refreshes the four properties below.
        #[qproperty(i32, preview_item, cxx_name = "previewItem", READ, WRITE = set_preview_item, NOTIFY, FINAL)]
        /// What the preview shows of its item: its picture's URL (`""` for none), its size in pixels as Yandex gives
        /// it (0 when it doesn't), and whether it is picked.
        #[qproperty(QString, preview_url, cxx_name = "previewUrl", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, preview_width, cxx_name = "previewWidth", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, preview_height, cxx_name = "previewHeight", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(bool, preview_picked, cxx_name = "previewPicked", READ, WRITE, NOTIFY, FINAL)]
        type YandexSearch = super::YandexSearchRust;
    }

    impl cxx_qt::Threading for YandexSearch {}

    unsafe extern "RustQt" {
        fn set_preview_item(self: Pin<&mut YandexSearch>, value: i32);

        /// Upload the catalog's file `file_id` (the photo) and read its other sizes and similar images.
        #[qinvokable]
        #[cxx_name = "searchFile"]
        fn search_file(self: Pin<&mut YandexSearch>, file_id: i64);

        /// Upload the photo the command line names (`--find-similar=<path>`), as `searchFile` does for a catalog file.
        #[qinvokable]
        #[cxx_name = "searchCommandLine"]
        fn search_command_line(self: Pin<&mut YandexSearch>);

        #[qinvokable]
        #[cxx_override]
        fn data(self: &YandexSearch, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &YandexSearch) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &YandexSearch, parent: &QModelIndex) -> i32;

        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut YandexSearch>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut YandexSearch>);

        #[inherit]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut YandexSearch>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );

        #[inherit]
        fn index(self: &YandexSearch, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QUrl, QVariant};
use pg_app::yandex::{Hit, Phase, SaveFailure, Saved, Search, SearchError, SimilarNote, Status, YandexError};
use pg_client::picks::Picks;
use pg_client::text;
use pg_model::ids::FileId;

/// The status line when a search or a save stops at an internal error (the panic is in the log).
const INTERNAL_STOP: &str = "Stopped by an internal error; the details are in gallery.log.";

const ROLE_THUMB: i32 = 256;
const ROLE_IMAGE_WIDTH: i32 = 257;
const ROLE_IMAGE_HEIGHT: i32 = 258;
const ROLE_PICKED: i32 = 259;

#[derive(Default)]
pub struct YandexSearchRust {
    busy: bool,
    status: QString,
    other_count: i32,
    similar_count: i32,
    picked_count: i32,
    other_picked: i32,
    similar_picked: i32,
    preview_item: i32,
    preview_url: QString,
    preview_width: i32,
    preview_height: i32,
    preview_picked: bool,
    search: Search,
    /// Whether the change pump runs: it starts with the first search or save, which subscribes before it starts one.
    pump: bool,
    /// The other sizes, then the similar images, as the latest search found them.
    hits: Vec<Hit>,
    picks: Picks,
}

impl YandexSearchRust {
    /// The results `first..first + count` that exist.
    fn range(&self, first: i32, count: i32) -> std::ops::Range<usize> {
        self.picks.range(usize::try_from(first).unwrap_or(0), usize::try_from(count).unwrap_or(0))
    }
}

/// What the user reads when a request fails.
fn explain(e: &YandexError) -> String {
    match e {
        YandexError::Captcha => text::render(&text::Message::YandexCaptcha),
        other => other.to_string(),
    }
}

fn phase_text(phase: Phase) -> String {
    match phase {
        Phase::Starting => "Starting…".to_string(),
        Phase::Uploading => text::render(&text::Message::YandexUploading),
        Phase::OtherSizes => text::render(&text::Message::YandexOtherSizes),
        Phase::Similar => text::render(&text::Message::YandexSimilar),
        Phase::Saving { files } => format!("Saving {files} files…"),
        Phase::SavingFile { n, of } => format!("Saving {n} of {of}…"),
    }
}

fn search_error_text(e: SearchError) -> String {
    match e {
        SearchError::NotStill => text::render(&text::Message::YandexNotStill),
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

/// A count or a row as Qt's int, saturating: a Qt model holds no more rows than an int counts.
fn qt_int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

impl qobject::YandexSearch {
    fn search_file(self: Pin<&mut Self>, file_id: i64) {
        match crate::app::get().file_path(FileId::new(file_id)) {
            Ok(path) => self.search_photo(&path),
            Err(err) => crate::panic_log::note(&format!("yandex: file {file_id}: {err:#}")),
        }
    }

    fn search_command_line(self: Pin<&mut Self>) {
        if let Some(path) = crate::command_line::path("find-similar") {
            self.search_photo(path);
        }
    }

    /// Start a search for the photo at `path`. The app refuses one already running, and nothing shows then.
    fn search_photo(mut self: Pin<&mut Self>, path: &Path) {
        self.as_mut().ensure_pump();
        if !self.search.start(path, crate::command_line::yandex_source()) {
            return;
        }
        // The pump's poll reads `busy` with the rest of the state; set it here too, so the panel doesn't wait for it.
        self.as_mut().set_busy(true);
        self.as_mut().set_status(QString::from(&phase_text(Phase::Starting)));
        self.as_mut().show(Vec::new(), Vec::new());
    }

    /// Start the thread that polls on each change of the search (once, unless it couldn't start). It subscribes before
    /// any search or save starts, so none of their changes is missed.
    fn ensure_pump(mut self: Pin<&mut Self>) {
        if self.pump {
            return;
        }
        self.as_mut().rust_mut().pump = true;
        let changes = self.search.subscribe();
        let thread = self.qt_thread();
        // Set while a poll is queued. A poll reads the whole state, so a change that comes in while one is queued is
        // read by it: the pump queues no more until it has run (at most one per event-loop turn).
        let queued = Arc::new(AtomicBool::new(false));
        let pump = std::thread::Builder::new().name("yandex changes".into()).spawn(move || {
            while changes.recv().is_some() {
                changes.take();
                if queued.swap(true, Ordering::AcqRel) {
                    continue;
                }
                let flag = Arc::clone(&queued);
                // Fails only when the window is gone: the pump ends with it.
                if thread
                    .queue(move |search| {
                        flag.store(false, Ordering::Release);
                        search.poll()
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        // Without its thread the panel isn't told the search's changes: the log says why, and the next search tries
        // again.
        if let Err(e) = pump {
            crate::panic_log::note(&format!("yandex: the change pump didn't start: {e}"));
            self.as_mut().rust_mut().pump = false;
        }
    }

    /// Show these results, none picked.
    fn show(mut self: Pin<&mut Self>, other: Vec<Hit>, similar: Vec<Hit>) {
        let (other_count, similar_count) = (qt_int(other.len()), qt_int(similar.len()));
        self.as_mut().begin_reset_model();
        {
            let mut this = self.as_mut().rust_mut();
            this.hits = other.into_iter().chain(similar).collect();
            let count = this.hits.len();
            this.picks.reset(count);
        }
        self.as_mut().end_reset_model();
        self.as_mut().set_other_count(other_count);
        self.as_mut().set_similar_count(similar_count);
        self.picks_changed();
    }

    /// After the picks change: each row's `picked` role, the counts, and the preview's pick.
    fn picks_changed(mut self: Pin<&mut Self>) {
        let rows = qt_int(self.hits.len());
        if rows > 0 {
            let root = QModelIndex::default();
            let (top, bottom) = (self.index(0, 0, &root), self.index(rows - 1, 0, &root));
            self.as_mut().data_changed(&top, &bottom, &QList::from(&[ROLE_PICKED][..]));
        }
        let other_picked = qt_int(self.picks.picked_in(self.range(0, self.other_count)));
        let similar_picked = qt_int(self.picks.picked_in(self.range(self.other_count, self.similar_count)));
        let picked_count = qt_int(self.picks.count());
        self.as_mut().set_picked_count(picked_count);
        self.as_mut().set_other_picked(other_picked);
        self.as_mut().set_similar_picked(similar_picked);
        self.refresh_preview();
    }

    fn set_preview_item(mut self: Pin<&mut Self>, value: i32) {
        if self.preview_item == value {
            return;
        }
        self.as_mut().rust_mut().preview_item = value;
        self.as_mut().preview_item_changed();
        self.refresh_preview();
    }

    /// The preview's values for its item: the picture's URL, its size and its pick (empty for no item).
    fn refresh_preview(mut self: Pin<&mut Self>) {
        let shown = usize::try_from(self.preview_item).ok().and_then(|i| self.hits.get(i).map(|hit| (i, hit)));
        let (url, width, height, picked) = match shown {
            Some((i, hit)) => (QString::from(&hit.url), hit.width, hit.height, self.picks.is_picked(i)),
            None => (QString::default(), 0, 0, false),
        };
        self.as_mut().set_preview_url(url);
        self.as_mut().set_preview_width(width);
        self.as_mut().set_preview_height(height);
        self.as_mut().set_preview_picked(picked);
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

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Ok(i) = usize::try_from(index.row()) else {
            return QVariant::default();
        };
        let Some(hit) = self.hits.get(i) else {
            return QVariant::default();
        };
        match role {
            ROLE_THUMB => QVariant::from(&QString::from(&hit.thumb)),
            ROLE_IMAGE_WIDTH => QVariant::from(&hit.width),
            ROLE_IMAGE_HEIGHT => QVariant::from(&hit.height),
            ROLE_PICKED => QVariant::from(&self.picks.is_picked(i)),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [
            (ROLE_THUMB, "thumb"),
            (ROLE_IMAGE_WIDTH, "imageWidth"),
            (ROLE_IMAGE_HEIGHT, "imageHeight"),
            (ROLE_PICKED, "picked"),
        ] {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    fn row_count(&self, _parent: &QModelIndex) -> i32 {
        qt_int(self.hits.len())
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

    fn start_folder(&self) -> QUrl {
        self.search.start_folder(crate::app::get()).map_or_else(QUrl::default, |f| QUrl::from(&file_url(&f)))
    }

    fn download(mut self: Pin<&mut Self>, folder: &QUrl) {
        let folder = folder.to_local_file().map_or_else(|| folder.to_qstring().to_string(), |p| p.to_string());
        let hits: Vec<Hit> = self.picks.indices().filter_map(|i| self.hits.get(i).cloned()).collect();
        let files = hits.len();
        self.as_mut().ensure_pump();
        // The app refuses an empty pick, a search or a save already running, and remembers the folder when it saves.
        if !self.search.save(crate::app::get(), hits, PathBuf::from(folder), crate::command_line::yandex_source()) {
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
