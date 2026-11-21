//! The edits the model carries out: turns, ratings, border marks, and the cropped copies. A turn, a rating and a border
//! mark are written off the Qt thread (`worker`), and the item shows them when the write is made; a write that fails is
//! a notice, and the item is left as it was.

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use pg_client::grid::Changed;
use pg_model::grid::{Borders, Entry};
use pg_model::ids::FileId;
use pg_model::notice::{EditOp, Notice, Op};
use pg_model::rating::Rating;
use pg_model::turns::Turns;

use super::qobject;
use crate::rows::qt_int;
use crate::worker;

impl qobject::GalleryModel {
    /// Turn a file by quarter turns clockwise (three is counterclockwise), losslessly, in the background: the
    /// controller's `Turn` command. The turn is queued for the file on the writes' thread, and the item shows it once
    /// it is queued (`turn_queued`).
    pub(crate) fn rotate(self: Pin<&mut Self>, item: i32, turns: Turns) {
        let Some(id) = self.file_at(item) else { return };
        if turns == Turns::NONE {
            return;
        }
        let model = self.qt_thread();
        worker::run(move || {
            if edit_outcome(EditOp::Rotate, crate::app::get().library().turn(id, turns)).is_some() {
                let _ = model.queue(move |model| model.turn_queued(id, turns));
            }
        });
    }

    /// A turn was queued for the file: the item shows it (the item's shape, when the turn swaps the axes, changes the
    /// rows).
    fn turn_queued(mut self: Pin<&mut Self>, id: FileId, turns: Turns) {
        let Some(at) = self.grid.position_of(id) else { return };
        let loading = self.loading;
        {
            let mut this = self.as_mut().rust_mut();
            this.grid.edit(at, loading, |e| e.pending_turns = e.pending_turns.plus(turns));
            this.forget_details();
        }
        // An odd turn changes the item's shape, and so the rows.
        if turns.swaps_axes() {
            let plan = self.as_mut().rust_mut().grid.plan(Changed::Items(&[at]));
            self.as_mut().show(plan);
        } else {
            self.as_mut().items_changed(&[at]);
        }
        self.as_mut().bump_revision();
    }

    /// Files were turned: re-read the items still waiting for a turn (new image, size and version), in place, so
    /// nothing is renumbered under the viewer (the controller's `RefreshTurned` command, and a list that arrives).
    pub(crate) fn refresh_turned(mut self: Pin<&mut Self>) {
        // The list on its way may have been read before these turns were written: do it again when it arrives.
        let loading = self.loading;
        self.as_mut().rust_mut().grid.note_turns_written(loading);
        self.request_turned();
    }

    /// Asks the reader for the entries of the items still waiting for a turn (`Grid::turned_request`). What it reads
    /// is put in by `turned_read`.
    fn request_turned(&self) {
        let Some((ids, revision)) = self.grid.turned_request() else { return };
        crate::app::session().entries(ids, move |read| {
            let Some(fresh) = crate::app::observe(Op::Reload, read, |e| Notice::ReloadFailed {
                error: crate::app::error_text(&format!("{e:#}")),
            }) else {
                return;
            };
            crate::gallery::post(move |model| model.turned_read(revision, fresh));
        });
    }

    /// The entries read for the items waiting for a turn (`request_turned`), put in unless an edit or a new list came
    /// since the read was made: then they are read again.
    fn turned_read(mut self: Pin<&mut Self>, revision: u64, fresh: Vec<Entry>) {
        let Some(turned) = self.as_mut().rust_mut().grid.refresh_turned(revision, fresh) else {
            self.request_turned();
            return;
        };
        if turned.changed.is_empty() {
            return;
        }
        // A written turn changes the file's size too: what was read of it is stale.
        self.as_mut().rust_mut().forget_details();
        if turned.reshaped {
            let plan = self.as_mut().rust_mut().grid.plan(Changed::Items(&turned.changed));
            self.as_mut().show(plan);
        } else {
            self.as_mut().items_changed(&turned.changed);
        }
        self.as_mut().bump_revision();
    }

