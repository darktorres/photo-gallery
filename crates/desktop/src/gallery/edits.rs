//! The edits the model carries out: turns, ratings, border marks, and the cropped copies.

use core::pin::Pin;

use cxx_qt::CxxQtType;
use pg_client::grid::Changed;
use pg_model::grid::Borders;
use pg_model::ids::FileId;
use pg_model::notice::{EditOp, Notice};
use pg_model::rating::Rating;
use pg_model::turns::Turns;

use super::qobject;
use super::roles::qt_int;
use super::state::fresh_entry;

impl qobject::GalleryModel {
    /// Turn a file by quarter turns (1 = clockwise, -1 = counterclockwise), losslessly, in the background: the
    /// controller's `Turn` command.
    pub(crate) fn rotate(mut self: Pin<&mut Self>, item: i32, turns: i32) {
        let Ok(at) = usize::try_from(item) else { return };
        let Some(id) = self.grid.id_at(item) else { return };
        let turns = Turns::new(u8::try_from(turns.rem_euclid(4)).unwrap_or(u8::MAX));
        if turns == Turns::NONE {
            return;
        }
        if let Err(e) = crate::app::get().library().turn(id, turns) {
            edit_failed(EditOp::Rotate, &e);
            return;
        }
        crate::app::resolved_part("edit-failed", EditOp::Rotate.code());
        let loading = self.loading;
        {
            let mut this = self.as_mut().rust_mut();
            this.grid.edit(item, loading, |e| e.pending_turns = e.pending_turns.plus(turns));
            this.forget_details();
        }
        // An odd turn changes the item's shape, and so the rows.
        if pg_model::orientation::turns_swap(turns) {
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
        // Not the queued turns (`file_writes`): the writer deletes a turn's row once it's written, so these are
        // exactly the entries that still show one.
        let turned = self.as_mut().rust_mut().grid.refresh_turned(fresh_entry);
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
    /// `Rate` command.
    pub(crate) fn set_rating(mut self: Pin<&mut Self>, item: i32, rating: i32) {
        let Ok(at) = usize::try_from(item) else { return };
        let Some(id) = self.grid.id_at(item) else { return };
        let stars = u8::try_from(rating.clamp(0, 5)).unwrap_or(0);
        // Zero stars is no rating: the catalog keeps no row for it.
        let rating = Rating::new(stars).filter(|r| r.get() > 0);
        if let Err(e) = crate::app::get().edit().rate(id, rating) {
            edit_failed(EditOp::Rate, &e);
            return;
        }
        crate::app::resolved_part("edit-failed", EditOp::Rate.code());
        let loading = self.loading;
        {
            let mut this = self.as_mut().rust_mut();
            this.grid.edit(item, loading, |e| e.rating = rating);
            this.forget_details();
        }
        self.as_mut().items_changed(&[at]);
        self.as_mut().bump_revision();
    }

    /// Mark the item's borders as not a border, or take the mark back. Only items with borders found.
    pub(crate) fn set_border_marked(mut self: Pin<&mut Self>, item: i32, marked: bool) {
        let Ok(at) = usize::try_from(item) else { return };
        let Some(e) = self.grid.item(item).copied() else { return };
        if !matches!(e.borders, Borders::Found | Borders::Marked) || (e.borders == Borders::Marked) == marked {
            return;
        }
        if let Err(err) = crate::app::get().edit().set_border_mark(e.id, marked) {
            edit_failed(EditOp::MarkBorder, &err);
            return;
        }
        crate::app::resolved_part("edit-failed", EditOp::MarkBorder.code());
        let loading = self.loading;
        self.as_mut()
            .rust_mut()
            .grid
            .edit(item, loading, |entry| entry.borders = if marked { Borders::Marked } else { Borders::Found });
        self.as_mut().items_changed(&[at]);
        self.as_mut().bump_revision();
    }

    pub(crate) fn save_cropped_copies_picked(&self) -> i32 {
        // The picker says `done` only with its folder, so no folder means nothing to save.
        let Some(folder) = crate::picker::take(crate::picker::state::Purpose::SaveCrops) else {
            return 0;
        };
        let ids: Vec<FileId> =
            self.grid.entries().iter().filter(|e| e.borders == Borders::Found).map(|e| e.id).collect();
        let n = qt_int(ids.len());
        if n > 0 {
            crate::app::get().library().save_crops(ids, folder);
        }
        n
    }
}

/// The inbox says that the edit `op` couldn't be saved, for `error`.
fn edit_failed(op: EditOp, error: &impl std::fmt::Display) {
    crate::app::post(Notice::EditFailed { op, error: crate::app::error_text(error) });
}
