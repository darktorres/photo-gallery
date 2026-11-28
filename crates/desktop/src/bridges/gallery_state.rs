//! The state of the `GalleryModel` QObject: the bridge's backing struct (the property values, the viewer's item's
//! values, the grid and the details read last), and what it reads of the catalog.

use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use cxx_qt_lib::{QRectF, QString, QStringList};
use pg_client::grid::{Grid, TurnCount};
use pg_model::details::Details;
use pg_model::grid::{Borders, Entry};
use pg_model::ids::FileId;
use pg_model::media::MediaType;
use pg_model::notice::{Notice, Op};
use pg_model::turns::Turns;

/// Hands a details read's answer to the model's thread: the generation the read was asked in, the file, and what the
/// catalog had (`None`: gone, or unreadable). The model's `initialize` sets it (`gallery.rs`); the state holds it rather
/// than calling into the module that holds the state.
pub(crate) type DetailsDelivery = Arc<dyn Fn(u64, FileId, Option<Details>) + Send + Sync>;

/// The model's properties, the list and the details read last.
pub struct GalleryRust {
    pub(crate) count: i32,
    pub(crate) revision: i32,
    pub(crate) croppable: i32,
    pub(crate) loading: bool,
    pub(crate) holding: bool,
    /// The list shown was asked for and could not be read (`listingFailed`).
    pub(crate) listing_failed: bool,
    /// The generation of a list asked for whose read failed, until that list is applied (`apply_failed`): it may wait
    /// under the viewer (`Grid::arrive`), and is shown as failed when it is.
    pub(crate) failed_generation: Option<u64>,
    pub(crate) current: i32,
    pub(crate) selection_info: QString,
    pub(crate) viewing: i32,
    pub(crate) anchor: i32,
    // The viewer's item's values (`refresh_viewed`): the qproperties' backing fields. Before any refresh they are the
    // values of no item, which `refresh_viewed` gives with `viewing` out of range.
    pub(crate) viewed_key: QString,
    pub(crate) viewed_neighbour_keys: QStringList,
    pub(crate) viewed_turns: i32,
    pub(crate) viewed_rating: i32,
    pub(crate) viewed_full_width: i32,
    pub(crate) viewed_full_height: i32,
    /// `viewedKind`'s value, as the domain's type: the bridge's `MediaKind` is read from it (`read_viewed_kind`).
    pub(crate) viewed_kind: MediaType,
    pub(crate) viewed_player_turns: i32,
    pub(crate) viewed_player_mirrored: bool,
    pub(crate) viewed_file_id: i64,
    pub(crate) viewed_full_url: QString,
    /// The thumbnails' height, once the shell has one: the thumbnail URLs are made for it. None until then, and the rows
    /// show their thumbhashes.
    pub(crate) thumb_height: Option<pg_model::size::ThumbHeight>,
    pub(crate) viewed_info: QString,
    pub(crate) viewed_border_box: QRectF,
    pub(crate) viewed_rejected_top: f64,
    pub(crate) viewed_rejected_bottom: f64,
    pub(crate) viewed_rejected_left: f64,
    pub(crate) viewed_rejected_right: f64,
    pub(crate) viewed_border_marked: bool,
    /// `viewedFitError`: the viewer's item's fit error, as `refresh_viewed` sets it from `last_fit_error`.
    pub(crate) viewed_fit_error: i32,
    /// The last fit of a file that was made (`fit_outcome`): the file and its error code (0 when the fit was made). One
    /// slot, not a record per file, so it does not grow with the library: only the viewer's item's counts.
    pub(crate) last_fit_error: Option<(FileId, i32)>,
    /// The list, its rows and the list session (`pg_client::grid`).
    pub(crate) grid: Grid,
    /// The details read last, of at most two files (the selection's and the viewer's), newest first, each as the
    /// catalog had it (`None`: the file is gone). Read off the window's thread (`read_details`). Emptied whenever a
    /// list is shown or a file changes.
    pub(crate) details: Vec<(FileId, Option<Details>)>,
    /// Counts the emptyings of `details`: a read asked for before the last one is dropped when it comes in.
    pub(crate) details_generation: u64,
    /// Where a details read's answer goes; set when the model is made. A read asked before it is set is not carried out.
    pub(crate) details_delivery: Option<DetailsDelivery>,
    /// The file whose details read is asked for and not yet delivered. The reader keeps only its newest details read, so
    /// a read asked for another file supersedes this one, and that file is asked for again when it is next wanted.
    pub(crate) details_pending: Cell<Option<FileId>>,
    /// The border marks written off the window's thread and not yet shown, per file: the marks the writes still to be
    /// made set, in the order they were asked for (`set_border_marked`).
    pub(crate) mark_writes: BTreeMap<FileId, VecDeque<bool>>,
    /// The turns pressed on each file that has a press in flight or not yet counted by a read (`TurnCount`). A press
    /// is counted at once, and a read counts the presses its catalog counter reaches (`settle_turns`); a file's count is
    /// dropped once a read counts all of them and none is in flight. So the map holds only those files, and does not
    /// grow with the library.
    pub(crate) turn_counts: BTreeMap<FileId, TurnCount>,
}