    /// Rate the item (0 takes its rating away), in the catalog and, through the writer, in its file: the controller's
    /// `Rate` command. The write is made on the writes' thread, and the item shows the rating once it is made.
    pub(crate) fn set_rating(self: Pin<&mut Self>, at: usize, stars: Rating) {
        let Some(id) = self.grid.id_at(at) else { return };
        // Zero stars is no rating: the catalog keeps no row for it.
        let rating = Some(stars).filter(|r| r.get() > 0);
        let model = self.qt_thread();
        worker::run(move || {
            if edit_outcome(EditOp::Rate, crate::app::get().edit().rate(id, rating)).is_some() {
                let _ = model.queue(move |model| model.rating_written(id, rating));
            }
        });
    }

    /// A rating was written for the file: the item shows it.
    fn rating_written(mut self: Pin<&mut Self>, id: FileId, rating: Option<Rating>) {
        let Some(at) = self.grid.position_of(id) else { return };
        let loading = self.loading;
        {
            let mut this = self.as_mut().rust_mut();
            this.grid.edit(at, loading, |e| e.rating = rating);
            this.forget_details();
        }
        self.as_mut().items_changed(&[at]);
        self.as_mut().bump_revision();
    }

    /// Mark the item's borders as not a border, or take the mark back. Only items with borders found. The mark is
    /// written on the writes' thread, and the item shows it once it is written. A toggle reads the mark the writes
    /// queued before it leave (`GalleryRust::mark_target`), so two quick toggles are two changes.
    pub(crate) fn set_border_marked(mut self: Pin<&mut Self>, item: i32, marked: bool) {
        let Some(at) = crate::rows::row_of(item) else { return };
        let Some(e) = self.grid.item(at).copied() else { return };
        if !matches!(e.borders, Borders::Found | Borders::Marked) {
            return;
        }
        if self.mark_target(e.id, e.borders) == marked {
            return;
        }
        self.as_mut().rust_mut().queue_mark(e.id, marked);
        let model = self.qt_thread();
        worker::run(move || {
            let written = edit_outcome(EditOp::MarkBorder, crate::app::get().edit().set_border_mark(e.id, marked));
            let _ = model.queue(move |model| model.border_mark_done(e.id, marked, written.is_some()));
        });
    }

    /// A border mark's write is done (written or not): the item shows the mark when it was written.
    fn border_mark_done(mut self: Pin<&mut Self>, id: FileId, marked: bool, written: bool) {
        self.as_mut().rust_mut().mark_done(id);
        if !written {
            return;
        }
        let Some(at) = self.grid.position_of(id) else { return };
        let loading = self.loading;
        self.as_mut()
            .rust_mut()
            .grid
            .edit(at, loading, |entry| entry.borders = if marked { Borders::Marked } else { Borders::Found });
        self.as_mut().items_changed(&[at]);
        self.as_mut().bump_revision();
    }

    pub(crate) fn save_cropped_copies_picked(&self) -> i32 {
        // The picker says `done` only with its folder, so no folder means nothing to save.
        let Some(folder) = crate::picker::take(crate::picker::state::Purpose::SaveCrops) else {
            return 0;
        };
        let ids = self.borders_found();
        let n = qt_int(ids.len());
        if n > 0 {
            crate::app::get().library().save_crops(ids, folder);
        }
        n
    }

    /// How many files the cut dialog says its borders come off: the files the copy action takes.
    pub(crate) fn cut_borders_count(&self) -> i32 {
        qt_int(self.borders_found().len())
    }

    /// The user confirmed "Cut borders from originals" in CutBordersDialog: the library rewrites those originals with
    /// their borders cut off, in the background. The only call that passes `confirmed` as true. Returns how many it asked for.
    pub(crate) fn cut_borders_confirmed(&self) -> i32 {
        let ids = self.borders_found();
        let n = qt_int(ids.len());
        if n > 0 {
            crate::app::get().library().cut_borders(ids, true);
        }
        n
    }

    /// The listed files whose borders were found (and not marked as not a border).
    fn borders_found(&self) -> Vec<FileId> {
        self.grid.entries().iter().filter(|e| e.borders == Borders::Found).map(|e| e.id).collect()
    }
}

/// The inbox says that the edit `op` couldn't be saved, for `error`.
fn edit_outcome<T, E: std::fmt::Display>(op: EditOp, result: Result<T, E>) -> Option<T> {
    crate::app::observe(pg_model::notice::Op::Edit(op), result, |e| Notice::EditFailed {
        op,
        error: crate::app::error_text(e),
    })
}
