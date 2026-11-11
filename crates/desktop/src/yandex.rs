//! Reverse image search on Yandex, from the viewer: the Qt glue over `pg_app::yandex`. A thread subscribes to the search's
//! changes and queues a `poll()` onto the Qt thread for each one, as the library's change pump does; QML doesn't poll.
//! The results are the model's rows, as the grid's are (`GalleryModel`): the other sizes first (`otherCount` of them),
//! then the similar images (`similarCount`). A row's `picked` role follows the picks to save. Each result's frame (its
//! `frameX`, `frameY`, `frameWidth` and `frameHeight` roles) places it in the panel's justified layout: the panel sets
//! `layoutWidth` and the gallery's `gap`; each group's block is `otherHeight` or `similarHeight` high.
//! The preview shows one result (`previewItem`): its URL, size and pick are properties, set whenever the item or its
//! pick changes. The words the panel shows are `pg_client::text`'s (and the QML's own), from the outcomes the app
//! returns as data (`pg_model::yandex`).

pub(crate) mod state;
use state::YandexSearchRust;

use crate::yandex_bridge::qobject;

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use pg_app::yandex::Hit;
use pg_client::image::ImageRequest;
use pg_client::layout;
use pg_client::results::Results;
use pg_client::text;
use pg_model::ids::FileId;
use pg_model::yandex::{Phase, Status};

const ROLE_THUMB: i32 = 256;
const ROLE_IMAGE_WIDTH: i32 = 257;
const ROLE_IMAGE_HEIGHT: i32 = 258;
const ROLE_PICKED: i32 = 259;
const ROLE_FRAME_X: i32 = 260;
const ROLE_FRAME_Y: i32 = 261;
const ROLE_FRAME_WIDTH: i32 = 262;
const ROLE_FRAME_HEIGHT: i32 = 263;

/// The height a row of results is laid out at, in px: fixed, at the size the gallery's rows start at. The search makes
/// its grid pictures at this height.
const ROW_HEIGHT: u16 = 220;

/// The app's search, which the panel shows.
fn search() -> &'static pg_app::yandex::Yandex {
    crate::app::get().yandex()
}

impl YandexSearchRust {
    /// The results `first..first + count` that exist.
    fn range(&self, first: i32, count: i32) -> std::ops::Range<usize> {
        let (first, count) = (crate::rows::row_of(first).unwrap_or(0), crate::rows::row_of(count).unwrap_or(0));
        self.results.picks().range(first, count)
    }
}

/// The URL of a result's grid picture: the image provider serves it from the search's files.
fn picture_url(id: u64) -> String {
    ImageRequest::Yandex { id, original: false }.url()
}

/// The URL of a result's file as its host gave it, for the preview at full size.
fn original_url(id: u64) -> String {
    ImageRequest::Yandex { id, original: true }.url()
}

/// A count or a row as Qt's int, saturating: a Qt model holds no more rows than an int counts.
fn qt_int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// A side in pixels as Qt's int, saturating.
fn qt_side(n: u32) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// A result's aspect ratio, for its frame: its size as Yandex gives it, or square when it gives none.
fn hit_aspect(hit: &Hit) -> f64 {
    if hit.width > 0 && hit.height > 0 { f64::from(hit.width) / f64::from(hit.height) } else { 1.0 }
}

impl qobject::YandexSearch {
    pub(crate) fn search_file(mut self: Pin<&mut Self>, file_id: i64) {
        match crate::app::get().catalogs().file_path(FileId::new(file_id)) {
            Ok(path) => self.search_photo(&path),
            Err(err) => {
                pg_log::warn("yandex", &format!("file {file_id}: {err:#}"));
                self.as_mut().set_status(QString::from(&text::yandex_unreadable_photo(&format!("{err:#}"))));
            }
        }
    }

    pub(crate) fn search_command_line(self: Pin<&mut Self>) {
        if let Some(path) = crate::command_line::path("find-similar") {
            self.search_photo(path);
        }
    }

    /// Start a search for the photo at `path`. The app refuses one already running; the status line keeps showing that
    /// one's progress.
    fn search_photo(mut self: Pin<&mut Self>, path: &Path) {
        self.as_mut().ensure_pump();
        match search().start(path, usize::from(ROW_HEIGHT)) {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => return self.not_started("search", &e),
        }
        // The pump's poll reads `busy` with the rest of the state; set it here too, so the panel doesn't wait for it.
        self.as_mut().set_busy(true);
        self.as_mut().set_status(QString::from(&text::yandex_phase(Phase::Starting)));
        self.as_mut().show(Results::default());
    }

