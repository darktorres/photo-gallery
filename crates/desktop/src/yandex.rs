//! Reverse image search on Yandex, from the viewer: the Qt glue over `pg_app::yandex`. A thread subscribes to the search's
//! changes and queues a `poll()` onto the Qt thread for each one, as the library's change pump does; QML doesn't poll.
//! The results are the model's rows, as the grid's are (`GalleryModel`): the other sizes first (`otherCount` of them),
//! then the similar images (`similarCount`). A row's `picked` role follows the picks to save. Each result's frame (its
//! `frameX`, `frameY`, `frameWidth` and `frameHeight` roles) places it in the panel's justified layout: the panel sets
//! `layoutWidth` and the gallery's `gap`; each group's block is `otherHeight` or `similarHeight` high.
//! The preview shows one result (`previewItem`): its URL, size and pick are properties, set whenever the item or its
//! pick changes. The words the panel shows are formatted here (`pg_client::text` and the QML's own), from the typed
//! outcomes the app returns.

pub(crate) mod state;
use state::YandexSearchRust;

use crate::yandex_bridge::qobject;

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use pg_app::reexport::YandexError;
use pg_app::yandex::{Hit, Phase, SaveFailure, Saved, SearchError, SimilarNote, Status};
use pg_client::layout;
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

/// The height a row of results is laid out at: fixed, at the size the gallery's rows start at.
const ROW_HEIGHT: f64 = 220.0;

impl YandexSearchRust {
    /// The results `first..first + count` that exist.
    fn range(&self, first: i32, count: i32) -> std::ops::Range<usize> {
        self.picks.range(usize::try_from(first).unwrap_or(0), usize::try_from(count).unwrap_or(0))
    }
}

/// The results left when the dead ones are taken off: in order, each with whether it is picked (the picks follow their
/// results, not their positions), and how many of the first `other` are left.
fn without_dead(hits: &[Hit], picks: &Picks, other: usize, dead: &[u64]) -> (Vec<Hit>, Vec<bool>, usize) {
    let mut kept = Vec::new();
    let mut picked = Vec::new();
    let mut other_kept = 0;
    for (i, hit) in hits.iter().enumerate() {
        if dead.contains(&hit.id) {
            continue;
        }
        if i < other {
            other_kept += 1;
        }
        kept.push(hit.clone());
        picked.push(picks.is_picked(i));
    }
    (kept, picked, other_kept)
}

/// The URL of a result's grid picture: the image provider serves it from the search's files.
fn picture_url(id: u64) -> String {
    format!("image://yandex/{id}")
}

/// The URL of a result's file as its host gave it, for the preview at full size.
fn original_url(id: u64) -> String {
    format!("image://yandex/{id}/original")
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
        YandexError::Undecodable(reason) => text::yandex_undecodable(reason),
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
            text::yandex_write_failed(&url, &pg_platform::names::shown(&path), &error.to_string())
        }
    }
}

