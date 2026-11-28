//! Reverse image search on Yandex, from the viewer: the Qt glue over `pg_app::yandex`. A thread subscribes to the search's
//! changes and queues a `poll()` onto the Qt thread for each one, as the library's change pump does; QML doesn't poll.
//! The results are the model's rows, as the grid's are (`GalleryModel`): the other sizes first (`otherCount` of them),
//! then the similar images (`similarCount`). A row's `picked` role follows the picks to save. Each result's frame (its
//! `frameX`, `frameY`, `frameWidth` and `frameHeight` roles) places it in the panel's justified layout: the panel sets
//! `layoutWidth` and the gallery's `gap`; each group's block is `otherHeight` or `similarHeight` high. A result's size
//! (its `imageWidth` and `imageHeight` roles, 0 until known) is its file's, as it decodes once fetched (`Poll::sized`):
//! until then its frame is square, and the results are laid out again as their sizes come in. Yandex's stated sizes are
//! never shown.
//! The preview shows one result (`previewItem`): its URL, size and pick are properties, set whenever the item or its
//! pick changes. The words the panel shows are `pg_wording`'s (and the QML's own), from the outcomes the app
//! returns as data (`pg_model::yandex`).
use crate::bridges::yandex_state::YandexSearchRust;

use crate::bridges::yandex_bridge::qobject;

use std::path::Path;
use std::pin::Pin;

use crate::rows::{file_id_of, qt_int};
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use pg_app::yandex::{Hit, Measured};
use pg_client::image::ImageRequest;
use pg_client::layout;
use pg_client::results::Results;
use pg_model::notice::Notice;
use pg_model::size::RowHeight;
use pg_model::yandex::{Phase, YandexStatus};
use pg_platform::native::PATH_RULES;
use pg_wording::{Message, notice_line, render};

