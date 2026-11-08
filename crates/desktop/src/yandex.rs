//! Reverse image search on Yandex, from the viewer: the Qt glue over `pg_app::yandex`. A thread subscribes to the search's
//! changes and queues a `poll()` onto the Qt thread for each one, as the library's change pump does; QML doesn't poll.
//! The results are the model's rows, as the grid's are (`GalleryModel`): the other sizes first (`otherCount` of them),
//! then the similar images (`similarCount`). A row's `picked` role follows the picks to save. Each result's frame (its
//! `frameX`, `frameY`, `frameWidth` and `frameHeight` roles) places it in the panel's justified layout: the panel sets
//! `layoutWidth`, and the gallery's `rowHeight` and `gap`; each group's block is `otherHeight` or `similarHeight` high.
//! The preview shows one result (`previewItem`): its URL, size and pick are properties, set whenever the item or its
//! pick changes. The words the panel shows are formatted here (`pg_client::text` and the QML's own), from the typed
//! outcomes the app returns.

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
        /// The width the results are laid out in: the panel sets it to its list's width, and they are laid out again.
        #[qproperty(f64, layout_width, cxx_name = "layoutWidth", READ, WRITE = set_layout_width, NOTIFY, FINAL)]
        /// The height a row of results is laid out at: the gallery's row height (the user's setting), so the two agree.
        #[qproperty(f64, row_height, cxx_name = "rowHeight", READ, WRITE = set_row_height, NOTIFY, FINAL)]
        /// The space between the results: the gallery's gap, so the two agree.
        #[qproperty(f64, gap, READ, WRITE = set_gap, NOTIFY, FINAL)]
        /// The height of each group's block of frames: the other sizes', and the similar images'.
        #[qproperty(f64, other_height, cxx_name = "otherHeight", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(f64, similar_height, cxx_name = "similarHeight", READ, WRITE, NOTIFY, FINAL)]
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

        fn set_layout_width(self: Pin<&mut YandexSearch>, value: f64);

        fn set_row_height(self: Pin<&mut YandexSearch>, value: f64);

        fn set_gap(self: Pin<&mut YandexSearch>, value: f64);

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

        /// Show `text` as the status line: the save's folder dialog failed (the picker's `failed`), and says why.
        #[qinvokable]
        #[cxx_name = "showFailure"]
        fn show_failure(self: Pin<&mut YandexSearch>, text: &QString);

        /// Before the save dialog opens: where it starts (the folder saved into last, while it exists, or else the photo's
        /// folder), for the folder picker (`FolderPicker`, purpose 2).
        #[qinvokable]
        #[cxx_name = "prepareSave"]
        fn prepare_save(self: &YandexSearch);

        /// Save the picked results into the folder the picker chose for them (`FolderPicker`, purpose 2).
        #[qinvokable]
        #[cxx_name = "downloadPicked"]
        fn download_picked(self: Pin<&mut YandexSearch>);
    }
}

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use pg_app::yandex::{Hit, Phase, SaveFailure, Saved, Search, SearchError, SimilarNote, Status, YandexError};
use pg_client::layout::{self, Frame};
use pg_client::picks::Picks;
use pg_client::text;
use pg_model::ids::FileId;

const ROLE_THUMB: i32 = 256;
const ROLE_IMAGE_WIDTH: i32 = 257;
const ROLE_IMAGE_HEIGHT: i32 = 258;
const ROLE_PICKED: i32 = 259;
const ROLE_FRAME_X: i32 = 260;
const ROLE_FRAME_Y: i32 = 261;
const ROLE_FRAME_WIDTH: i32 = 262;
const ROLE_FRAME_HEIGHT: i32 = 263;

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
    layout_width: f64,
    row_height: f64,
    gap: f64,
    other_height: f64,
    similar_height: f64,
    search: Search,
    /// Whether the change pump runs: it starts with the first search or save, which subscribes before it starts one.
    pump: bool,
    /// The other sizes, then the similar images, as the latest search found them.
    hits: Vec<Hit>,
    /// Each result's frame in the panel's layout (`relayout`), by its index in `hits`.
    frames: Vec<Frame>,
    picks: Picks,
}

impl YandexSearchRust {
    /// The results `first..first + count` that exist.
    fn range(&self, first: i32, count: i32) -> std::ops::Range<usize> {
        self.picks.range(usize::try_from(first).unwrap_or(0), usize::try_from(count).unwrap_or(0))
    }
}