impl Default for GalleryRust {
    fn default() -> Self {
        GalleryRust {
            count: 0,
            revision: 0,
            croppable: 0,
            loading: false,
            holding: false,
            listing_failed: false,
            failed_generation: None,
            current: -1,
            selection_info: QString::default(),
            viewing: -1,
            anchor: -1,
            viewed_key: QString::default(),
            viewed_neighbour_keys: QStringList::default(),
            viewed_turns: 0,
            viewed_rating: 0,
            viewed_full_width: 0,
            viewed_full_height: 0,
            viewed_kind: MediaType::Image,
            viewed_player_turns: 0,
            viewed_player_mirrored: false,
            viewed_file_id: 0,
            viewed_full_url: QString::default(),
            thumb_height: None,
            viewed_info: QString::default(),
            viewed_border_box: QRectF::default(),
            viewed_rejected_top: -1.0,
            viewed_rejected_bottom: -1.0,
            viewed_rejected_left: -1.0,
            viewed_rejected_right: -1.0,
            viewed_border_marked: false,
            viewed_fit_error: 0,
            last_fit_error: None,
            grid: Grid::default(),
            details: Vec::new(),
            details_generation: 0,
            details_delivery: None,
            details_pending: Cell::new(None),
            mark_writes: BTreeMap::new(),
            turn_counts: BTreeMap::new(),
        }
    }
}

impl GalleryRust {
    /// The entry of an item as QML numbers it (`None` for -1, or past the list).
    pub(crate) fn entry(&self, item: i32) -> Option<&Entry> {
        crate::rows::row_of(item).and_then(|i| self.grid.item(i))
    }

    /// The file of an item as QML numbers it (`None` for -1, or past the list).
    pub(crate) fn file_at(&self, item: i32) -> Option<FileId> {
        self.entry(item).map(|e| e.id)
    }

    /// The file the window shows: the viewer's while it is open, else the selection's (`None` for neither).
    pub(crate) fn shown_file(&self) -> Option<FileId> {
        self.file_at(if self.viewing >= 0 { self.viewing } else { self.current })
    }

    /// `f` of an item's entry and details, as the item's details stand. A read is asked for on a miss, and the model shows
    /// the details once they come in (`details_read`): until then they are `Loading`. `Unavailable` is an item with no
    /// file, or a file that is gone from the catalog or can't be read.
    pub(crate) fn with_details<R>(&self, item: i32, f: impl FnOnce(&Entry, &Details) -> R) -> Detail<R> {
        let Some(e) = self.entry(item) else { return Detail::Unavailable };
        match self.details.iter().find(|(id, _)| *id == e.id) {
            Some((_, Some(details))) => Detail::Loaded(f(e, details)),
            Some((_, None)) => Detail::Unavailable,
            None => {
                if self.details_pending.get() != Some(e.id) {
                    self.read_details(e.id);
                }
                Detail::Loading
            }
        }
    }

    /// Asks the reader for a file's details. The answer is delivered to the model, which keeps it (`details_read`)
    /// unless the details were emptied since the request.
    fn read_details(&self, file: FileId) {
        let generation = self.details_generation;
        let Some(deliver) = self.details_delivery.clone() else {
            pg_log::warn("gallery", "a file's details were asked for before the grid model was made; not read");
            return;
        };
        self.details_pending.set(Some(file));
        crate::app::session().details(file, move |read| {
            // No details when the read fails (the notice says why) or the file is gone.
            let details = crate::app::observe(Op::RowRead, read, |err| Notice::RowReadFailed {
                error: crate::app::error_text(err),
            })
            .flatten();
            deliver(generation, file, details);
        });
    }

    /// A file changed, or a list is shown: what was read of it may be stale.
    pub(crate) fn forget_details(&mut self) {
        self.details.clear();
        self.details_generation += 1;
        self.details_pending.set(None);
    }

