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

use crate::rows::qt_int;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use pg_app::yandex::Hit;
use pg_client::image::ImageRequest;
use pg_client::layout;
use pg_client::results::Results;
use pg_client::text::{self, Message, render};
use pg_model::ids::FileId;
use pg_model::yandex::{Phase, YandexStatus};

/// A status line's text. A panic's line names the log's file as it is on disk: the log is open before the window is made.
fn status_text(status: &YandexStatus) -> String {
    let log = pg_log::path().map(|p| p.display().to_string()).unwrap_or_default();
    text::yandex_status(status, &log)
}

const ROLE_THUMB: i32 = 256;
const ROLE_IMAGE_WIDTH: i32 = 257;
const ROLE_IMAGE_HEIGHT: i32 = 258;
const ROLE_PICKED: i32 = 259;
const ROLE_FRAME_X: i32 = 260;
const ROLE_FRAME_Y: i32 = 261;
const ROLE_FRAME_WIDTH: i32 = 262;
const ROLE_FRAME_HEIGHT: i32 = 263;
/// Whether the row is one of the first group's (the other sizes): the panel's two groups show their own rows by it.
const ROLE_IS_OTHER: i32 = 264;

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

/// A result's aspect ratio, for its frame: its size as Yandex gives it, or square when it gives none.
fn hit_aspect(hit: &Hit) -> f64 {
    if hit.width > 0 && hit.height > 0 { f64::from(hit.width) / f64::from(hit.height) } else { 1.0 }
}

/// The runs of consecutive results whose ids are in `dead`, as row ranges, in order.
fn dead_runs(items: &[Hit], dead: &[u64]) -> Vec<std::ops::Range<usize>> {
    let mut runs: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, hit) in items.iter().enumerate() {
        if !dead.contains(&hit.id) {
            continue;
        }
        match runs.last_mut() {
            Some(run) if run.end == i => run.end = i + 1,
            _ => runs.push(i..i + 1),
        }
    }
    runs
}

/// The preview's item once the results are `ids` (in order): the position of the picture it showed (`shown`, its id),
/// or -1 when that picture is gone or nothing was shown.
fn preview_item_after(ids: &[u64], shown: Option<u64>) -> i32 {
    shown.and_then(|id| ids.iter().position(|&x| x == id)).map_or(-1, qt_int)
}

/// The thumbnail turn after row `row` settled: it moves on only when `row` is the row at the turn, and not past the last
/// row of `len` results.
fn settle_turn(turn: i32, row: i32, len: usize) -> i32 {
    if row == turn && turn < qt_int(len) { turn + 1 } else { turn }
}

/// The thumbnail turn after the rows of `runs` are removed: rows removed before it move it back, and rows from it on
/// leave it where it is (the row that follows takes its place).
fn turn_after_removal(turn: i32, runs: &[std::ops::Range<usize>]) -> i32 {
    let at = usize::try_from(turn).unwrap_or(0);
    let before: usize = runs.iter().map(|run| run.end.min(at).saturating_sub(run.start.min(at))).sum();
    turn - qt_int(before)
}