/// What the user reads when a request fails: each of the client's errors has its own words; the values they carry (the
/// transport's error, what an answer lacked) are the lower layers' text.
fn explain(e: &YandexError) -> String {
    match e {
        YandexError::Captcha => text::render(&text::Message::YandexCaptcha),
        YandexError::Status(status) => text::yandex_http_status(*status),
        YandexError::Transport(error) => text::yandex_transport(error),
        YandexError::Unexpected(detail) => text::yandex_unexpected(detail),
        YandexError::Recorded(error) => text::yandex_recorded(error),
    }
}

fn phase_text(phase: Phase) -> String {
    match phase {
        Phase::Starting => text::render(&text::Message::YandexStarting),
        Phase::Uploading => text::render(&text::Message::YandexUploading),
        Phase::OtherSizes => text::render(&text::Message::YandexOtherSizes),
        Phase::Similar => text::render(&text::Message::YandexSimilar),
        Phase::Saving { files } => text::yandex_saving(files),
        Phase::SavingFile { n, of } => text::yandex_saving_file(n, of),
    }
}

fn search_error_text(e: SearchError) -> String {
    match e {
        SearchError::NotStill => text::render(&text::Message::YandexNotStill),
        SearchError::Unreadable(e) => text::yandex_unreadable_photo(&e.to_string()),
        SearchError::Undecodable(e) => text::yandex_unreadable_photo(&e),
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
        SaveFailure::Download { url, error } => text::yandex_download_failed(&url, &explain(&error)),
        SaveFailure::Write { url, path, error } => {
            text::yandex_write_failed(&url, &path.display().to_string(), &error.to_string())
        }
    }
}

fn saved_text(saved: Saved) -> String {
    let Saved { folder, written, failed, first_error } = saved;
    let first_error = first_error.map(failure_text);
    text::yandex_saved(written, &folder.display().to_string(), failed, first_error.as_deref())
}

fn status_text(status: Status) -> String {
    match status {
        Status::Working(phase) => phase_text(phase),
        Status::Searched(note) => note.map_or_else(String::new, similar_note_text),
        Status::SearchFailed(e) => search_error_text(e),
        Status::Saved(saved) => saved_text(saved),
        // The panic that stopped it is in gallery.log (a thread the system refused is `not_started`'s line instead).
        Status::Stopped => text::render(&text::Message::YandexStopped),
    }
}

/// A count or a row as Qt's int, saturating: a Qt model holds no more rows than an int counts.
fn qt_int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// A result's aspect ratio, for its frame: its size as Yandex gives it, or square when it gives none.
fn hit_aspect(hit: &Hit) -> f64 {
    if hit.width > 0 && hit.height > 0 { f64::from(hit.width) / f64::from(hit.height) } else { 1.0 }
}

impl qobject::YandexSearch {
    fn search_file(mut self: Pin<&mut Self>, file_id: i64) {
        match crate::app::get().file_path(FileId::new(file_id)) {
            Ok(path) => self.search_photo(&path),
            Err(err) => {
                crate::panic_log::note(&format!("yandex: file {file_id}: {err:#}"));
                self.as_mut().set_status(QString::from(&text::yandex_unreadable_photo(&format!("{err:#}"))));
            }
        }
    }

    fn search_command_line(self: Pin<&mut Self>) {
        if let Some(path) = crate::command_line::path("find-similar") {
            self.search_photo(path);
        }
    }

