//! The edits the model carries out: turns, ratings, border marks, and the cropped copies. A turn, a rating and a border
//! mark are written off the Qt thread (`worker`), and the item shows them when the write is made; a write that fails is
//! a notice, and the item is left as it was.

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use pg_client::grid::Changed;
use pg_model::grid::{Borders, Entry};
use pg_model::ids::FileId;
use pg_model::notice::{EditOp, NothingKind, Notice, Op};
use pg_model::rating::Rating;
use pg_model::turns::Turns;

use super::qobject;
use crate::rows::qt_int;
use crate::worker;

impl qobject::GalleryModel {
    /// Turn a file by quarter turns clockwise (three is counterclockwise), losslessly, in the background: the
    /// controller's `Turn` command. The item shows the turn at press (`pending_turns`), before the write, which waits on
    /// the writes' thread for the catalog's write lock; a turn that is not written is taken back off the item.
    pub(crate) fn rotate(mut self: Pin<&mut Self>, item: i32, turns: Turns) {
        let Some(id) = self.file_at(item) else { return };
        if turns == Turns::NONE {
            return;
        }
        self.as_mut().pending_turn(id, turns, true);
        self.as_mut().unqueued_turn(id, turns, true);
        let model = self.qt_thread();
        worker::run(move || {
            // Done whatever the write does: a write that panics still takes its turn back off the item.
            let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                edit_outcome(EditOp::Rotate, crate::app::get().library().turn(id, turns)).is_some()
            }))
            .unwrap_or(false);
            let _ = model.queue(move |model| model.turn_done(id, turns, written));
        });
    }

    /// A turn's write is done. One that was not written is taken back off the item; one that was stays pending until the
    /// catalog's read shows it written (`refresh_turned`).
    fn turn_done(mut self: Pin<&mut Self>, id: FileId, turns: Turns, written: bool) {
        // The writer is done with the press: it is queued in the catalog now (or refused), so a list read from here counts
        // it.
        self.as_mut().unqueued_turn(id, turns, false);
        if written {
            // The file changed in the catalog: what was read of it before is stale.
            self.as_mut().rust_mut().forget_details();
        } else {
            self.as_mut().pending_turn(id, turns, false);
        }
    }

    /// Records a press as not queued in the catalog yet (`add`), or takes one off when the writer is done with it
    /// (`turn_done`). The record is `unqueued`, what `with_unqueued` puts back on a list that arrives before the queue.
    fn unqueued_turn(mut self: Pin<&mut Self>, id: FileId, turns: Turns, add: bool) {
        let mut this = self.as_mut().rust_mut();
        let before = this.unqueued.get(&id).copied().unwrap_or(Turns::NONE);
        let after = if add { before.plus(turns) } else { unturned(before, turns) };
        if after == Turns::NONE {
            this.unqueued.remove(&id);
        } else {
            this.unqueued.insert(id, after);
        }
    }

    /// The entries a list read brings, with the presses the catalog does not count yet put back on them. The catalog
    /// counts a turn as pending once the writer has queued it, so a read made before that has none for the press: it is
    /// counted again here, until `turn_done` takes it off. An entry the read already has pending turns for is left as
    /// read, so a press queued before the read is not counted twice (a press queued before the read, and another not yet
    /// queued, is undercounted until the write lands and the turned files are read again).
    pub(super) fn with_unqueued(&self, mut entries: Vec<Entry>) -> Vec<Entry> {
        if self.unqueued.is_empty() {
            return entries;
        }
        for entry in &mut entries {
            if entry.pending_turns == Turns::NONE
                && let Some(&turns) = self.unqueued.get(&entry.id)
            {
                entry.pending_turns = turns;
            }
        }
        entries
    }

    /// Puts a turn on the file's item as waiting to be written (`add`), or takes one back (a write that failed), and shows
    /// the item with it. A turn that swaps the picture's axes changes its shape, and so the rows.
    fn pending_turn(mut self: Pin<&mut Self>, id: FileId, turns: Turns, add: bool) {
        let Some(at) = self.grid.position_of(id) else { return };
        let loading = self.loading;
        {
            let mut this = self.as_mut().rust_mut();
            this.grid.edit(at, loading, |e| {
                e.pending_turns = if add { e.pending_turns.plus(turns) } else { unturned(e.pending_turns, turns) };
            });
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
        let fresh = self.with_unqueued(fresh);
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

    /// Rate the item (`None` takes its rating away), in the catalog and, through the writer, in its file: the controller's
    /// `Rate` command. The write is made on the writes' thread, and the item shows the rating once it is made.
    pub(crate) fn set_rating(self: Pin<&mut Self>, at: usize, rating: Option<Rating>) {
        let Some(id) = self.grid.id_at(at) else { return };
        let model = self.qt_thread();
        worker::run(move || {
            if edit_outcome(EditOp::Rate, crate::app::get().edit().rate(id, rating)).is_some() {
                let _ = model.queue(move |model| model.rating_written(id, rating));
            }
        });
    }

    /// A rating was written for the file: the viewer's popup says so (`ratingWritten`), and the item shows it.
    fn rating_written(mut self: Pin<&mut Self>, id: FileId, rating: Option<Rating>) {
        self.as_mut().emit_rating_written(rating.map_or(0, |r| i32::from(r.get())));
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
            // The key was received and has nothing to mark: the user is told why nothing changed.
            crate::app::post(Notice::NothingToDo { what: NothingKind::BorderMarkNoBorders });
            return;
        }
        if self.mark_target(e.id, e.borders) == marked {
            return;
        }
        self.as_mut().rust_mut().queue_mark(e.id, marked);
        let model = self.qt_thread();
        worker::run(move || {
            // Done whatever the write does: a write that panics still lets go of its queued mark, so the item does not
            // keep a target that was never written.
            let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                edit_outcome(EditOp::MarkBorder, crate::app::get().edit().set_border_mark(e.id, marked))
            }))
            .ok()
            .flatten();
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

    /// Crops the listed files with borders found into the folder the user chose. Nothing queued is said to the user: no
    /// file with borders found, or a library that has not started yet (`save_crops` is a no-op then).
    pub(crate) fn save_cropped_copies_picked(&self) -> i32 {
        // The picker says `done` only with its folder, so no folder means nothing to save.
        let Some(folder) = crate::picker::take(crate::bridges::picker_state::Purpose::SaveCrops) else {
            return 0;
        };
        let ids = self.borders_found();
        if ids.is_empty() {
            crate::app::post(Notice::NothingToDo { what: NothingKind::CropNoBorders });
            return 0;
        }
        if crate::library::running().is_none() {
            crate::app::post(Notice::NothingToDo { what: NothingKind::CropLibraryNotRunning });
            return 0;
        }
        let n = qt_int(ids.len());
        crate::app::get().library().save_crops(ids, folder);
        n
    }

    /// How many files the cut dialog says its borders come off: the files the copy action takes.
    pub(crate) fn cut_borders_count(&self) -> i32 {
        qt_int(self.borders_found().len())
    }

    /// The user confirmed "Cut borders from originals" in CutBordersDialog: the library rewrites those originals with
    /// their borders cut off, in the background. The only call that passes `confirmed` as true. Returns how many it asked for.
    pub(crate) fn cut_borders_confirmed(&self) -> i32 {
        self.with_borders_found(|ids| crate::app::get().library().cut_borders(ids, true))
    }

    /// The listed files whose borders were found (and not marked as not a border).
    fn borders_found(&self) -> Vec<FileId> {
        self.grid.entries().iter().filter(|e| e.borders == Borders::Found).map(|e| e.id).collect()
    }

    /// Hands those files to `act` when there are any, and returns how many there are.
    fn with_borders_found(&self, act: impl FnOnce(Vec<FileId>)) -> i32 {
        let ids = self.borders_found();
        let n = qt_int(ids.len());
        if n > 0 {
            act(ids);
        }
        n
    }
}

/// The quarter turns `pending` less `taken` (the turns a write that failed takes back), modulo four.
fn unturned(pending: Turns, taken: Turns) -> Turns {
    Turns::new(pending.get() + 4 - taken.get())
}

/// The inbox says that the edit `op` couldn't be saved, for `error`.
fn edit_outcome<T, E: std::fmt::Display>(op: EditOp, result: Result<T, E>) -> Option<T> {
    crate::app::observe(pg_model::notice::Op::Edit(op), result, |e| Notice::EditFailed {
        op,
        error: crate::app::error_text(e),
    })
}