    /// The mark an item's borders will have once the border marks asked for are written: the last one asked for, else
    /// the mark its borders have now (`borders`).
    pub(crate) fn mark_target(&self, file: FileId, borders: Borders) -> bool {
        self.mark_writes.get(&file).and_then(|queue| queue.back().copied()).unwrap_or(borders == Borders::Marked)
    }

    /// A border mark is asked for: its write is queued (`mark_target` then reads it).
    pub(crate) fn queue_mark(&mut self, file: FileId, marked: bool) {
        self.mark_writes.entry(file).or_default().push_back(marked);
    }

    /// A queued border mark's write is done, written or not: the first one asked for of the file is no longer waiting.
    pub(crate) fn mark_done(&mut self, file: FileId) {
        let empty = match self.mark_writes.get_mut(&file) {
            Some(queue) => {
                queue.pop_front();
                queue.is_empty()
            }
            None => false,
        };
        if empty {
            self.mark_writes.remove(&file);
        }
    }

    /// The entry the grid shows for `file`, as it stands (before a list or a refresh is put in).
    pub(crate) fn shown_entry(&self, file: FileId) -> Option<&Entry> {
        self.grid.position_of(file).and_then(|at| self.grid.item(at))
    }

    /// A press of `turns` on `file` is made: it is counted at once, and shown from now (`TurnCount::press`). A file with
    /// no count yet starts at the catalog's counter the shown entry was read at.
    pub(crate) fn press(&mut self, file: FileId, turns: Turns) {
        let read = self.shown_entry(file).map_or(0, |e| e.turns_queued);
        self.turn_counts.entry(file).or_insert_with(|| TurnCount::new(read)).press(turns);
    }

    /// The press on `file` is committed by its write (`TurnCount::written`). The read that reaches its counter counts it.
    pub(crate) fn turn_written(&mut self, file: FileId) {
        if let Some(count) = self.turn_counts.get_mut(&file) {
            count.written();
        }
        self.drop_settled(file);
    }

    /// The press on `file` is refused by its write, so it is taken back (`TurnCount::refused`).
    pub(crate) fn turn_refused(&mut self, file: FileId, turns: Turns) {
        if let Some(count) = self.turn_counts.get_mut(&file) {
            count.refused(turns);
        }
        self.drop_settled(file);
    }

    /// The entries a read brings, with their pending turns made to count the presses the read does not reach
    /// (`TurnCount::pending`). Each file's count takes the read's counter (`TurnCount::read`), and is dropped once a read
    /// counts all of its presses.
    ///
    /// An entry older than the one `shown` gives for its file (its counter below the shown one's: a read asked for
    /// earlier that came in later) is replaced by the one shown.
    pub(crate) fn settle_turns(
        &mut self,
        mut entries: Vec<Entry>,
        shown: impl Fn(FileId) -> Option<Entry>,
    ) -> Vec<Entry> {
        for entry in &mut entries {
            if let Some(prior) = shown(entry.id)
                && prior.turns_queued > entry.turns_queued
            {
                *entry = prior;
                continue;
            }
            if let Some(count) = self.turn_counts.get_mut(&entry.id) {
                count.read(entry.turns_queued);
                let pending = count.pending(entry);
                entry.pending_turns = pending;
            }
            self.drop_settled(entry.id);
        }
        entries
    }

    /// Drops `file`'s count once a read counts all of its presses and none is in flight (`TurnCount::settled`).
    fn drop_settled(&mut self, file: FileId) {
        if self.turn_counts.get(&file).is_some_and(TurnCount::settled) {
            self.turn_counts.remove(&file);
        }
    }
}

/// A file's details as the model shows them (`GalleryRust::with_details`): read and in the catalog, not yet read, or
/// not to be had. Kept apart so that a value shown for a file says which of the three it is.
pub(crate) enum Detail<R> {
    /// Asked for from the reader, not yet delivered.
    Loading,
    /// The file has no details to show: it is gone from the catalog, or its read failed (the inbox says why).
    Unavailable,
    /// The file's details, read.
    Loaded(R),
}

impl<R> Detail<R> {
    /// The value, when the details are read.
    pub(crate) fn loaded(self) -> Option<R> {
        match self {
            Detail::Loaded(value) => Some(value),
            Detail::Loading | Detail::Unavailable => None,
        }
    }
}

/// A file's entry as the catalog has it now; `None` when it is gone or can't be read (the inbox says why).
pub(crate) fn fresh_entry(id: FileId) -> Option<Entry> {
    crate::app::observe(Op::Reload, crate::app::get().reading().entry(id), |e| Notice::ReloadFailed {
        error: crate::app::error_text(e),
    })
    .flatten()
}