    /// Start a search for the photo at `path`. The app refuses one already running; the status line keeps showing that
    /// one's progress.
    fn search_photo(mut self: Pin<&mut Self>, path: &Path) {
        self.as_mut().ensure_pump();
        match self.search.start(path, crate::command_line::yandex_source()) {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => return self.not_started("search", &e),
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
        self.as_mut().relayout();
        self.picks_changed();
    }

    /// The panel's width changed: the results are laid out in it again.
    fn set_layout_width(mut self: Pin<&mut Self>, value: f64) {
        if self.layout_width == value {
            return;
        }
        self.as_mut().rust_mut().layout_width = value;
        self.as_mut().layout_width_changed();
        self.relayout();
    }

    /// The gallery's row height changed: the results are laid out at it again.
    fn set_row_height(mut self: Pin<&mut Self>, value: f64) {
        if self.row_height == value {
            return;
        }
        self.as_mut().rust_mut().row_height = value;
        self.as_mut().row_height_changed();
        self.relayout();
    }

    /// The gallery's gap changed: the results are laid out again.
    fn set_gap(mut self: Pin<&mut Self>, value: f64) {
        if self.gap == value {
            return;
        }
        self.as_mut().rust_mut().gap = value;
        self.as_mut().gap_changed();
        self.relayout();
    }

    /// Lay the results out in the panel's width at the gallery's row height and gap, as the gallery lays its rows out:
    /// each group's rows are justified (`layout::frames`), from the top of the group's block, and the blocks' heights
    /// are set. Each result's frame is read again (its roles).
    fn relayout(mut self: Pin<&mut Self>) {
        let other = usize::try_from(self.other_count).unwrap_or(0).min(self.hits.len());
        let aspects: Vec<f64> = self.hits.iter().map(hit_aspect).collect();
        let (other_aspects, similar_aspects) = aspects.split_at(other);
        let (width, height, gap) = (self.layout_width, self.row_height, self.gap);
        let (mut frames, other_height) = layout::frames(other_aspects, width, height, gap);
        let (similar, similar_height) = layout::frames(similar_aspects, width, height, gap);
        frames.extend(similar);
        let rows = qt_int(frames.len());
        self.as_mut().rust_mut().frames = frames;
        self.as_mut().set_other_height(other_height);
        self.as_mut().set_similar_height(similar_height);
        if rows > 0 {
            let root = QModelIndex::default();
            let (top, bottom) = (self.index(0, 0, &root), self.index(rows - 1, 0, &root));
            let roles = [ROLE_FRAME_X, ROLE_FRAME_Y, ROLE_FRAME_WIDTH, ROLE_FRAME_HEIGHT];
            self.as_mut().data_changed(&top, &bottom, &QList::from(&roles[..]));
        }
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
        let frame = self.frames.get(i).copied().unwrap_or_default();
        match role {
            ROLE_THUMB => QVariant::from(&QString::from(&hit.thumb)),
            ROLE_IMAGE_WIDTH => QVariant::from(&hit.width),
            ROLE_IMAGE_HEIGHT => QVariant::from(&hit.height),
            ROLE_PICKED => QVariant::from(&self.picks.is_picked(i)),
            ROLE_FRAME_X => QVariant::from(&frame.x),
            ROLE_FRAME_Y => QVariant::from(&frame.y),
            ROLE_FRAME_WIDTH => QVariant::from(&frame.width),
            ROLE_FRAME_HEIGHT => QVariant::from(&frame.height),
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
            (ROLE_FRAME_X, "frameX"),
            (ROLE_FRAME_Y, "frameY"),
            (ROLE_FRAME_WIDTH, "frameWidth"),
            (ROLE_FRAME_HEIGHT, "frameHeight"),
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

    fn show_failure(mut self: Pin<&mut Self>, text: &QString) {
        let text = String::from(text);
        self.as_mut().set_status(QString::from(&text));
    }

    fn prepare_save(&self) {
        crate::picker::set_start(self.search.start_folder(crate::app::get()));
    }

    fn download_picked(mut self: Pin<&mut Self>) {
        let Some(folder) = crate::picker::take(crate::picker::Purpose::YandexSave) else {
            self.as_mut().set_status(QString::from(&text::yandex_no_folder()));
            return;
        };
        let hits: Vec<Hit> = self.picks.indices().filter_map(|i| self.hits.get(i).cloned()).collect();
        if hits.is_empty() {
            self.as_mut().set_status(QString::from(&text::yandex_nothing_picked()));
            return;
        }
        let files = hits.len();
        self.as_mut().ensure_pump();
        // The app refuses a search or a save already running (the status line keeps showing its progress), and
        // remembers the folder when it saves.
        match self.search.save(crate::app::get(), hits, folder, crate::command_line::yandex_source()) {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => return self.not_started("save", &e),
        }
        self.as_mut().set_busy(true);
        self.as_mut().set_status(QString::from(&status_text(Status::Working(Phase::Saving { files }))));
    }

    /// A search or a save (`what`) whose thread the system refused: the log says why, and so does the status line.
    /// Nothing runs, so the panel isn't busy.
    fn not_started(mut self: Pin<&mut Self>, what: &str, e: &std::io::Error) {
        crate::panic_log::note(&format!("yandex: the {what} didn't start: {e}"));
        self.as_mut().set_busy(false);
        self.as_mut().set_status(QString::from(&text::yandex_not_started(&e.to_string())));
    }
}