impl qobject::YandexSearch {
    pub(crate) fn search_file(mut self: Pin<&mut Self>, file_id: i64) {
        match crate::app::get().catalogs().file_path(FileId::new(file_id)) {
            Ok(path) => self.search_photo(&path),
            Err(err) => {
                pg_log::warn("yandex", &format!("file {file_id}: {err:#}"));
                self.as_mut()
                    .set_status(QString::from(&render(&Message::YandexUnreadablePhoto { error: format!("{err:#}") })));
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
        // A poll reads the whole state, so the changes carry nothing: a change that comes in while a poll is queued is
        // read by it (the pump queues no more until it has run).
        let pump = crate::pump::start(
            "yandex changes",
            self.qt_thread(),
            move || {
                changes.recv().map(|()| {
                    changes.take();
                })
            },
            |_: &mut (), ()| {},
            |search, pending| {
                crate::pump::Pending::take(&pending);
                search.poll();
            },
        );
        // Without its thread the panel isn't told the search's changes: the log says why, and the next search tries
        // again.
        if let Err(e) = pump {
            pg_log::warn("yandex", &format!("the change pump didn't start: {e:#}"));
            self.as_mut().rust_mut().pump = false;
        }
    }

    /// Show these results, none picked. The counts and the layout are set inside the reset, so that no row is ever
    /// shown with a count or a frame that does not match the results (a delegate reads them as it is made).
    fn show(mut self: Pin<&mut Self>, results: Results<Hit>) {
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().results = results;
        self.as_mut().set_group_counts();
        // The new results' pictures load from the first row, as the tiles are made inside the reset.
        self.as_mut().set_thumb_turn(0);
        self.as_mut().compute_layout();
        self.as_mut().end_reset_model();
        self.picks_changed();
    }

    /// A row's picture loaded or failed: the turn moves on if that is the row at it (the next row's picture may load).
    pub(crate) fn thumb_settled(mut self: Pin<&mut Self>, row: i32) {
        let turn = settle_turn(self.thumb_turn, row, self.results.len());
        if turn != self.thumb_turn {
            self.as_mut().set_thumb_turn(turn);
        }
    }

    /// The groups' counts, from the results.
    fn set_group_counts(mut self: Pin<&mut Self>) {
        let (other, similar) = (qt_int(self.results.first().len()), qt_int(self.results.second().len()));
        self.as_mut().set_other_count(other);
        self.as_mut().set_similar_count(similar);
    }

    /// After the results changed (not by a reset): the groups' counts, the layout and the picks.
    fn groups_changed(mut self: Pin<&mut Self>) {
        self.as_mut().set_group_counts();
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

    /// The results whose files died come off the grid, one run of consecutive rows at a time (the model says which rows
    /// went). Each run goes highest first, so the rows below it keep their indices; the frames of the rows go with them,
    /// and the layout of the rest is made once all are gone. The picks of the others stay on their results. The preview
    /// keeps its picture where it went with the removals, and closes if the picture died.
    fn drop_dead(mut self: Pin<&mut Self>, dead: &[u64]) {
        let runs = dead_runs(self.results.items(), dead);
        if runs.is_empty() {
            return;
        }
        let shown = self.shown_id();
        let turn = turn_after_removal(self.thumb_turn, &runs);
        let root = QModelIndex::default();
        for run in runs.into_iter().rev() {
            self.as_mut().begin_remove_rows(&root, qt_int(run.start), qt_int(run.end) - 1);
            let mut state = self.as_mut().rust_mut();
            state.results.remove_range(run.clone());
            let end = run.end.min(state.frames.len());
            state.frames.drain(run.start.min(end)..end);
            self.as_mut().end_remove_rows();
        }
        self.as_mut().set_thumb_turn(turn);
        // Before the groups' counts and the layout are set: the preview's item is then one the results have.
        let ids: Vec<u64> = self.results.items().iter().map(|hit| hit.id).collect();
        self.as_mut().set_preview_item(preview_item_after(&ids, shown));
        self.groups_changed();
    }

    /// The id of the result the preview shows, if it shows one.
    fn shown_id(&self) -> Option<u64> {
        let i = crate::rows::row_of(self.preview_item)?;
        self.results.get(i).map(|hit| hit.id)
    }

    /// Lay the results out in the panel's width, at `ROW_HEIGHT` and the gallery's gap, as the gallery lays its rows out:
    /// each group's rows are justified (`layout::frames`), from the top of the group's block, and the blocks' heights
    /// are set. Each result's frame is read again (its roles).
    fn relayout(mut self: Pin<&mut Self>) {
        self.as_mut().compute_layout();
        self.emit_frames();
    }

    /// The frames of the results, and the two blocks' heights, from the results and the panel's width and gap. Inside a
    /// model reset this is run before the reset ends, so `data()` reads the frames of the rows the reset makes.
    fn compute_layout(mut self: Pin<&mut Self>) {
        let other_aspects: Vec<f64> = self.results.first().iter().map(hit_aspect).collect();
        let similar_aspects: Vec<f64> = self.results.second().iter().map(hit_aspect).collect();
        let (width, height, gap) = (self.layout_width, f64::from(ROW_HEIGHT), self.gap);
        let (mut frames, other_height) = layout::frames(&other_aspects, width, height, gap);
        let (similar, similar_height) = layout::frames(&similar_aspects, width, height, gap);
        frames.extend(similar);
        self.as_mut().rust_mut().frames = frames;
        self.as_mut().set_other_height(other_height);
        self.as_mut().set_similar_height(similar_height);
    }

    /// Signals that the frame roles of every row changed (after `compute_layout`).
    fn emit_frames(mut self: Pin<&mut Self>) {
        let rows = qt_int(self.frames.len());
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

    /// The preview's values for its item: the picture's URL, its size and its pick (empty for no item), and the URLs of
    /// the previous and the next result, which the preview loads ahead (empty when there is no other result). The
    /// neighbours are set after the item's own URL, so the preview's picture is the first one requested.
    fn refresh_preview(mut self: Pin<&mut Self>) {
        let shown = crate::rows::row_of(self.preview_item).and_then(|i| self.results.get(i).map(|hit| (i, hit)));
        let len = self.results.len();
        let (url, width, height, picked, prev, next) = match shown {
            Some((i, hit)) => {
                // The result `k` steps from the shown one, wrapping as the preview's ←/→ do.
                let around = |k: usize| {
                    if len < 2 {
                        String::new()
                    } else {
                        self.results.get(k % len).map_or_else(String::new, |h| original_url(h.id))
                    }
                };
                (
                    QString::from(&original_url(hit.id)),
                    qt_int(hit.width),
                    qt_int(hit.height),
                    self.results.picks().is_picked(i),
                    QString::from(&around(i + len - 1)),
                    QString::from(&around(i + 1)),
                )
            }
            None => (QString::default(), 0, 0, false, QString::default(), QString::default()),
        };
        self.as_mut().set_preview_url(url);
        self.as_mut().set_preview_prev_url(prev);
        self.as_mut().set_preview_next_url(next);
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
            self.as_mut().set_status(QString::from(&status_text(&status)));
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
            ROLE_IMAGE_WIDTH => QVariant::from(&qt_int(hit.width)),
            ROLE_IMAGE_HEIGHT => QVariant::from(&qt_int(hit.height)),
            ROLE_PICKED => QVariant::from(&self.results.picks().is_picked(i)),
            ROLE_FRAME_X => QVariant::from(&frame.x),
            ROLE_FRAME_Y => QVariant::from(&frame.y),
            ROLE_FRAME_WIDTH => QVariant::from(&frame.width),
            ROLE_FRAME_HEIGHT => QVariant::from(&frame.height),
            ROLE_IS_OTHER => QVariant::from(&(i < self.results.first().len())),
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
            (ROLE_IS_OTHER, "isOther"),
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
            self.as_mut().set_status(QString::from(&render(&Message::YandexNoFolder)));
            return;
        };
        let hits: Vec<Hit> = self.results.picked().cloned().collect();
        if hits.is_empty() {
            self.as_mut().set_status(QString::from(&render(&Message::YandexNothingPicked)));
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
        self.as_mut().set_status(QString::from(&status_text(&YandexStatus::Working(Phase::Saving { files }))));
    }

    /// A search or a save (`what`) whose thread the system refused: the log says why, and so does the status line.
    /// Nothing runs, so the panel isn't busy.
    fn not_started(mut self: Pin<&mut Self>, what: &str, e: &std::io::Error) {
        pg_log::warn("yandex", &format!("the {what} didn't start: {e:#}"));
        self.as_mut().set_busy(false);
        self.as_mut().set_status(QString::from(&render(&Message::YandexNotStarted { error: e.to_string() })));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_turn_moves_only_when_the_row_at_it_settles() {
        assert_eq!(settle_turn(0, 0, 3), 1);
        assert_eq!(settle_turn(0, 2, 3), 0, "a row past the turn does not move it");
        assert_eq!(settle_turn(3, 3, 3), 3, "past the last row it stays");
    }

    #[test]
    fn removals_before_the_turn_move_it_back_and_those_from_it_on_do_not() {
        // Two runs, the second empty (so the arrays are not single-element ones, which clippy flags).
        assert_eq!(turn_after_removal(5, &[1..3, 0..0]), 3);
        assert_eq!(turn_after_removal(5, &[4..7, 0..0]), 4, "the run from the turn's row on moves nothing before it");
        assert_eq!(turn_after_removal(2, &[4..6, 0..0]), 2);
    }

    #[test]
    fn the_preview_follows_its_picture_through_removals() {
        assert_eq!(preview_item_after(&[10, 20, 30], Some(30)), 2);
        assert_eq!(preview_item_after(&[10, 30], Some(30)), 1, "the picture moved up over a removed one before it");
        assert_eq!(preview_item_after(&[10, 20], Some(30)), -1, "the picture died: the preview closes");
        assert_eq!(preview_item_after(&[10, 20], None), -1);
    }
}