/// A status line's text. A panic's line names the log's file as it is on disk: the log is open before the window is made.
fn status_text(status: &YandexStatus) -> String {
    let log = pg_log::path().map(|p| p.display().to_string()).unwrap_or_default();
    pg_wording::yandex_status(status, &log)
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
/// The URL of the result's original when it is an animation (`""` for a still): the tile plays it.
const ROLE_ANIMATION_URL: i32 = 265;
/// Whether the result's frames are mirrored before their turns, and the quarter turns (its file's EXIF orientation).
const ROLE_MIRRORED: i32 = 266;
const ROLE_TURNS: i32 = 267;

/// The height a row of results is laid out at: fixed, at the size the gallery's rows start at. The search makes its grid
/// pictures at this height. Not the gallery's row height range (`pg_model::size::ROW_HEIGHT`).
const RESULT_ROW_HEIGHT: RowHeight = RowHeight::DEFAULT;

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

/// A result's aspect ratio, for its frame: its file's, or square until the file is in.
fn hit_aspect(hit: &Hit) -> f64 {
    match hit.size {
        Some((width, height)) if width > 0 && height > 0 => f64::from(width) / f64::from(height),
        _ => 1.0,
    }
}

/// A result's size for the roles and the preview: (width, height), or 0×0 until its file is in.
fn hit_size(hit: &Hit) -> (i32, i32) {
    hit.size.map_or((0, 0), |(width, height)| (qt_int(width), qt_int(height)))
}

/// The URL of the result's original when it is an animation, `""` for a still (what the preview and a tile play).
fn animation_url(hit: &Hit) -> QString {
    if hit.animated { QString::from(&original_url(hit.id)) } else { QString::default() }
}

/// The rows of `items` that `sized` names, with their sizes, in row order. A size for a result that is not shown (an
/// older search's, or one taken off) names none.
fn sized_rows(items: &[Hit], sized: &[Measured]) -> Vec<(usize, Measured)> {
    items.iter().enumerate().filter_map(|(row, hit)| sized.iter().find(|m| m.id == hit.id).map(|m| (row, *m))).collect()
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
    pub(crate) fn search_file(mut self: Pin<&mut Self>, file_id: f64) {
        let Some(file) = file_id_of(file_id) else { return };
        match crate::app::get().catalogs().file_path(file) {
            Ok(path) => self.search_photo(&path),
            Err(err) => {
                pg_log::warn("yandex", &format!("file {}: {err:#}", file.get()));
                // The results on the panel are another photo's: they go, so the error is not read as theirs. The panel is for
                // no photo now, so the search that is running (if any) shows nothing here.
                self.as_mut().keep_picks();
                self.as_mut().rust_mut().photo = None;
                self.as_mut().show(Results::default());
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
        // The panel is for this photo from now on, whatever the search does: the last photo's picks are kept for it, and
        // its results leave the screen, so Save can't act on them (a refused search leaves none of the last photo's behind).
        self.as_mut().keep_picks();
        self.as_mut().rust_mut().photo = Some(path.to_path_buf());
        self.as_mut().show(Results::default());
        if let Err(e) = self.as_mut().ensure_pump() {
            return self.not_started("search", &e);
        }
        match search().start(path, RESULT_ROW_HEIGHT) {
            // The app's search is this photo's from now on: its results and progress are the panel's.
            Ok(true) => self.as_mut().rust_mut().searched = Some(path.to_path_buf()),
            // Another photo's search is still running: the panel shows nothing of it (`poll`), and says why.
            Ok(false) => return self.refused("search"),
            Err(e) => return self.not_started("search", &e),
        }
        // The pump's poll reads `busy` with the rest of the state; set it here too, so the panel doesn't wait for it.
        self.as_mut().set_busy(true);
        self.as_mut().set_status(QString::from(&pg_wording::yandex_phase(Phase::Starting)));
    }

    /// The picks of the photo the panel shows are kept for it, by its results' ids, before its results leave the panel.
    /// Nothing is kept when no results are shown (a refused search's clearing keeps the photo's picks as they were).
    fn keep_picks(mut self: Pin<&mut Self>) {
        let Some(photo) = self.photo.clone() else { return };
        if self.results.is_empty() {
            return;
        }
        let picked: Vec<u64> = self.results.picked().map(|hit| hit.id).collect();
        self.as_mut().rust_mut().picks.insert(photo, picked);
    }

    /// The picks kept for the photo the panel shows are put back on its results, the ones with the same ids.
    fn restore_picks(mut self: Pin<&mut Self>) {
        let Some(kept) = self.photo.as_ref().and_then(|photo| self.picks.get(photo)).cloned() else { return };
        let rows: Vec<usize> = self
            .results
            .items()
            .iter()
            .enumerate()
            .filter(|(_, hit)| kept.contains(&hit.id))
            .map(|(row, _)| row)
            .collect();
        for row in rows {
            self.as_mut().rust_mut().results.picks_mut().set(row..row + 1, true);
        }
        self.picks_changed();
    }

    /// Start the thread that polls on each change of the search (once, unless it couldn't start). It subscribes before
    /// any search or save starts, so none of their changes is missed. Without it no result would be shown, so a search
    /// or a save is not started then: the error is returned, and the next one tries again.
    fn ensure_pump(mut self: Pin<&mut Self>) -> std::io::Result<()> {
        if self.pump {
            return Ok(());
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
            return Err(e);
        }
        Ok(())
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
    /// keeps its picture where it went with the removals, and closes if the picture died. Returns how many rows went.
    fn drop_dead(mut self: Pin<&mut Self>, dead: &[u64]) -> usize {
        let runs = dead_runs(self.results.items(), dead);
        if runs.is_empty() {
            return 0;
        }
        let removed: usize = runs.iter().map(|run| run.len()).sum();
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
        removed
    }

    /// The results whose files came in take their sizes: each row's size roles change, the results are laid out again
    /// by their aspects (once for all of them), and the preview's size follows.
    fn apply_sizes(mut self: Pin<&mut Self>, sized: &[Measured]) {
        let rows = sized_rows(self.results.items(), sized);
        let (Some(&(first, _)), Some(&(last, _))) = (rows.first(), rows.last()) else {
            return;
        };
        {
            let mut state = self.as_mut().rust_mut();
            for &(row, measured) in &rows {
                if let Some(hit) = state.results.get_mut(row) {
                    hit.size = Some(measured.size);
                    hit.animated = measured.animated;
                    hit.orientation = measured.orientation;
                }
            }
        }
        let root = QModelIndex::default();
        let (top, bottom) = (self.index(qt_int(first), 0, &root), self.index(qt_int(last), 0, &root));
        let roles = [ROLE_IMAGE_WIDTH, ROLE_IMAGE_HEIGHT, ROLE_ANIMATION_URL, ROLE_MIRRORED, ROLE_TURNS];
        self.as_mut().data_changed(&top, &bottom, &QList::from(&roles[..]));
        self.as_mut().relayout();
        self.refresh_preview();
    }

    /// The id of the result the preview shows, if it shows one.
    fn shown_id(&self) -> Option<u64> {
        let i = crate::rows::row_of(self.preview_item)?;
        self.results.get(i).map(|hit| hit.id)
    }

    /// Lay the results out in the panel's width, at `RESULT_ROW_HEIGHT` and the gallery's gap, as the gallery lays its
    /// rows out: each group's rows are justified (`layout::frames`), from the top of the group's block, and the blocks'
    /// heights are set. Each result's frame is read again (its roles).
    fn relayout(mut self: Pin<&mut Self>) {
        self.as_mut().compute_layout();
        self.emit_frames();
    }

    /// The frames of the results, and the two blocks' heights, from the results and the panel's width and gap. Inside a
    /// model reset this is run before the reset ends, so `data()` reads the frames of the rows the reset makes.
    fn compute_layout(mut self: Pin<&mut Self>) {
        let other_aspects: Vec<f64> = self.results.first().iter().map(hit_aspect).collect();
        let similar_aspects: Vec<f64> = self.results.second().iter().map(hit_aspect).collect();
        let (width, height, gap) = (self.layout_width, f64::from(RESULT_ROW_HEIGHT.get()), self.gap);
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

    /// The previous (`by` -1) or next (`by` 1) result of the preview, wrapping round the results.
    pub(crate) fn step_preview(self: Pin<&mut Self>, by: i32) {
        let total = self.other_count + self.similar_count;
        if total > 0 {
            let next = (self.preview_item + by).rem_euclid(total);
            self.set_preview_item(next);
        }
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
        let (url, width, height, picked, prev, next, mirrored, turns, animation) = match shown {
            Some((i, hit)) => {
                // The result `k` steps from the shown one, wrapping as the preview's ←/→ do.
                let around = |k: usize| {
                    if len < 2 {
                        String::new()
                    } else {
                        self.results.get(k % len).map_or_else(String::new, |h| original_url(h.id))
                    }
                };
                let (width, height) = hit_size(hit);
                // A result's file is turned by its own EXIF orientation alone (no catalog turns); an animation's URL is
                // set only when the file is one, so the preview plays it and no still.
                let (mirrored, turns) = hit.orientation.parts();
                let animation = animation_url(hit);
                (
                    QString::from(&original_url(hit.id)),
                    width,
                    height,
                    self.results.picks().is_picked(i),
                    QString::from(&around(i + len - 1)),
                    QString::from(&around(i + 1)),
                    mirrored,
                    i32::from(turns.get()),
                    animation,
                )
            }
            None => {
                (QString::default(), 0, 0, false, QString::default(), QString::default(), false, 0, QString::default())
            }
        };
        self.as_mut().set_preview_url(url);
        self.as_mut().set_preview_prev_url(prev);
        self.as_mut().set_preview_next_url(next);
        self.as_mut().set_preview_width(width);
        self.as_mut().set_preview_height(height);
        self.as_mut().set_preview_picked(picked);
        // The turn is set before the animation's URL: a player starts with its own item's turn, never the last one's.
        self.as_mut().set_preview_mirrored(mirrored);
        self.as_mut().set_preview_turns(turns);
        self.as_mut().set_preview_animation_url(animation);
    }

    fn poll(mut self: Pin<&mut Self>) {
        let poll = search().poll();
        // The app's search is shown only for the panel's photo: another photo's (one still running when this one was asked
        // for) is not the panel's, so its results, status and progress are left out.
        let ours = self.photo.is_some() && self.photo == self.searched;
        // The results first: whatever watches `busy` going off finds them in place.
        if let Some(found) = poll.found.filter(|_| ours) {
            self.as_mut().show(Results::new(found.other, found.similar));
            self.as_mut().restore_picks();
        }
        // The results whose files died come off the grid (their ids are the search's, so an older search's are ignored).
        let dropped = if poll.dead.is_empty() { 0 } else { self.as_mut().drop_dead(&poll.dead) };
        // The results whose files came in show their sizes, and are laid out by them.
        if !poll.sized.is_empty() {
            self.as_mut().apply_sizes(&poll.sized);
        }
        // The status line says what the panel's own request got (`refused`) while the app's search is another photo's.
        if let Some(status) = poll.status.filter(|_| ours) {
            self.as_mut().set_status(QString::from(&status_text(&status)));
        }
        // Results taken off the grid are said on the status line, as the notice that reports them: after the poll's
        // status, which may be older. It is the line a panel with no results left shows, instead of "No results".
        if dropped > 0 {
            let notice = Notice::YandexResultsDropped {
                dropped: u64::try_from(dropped).unwrap_or(u64::MAX),
                found: u64::try_from(poll.total).unwrap_or(u64::MAX),
            };
            self.as_mut().set_status(QString::from(&notice_line(&notice, PATH_RULES)));
        }
        // Not the panel's photo's search: the panel is not busy with it.
        self.set_busy(poll.busy && ours);
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
            ROLE_IMAGE_WIDTH => QVariant::from(&hit_size(hit).0),
            ROLE_IMAGE_HEIGHT => QVariant::from(&hit_size(hit).1),
            ROLE_PICKED => QVariant::from(&self.results.picks().is_picked(i)),
            ROLE_FRAME_X => QVariant::from(&frame.x),
            ROLE_FRAME_Y => QVariant::from(&frame.y),
            ROLE_FRAME_WIDTH => QVariant::from(&frame.width),
            ROLE_FRAME_HEIGHT => QVariant::from(&frame.height),
            ROLE_IS_OTHER => QVariant::from(&(i < self.results.first().len())),
            ROLE_ANIMATION_URL => QVariant::from(&animation_url(hit)),
            ROLE_MIRRORED => QVariant::from(&hit.orientation.parts().0),
            ROLE_TURNS => QVariant::from(&i32::from(hit.orientation.parts().1.get())),
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
            (ROLE_ANIMATION_URL, "animationUrl"),
            (ROLE_MIRRORED, "mirrored"),
            (ROLE_TURNS, "turns"),
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
        // The picks are checked before the folder is taken, so a folder chosen for nothing to save is kept. A pick whose
        // file died while the dialog was open is off the grid already, and the status line says so (`poll`): with no pick
        // left, the save has nothing to do and says nothing over that line.
        let hits: Vec<Hit> = self.results.picked().cloned().collect();
        if hits.is_empty() {
            return;
        }
        let Some(folder) = crate::picker::take(crate::bridges::picker_state::Purpose::YandexSave) else {
            self.as_mut().set_status(QString::from(&render(&Message::YandexNoFolder)));
            return;
        };
        let files = hits.len();
        if let Err(e) = self.as_mut().ensure_pump() {
            return self.not_started("save", &e);
        }
        // The app refuses a search or a save already running (the status line keeps showing its progress), and
        // remembers the folder when it saves.
        match search().save(hits, folder) {
            Ok(true) => {}
            Ok(false) => return self.refused("save"),
            Err(e) => return self.not_started("save", &e),
        }
        self.as_mut().set_busy(true);
        self.as_mut().set_status(QString::from(&status_text(&YandexStatus::Working(Phase::Saving { files }))));
    }

    /// A search or a save (`what`) the app refused because another is running: the status line says so, so the request
    /// is not taken for the running one's (the panel keeps the results it shows, which may be that search's).
    fn refused(mut self: Pin<&mut Self>, what: &str) {
        pg_log::warn("yandex", &format!("the {what} was refused: a search or a save is still running"));
        let error = "a search or a save is still running".to_string();
        self.as_mut().set_status(QString::from(&render(&Message::YandexNotStarted { error })));
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
    fn sizes_go_to_the_rows_of_their_results() {
        let hit = |id: u64| Hit {
            id,
            url: String::new(),
            size: None,
            animated: false,
            orientation: pg_model::orientation::Orientation::UPRIGHT,
            label: pg_app::yandex::Label::OtherSize,
        };
        let measured = |id: u64, size: (u32, u32)| Measured {
            id,
            size,
            animated: false,
            orientation: pg_model::orientation::Orientation::UPRIGHT,
        };
        let items = [hit(10), hit(20), hit(30)];
        let sized = [measured(30, (810, 1080)), measured(99, (1, 1)), measured(10, (4, 3))];
        assert_eq!(
            sized_rows(&items, &sized),
            [(0, measured(10, (4, 3))), (2, measured(30, (810, 1080)))],
            "in row order; an unknown id is no row"
        );
        assert!(sized_rows(&items, &[]).is_empty());
        // Until its file is in, a result is square and shows no size; then it has its file's.
        assert_eq!((hit_aspect(&items[0]), hit_size(&items[0])), (1.0, (0, 0)));
        let known = Hit { size: Some((810, 1080)), ..hit(1) };
        assert_eq!((hit_aspect(&known), hit_size(&known)), (0.75, (810, 1080)));
    }

    #[test]
    fn the_preview_follows_its_picture_through_removals() {
        assert_eq!(preview_item_after(&[10, 20, 30], Some(30)), 2);
        assert_eq!(preview_item_after(&[10, 30], Some(30)), 1, "the picture moved up over a removed one before it");
        assert_eq!(preview_item_after(&[10, 20], Some(30)), -1, "the picture died: the preview closes");
        assert_eq!(preview_item_after(&[10, 20], None), -1);
    }
}