fn saved_text(saved: Saved) -> String {
    let Saved { folder, written, failed, first_error } = saved;
    let first_error = first_error.map(failure_text);
    text::yandex_saved(written, &pg_platform::names::shown(&folder), failed, first_error.as_deref())
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
            pg_log::warn("yandex", &format!("the change pump didn't start: {e}"));
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
        let other = usize::try_from(self.other_count).unwrap_or(0);
        let (hits, picked, other) = without_dead(&self.hits, &self.picks, other, dead);
        if hits.len() == self.hits.len() {
            return;
        }
        let len = hits.len();
        self.as_mut().begin_reset_model();
        {
            let mut this = self.as_mut().rust_mut();
            this.hits = hits;
            this.picks.reset(len);
            for (i, is_picked) in picked.into_iter().enumerate() {
                if is_picked {
                    this.picks.set(i..i + 1, true);
                }
            }
        }
        self.as_mut().end_reset_model();
        let similar = self.hits.len() - other;
        self.as_mut().set_other_count(qt_int(other));
        self.as_mut().set_similar_count(qt_int(similar));
        self.as_mut().relayout();
        self.picks_changed();
    }

    fn relayout(mut self: Pin<&mut Self>) {
        let other = usize::try_from(self.other_count).unwrap_or(0).min(self.hits.len());
        let aspects: Vec<f64> = self.hits.iter().map(hit_aspect).collect();
        let (other_aspects, similar_aspects) = aspects.split_at(other);
        let (width, height, gap) = (self.layout_width, ROW_HEIGHT, self.gap);
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
        let shown = usize::try_from(self.preview_item).ok().and_then(|i| self.hits.get(i).map(|hit| (i, hit)));
        let (url, width, height, picked) = match shown {
            Some((i, hit)) => (QString::from(&original_url(hit.id)), hit.width, hit.height, self.picks.is_picked(i)),
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
        // The results whose files died come off the grid (their ids are the search's, so an older search's are ignored).
        if !poll.dead.is_empty() {
            self.as_mut().drop_dead(&poll.dead);
        }
        if let Some(status) = poll.status {
            self.as_mut().set_status(QString::from(&status_text(status)));
        }
        self.set_busy(poll.busy);
    }

    pub(crate) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Ok(i) = usize::try_from(index.row()) else {
            return QVariant::default();
        };
        let Some(hit) = self.hits.get(i) else {
            return QVariant::default();
        };
        let frame = self.frames.get(i).copied().unwrap_or_default();
        match role {
            ROLE_THUMB => QVariant::from(&QString::from(&picture_url(hit.id))),
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
        qt_int(self.hits.len())
    }

    pub(crate) fn toggle_pick(mut self: Pin<&mut Self>, item: i32) {
        let Ok(i) = usize::try_from(item) else { return };
        self.as_mut().rust_mut().picks.toggle(i);
        self.picks_changed();
    }

    pub(crate) fn pick_to(mut self: Pin<&mut Self>, item: i32) {
        let Ok(i) = usize::try_from(item) else { return };
        self.as_mut().rust_mut().picks.pick_to(i);
        self.picks_changed();
    }

    pub(crate) fn set_picked(mut self: Pin<&mut Self>, first: i32, count: i32, picked: bool) {
        let range = self.range(first, count);
        self.as_mut().rust_mut().picks.set(range, picked);
        self.picks_changed();
    }

    pub(crate) fn show_failure(mut self: Pin<&mut Self>, text: &QString) {
        let text = String::from(text);
        self.as_mut().set_status(QString::from(&text));
    }

    pub(crate) fn prepare_save(&self) {
        crate::picker::set_start(self.search.start_folder(crate::app::get().prefs()));
    }

    pub(crate) fn download_picked(mut self: Pin<&mut Self>) {
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
        match self.search.save(crate::app::get().prefs(), hits, folder) {
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
        pg_log::warn("yandex", &format!("the {what} didn't start: {e}"));
        self.as_mut().set_busy(false);
        self.as_mut().set_status(QString::from(&text::yandex_not_started(&e.to_string())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: u64) -> Hit {
        Hit { id, url: format!("https://a.example/{id}.jpg"), width: 1, height: 1, suffix: String::new() }
    }

    /// The picks follow their results when a dead one is taken off: the picked results stay picked, in order.
    #[test]
    fn the_picks_stay_on_their_results_when_a_dead_one_goes() {
        let hits = vec![hit(1), hit(2), hit(3), hit(4)];
        let mut picks = Picks::new(4);
        picks.set(0..1, true);
        picks.set(2..4, true);
        let (kept, picked, other) = without_dead(&hits, &picks, 2, &[2]);
        assert_eq!(kept.iter().map(|h| h.id).collect::<Vec<_>>(), [1, 3, 4]);
        assert_eq!(picked, [true, true, true]);
        assert_eq!(other, 1, "result 2 was one of the other sizes; one of them is left");
    }
}