    /// Start the thread that polls on each change of the search (once, unless it couldn't start). It subscribes before
    /// any search or save starts, so none of their changes is missed.
    fn ensure_pump(mut self: Pin<&mut Self>) {
        if self.pump {
            return;
        }
        self.as_mut().rust_mut().pump = true;
        let changes = search().subscribe();
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
            pg_log::warn("yandex", &format!("the change pump didn't start: {e}"));
            self.as_mut().rust_mut().pump = false;
        }
    }

    /// Show these results, none picked.
    fn show(mut self: Pin<&mut Self>, results: Results<Hit>) {
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().results = results;
        self.as_mut().end_reset_model();
        self.as_mut().groups_changed();
    }

    /// After the results changed: the groups' counts, the layout and the picks.
    fn groups_changed(mut self: Pin<&mut Self>) {
        let (other, similar) = (qt_int(self.results.first().len()), qt_int(self.results.second().len()));
        self.as_mut().set_other_count(other);
        self.as_mut().set_similar_count(similar);
        self.as_mut().relayout();
        self.picks_changed();
    }

    /// The panel's width changed: the results are laid out in it again.
    pub(crate) fn set_layout_width(mut self: Pin<&mut Self>, value: f64) {
        if self.layout_width == value {
            return;
        }
        self.as_mut().rust_mut().layout_width = value;
        self.as_mut().layout_width_changed();
        self.relayout();
    }

    /// The gallery's gap changed: the results are laid out again.
    pub(crate) fn set_gap(mut self: Pin<&mut Self>, value: f64) {
        if self.gap == value {
            return;
        }
        self.as_mut().rust_mut().gap = value;
        self.as_mut().gap_changed();
        self.relayout();
    }

    /// Lay the results out in the panel's width, at `ROW_HEIGHT` and the gallery's gap, as the gallery lays its rows out:
    /// each group's rows are justified (`layout::frames`), from the top of the group's block, and the blocks' heights
    /// are set. Each result's frame is read again (its roles).
    /// The results whose files died are taken off the grid: the layout is rebuilt without them, and the picks of the
    /// others stay on their results.
    fn drop_dead(mut self: Pin<&mut Self>, dead: &[u64]) {
        // The model is reset around the change only when one goes.
        if !self.results.items().iter().any(|hit| dead.contains(&hit.id)) {
            return;
        }
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().results.drop_where(|hit| dead.contains(&hit.id));
        self.as_mut().end_reset_model();
        self.as_mut().groups_changed();
    }

    fn relayout(mut self: Pin<&mut Self>) {
        let other_aspects: Vec<f64> = self.results.first().iter().map(hit_aspect).collect();
        let similar_aspects: Vec<f64> = self.results.second().iter().map(hit_aspect).collect();
        let (width, height, gap) = (self.layout_width, f64::from(ROW_HEIGHT), self.gap);
        let (mut frames, other_height) = layout::frames(&other_aspects, width, height, gap);
        let (similar, similar_height) = layout::frames(&similar_aspects, width, height, gap);
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
        let rows = qt_int(self.results.len());
        if rows > 0 {
            let root = QModelIndex::default();
            let (top, bottom) = (self.index(0, 0, &root), self.index(rows - 1, 0, &root));
            self.as_mut().data_changed(&top, &bottom, &QList::from(&[ROLE_PICKED][..]));
        }
        let picks = self.results.picks();
        let other_picked = qt_int(picks.picked_in(self.results.first_range()));
        let similar_picked = qt_int(picks.picked_in(self.results.second_range()));
        let picked_count = qt_int(picks.count());
        self.as_mut().set_picked_count(picked_count);
        self.as_mut().set_other_picked(other_picked);
        self.as_mut().set_similar_picked(similar_picked);
        self.refresh_preview();
    }

    pub(crate) fn set_preview_item(mut self: Pin<&mut Self>, value: i32) {
        if self.preview_item == value {
            return;
        }
        self.as_mut().rust_mut().preview_item = value;
        self.as_mut().preview_item_changed();
        self.refresh_preview();
    }

    /// The preview's values for its item: the picture's URL, its size and its pick (empty for no item).
    fn refresh_preview(mut self: Pin<&mut Self>) {
        let shown = crate::rows::row_of(self.preview_item).and_then(|i| self.results.get(i).map(|hit| (i, hit)));
        let (url, width, height, picked) = match shown {
            Some((i, hit)) => {
                (QString::from(&original_url(hit.id)), qt_side(hit.width), qt_side(hit.height), self.results.picks().is_picked(i))
            }
            None => (QString::default(), 0, 0, false),
        };
        self.as_mut().set_preview_url(url);
        self.as_mut().set_preview_width(width);
        self.as_mut().set_preview_height(height);
        self.as_mut().set_preview_picked(picked);
    }

    fn poll(mut self: Pin<&mut Self>) {
        let poll = search().poll();
        // The results first: whatever watches `busy` going off finds them in place.
        if let Some(found) = poll.found {
            self.as_mut().show(Results::new(found.other, found.similar));
        }
        // The results whose files died come off the grid (their ids are the search's, so an older search's are ignored).
        if !poll.dead.is_empty() {
            self.as_mut().drop_dead(&poll.dead);
        }
        if let Some(status) = poll.status {
            self.as_mut().set_status(QString::from(&text::yandex_status(&status)));
        }
        self.set_busy(poll.busy);
    }

    pub(crate) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(i) = crate::rows::row_of(index.row()) else {
            return QVariant::default();
        };
        let Some(hit) = self.results.get(i) else {
            return QVariant::default();
        };
        let frame = self.frames.get(i).copied().unwrap_or_default();
        match role {
            ROLE_THUMB => QVariant::from(&QString::from(&picture_url(hit.id))),
            ROLE_IMAGE_WIDTH => QVariant::from(&qt_side(hit.width)),
            ROLE_IMAGE_HEIGHT => QVariant::from(&qt_side(hit.height)),
            ROLE_PICKED => QVariant::from(&self.results.picks().is_picked(i)),
            ROLE_FRAME_X => QVariant::from(&frame.x),
            ROLE_FRAME_Y => QVariant::from(&frame.y),
            ROLE_FRAME_WIDTH => QVariant::from(&frame.width),
            ROLE_FRAME_HEIGHT => QVariant::from(&frame.height),
            _ => QVariant::default(),
        }
    }

    pub(crate) fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
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

    pub(crate) fn row_count(&self, _parent: &QModelIndex) -> i32 {
        qt_int(self.results.len())
    }

    pub(crate) fn toggle_pick(mut self: Pin<&mut Self>, item: i32) {
        let Some(i) = crate::rows::row_of(item) else { return };
        self.as_mut().rust_mut().results.picks_mut().toggle(i);
        self.picks_changed();
    }

    pub(crate) fn pick_to(mut self: Pin<&mut Self>, item: i32) {
        let Some(i) = crate::rows::row_of(item) else { return };
        self.as_mut().rust_mut().results.picks_mut().pick_to(i);
        self.picks_changed();
    }

    pub(crate) fn set_picked(mut self: Pin<&mut Self>, first: i32, count: i32, picked: bool) {
        let range = self.range(first, count);
        self.as_mut().rust_mut().results.picks_mut().set(range, picked);
        self.picks_changed();
    }

    pub(crate) fn show_failure(mut self: Pin<&mut Self>, text: &QString) {
        let text = String::from(text);
        self.as_mut().set_status(QString::from(&text));
    }

    pub(crate) fn download_picked(mut self: Pin<&mut Self>) {
        let Some(folder) = crate::picker::take(crate::picker::state::Purpose::YandexSave) else {
            self.as_mut().set_status(QString::from(&text::yandex_no_folder()));
            return;
        };
        let hits: Vec<Hit> = self.results.picked().cloned().collect();
        if hits.is_empty() {
            self.as_mut().set_status(QString::from(&text::yandex_nothing_picked()));
            return;
        }
        let files = hits.len();
        self.as_mut().ensure_pump();
        // The app refuses a search or a save already running (the status line keeps showing its progress), and
        // remembers the folder when it saves.
        match search().save(hits, folder) {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => return self.not_started("save", &e),
        }
        self.as_mut().set_busy(true);
        self.as_mut().set_status(QString::from(&text::yandex_status(&Status::Working(Phase::Saving { files }))));
    }

    /// A search or a save (`what`) whose thread the system refused: the log says why, and so does the status line.
    /// Nothing runs, so the panel isn't busy.
    fn not_started(mut self: Pin<&mut Self>, what: &str, e: &std::io::Error) {
        pg_log::warn("yandex", &format!("the {what} didn't start: {e}"));
        self.as_mut().set_busy(false);
        self.as_mut().set_status(QString::from(&text::yandex_not_started(&e.to_string())));
    }
}

