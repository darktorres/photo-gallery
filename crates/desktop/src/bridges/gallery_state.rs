//! The state of the `GalleryModel` QObject: the bridge's backing struct (the property values, the viewer's item's
//! values, the grid and the details read last), and what it reads of the catalog.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use cxx_qt_lib::{QRectF, QString, QStringList};
use pg_client::grid::Grid;
use pg_model::details::Details;
use pg_model::grid::{Borders, Entry};
use pg_model::ids::FileId;
use pg_model::media::MediaType;
use pg_model::notice::{Notice, Op};

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
    /// The border marks written off the window's thread and not yet shown, per file: the marks the writes still to be
    /// made set, in the order they were asked for (`set_border_marked`).
    pub(crate) mark_writes: BTreeMap<FileId, VecDeque<bool>>,
}

impl Default for GalleryRust {
    fn default() -> Self {
        GalleryRust {
            count: 0,
            revision: 0,
            croppable: 0,
            loading: false,
            holding: false,
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
            grid: Grid::default(),
            details: Vec::new(),
            details_generation: 0,
            details_delivery: None,
            mark_writes: BTreeMap::new(),
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

    /// `f` of an item's entry and details; `None` without the item, when its file is gone from the catalog, or while
    /// its details are being read: a read is asked for on a miss, and the model shows the details once they come in
    /// (`details_read`).
    pub(crate) fn with_details<R>(&self, item: i32, f: impl FnOnce(&Entry, &Details) -> R) -> Option<R> {
        let e = self.entry(item)?;
        match self.details.iter().find(|(id, _)| *id == e.id) {
            Some((_, Some(details))) => Some(f(e, details)),
            Some((_, None)) => None,
            None => {
                self.read_details(e.id);
                None
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
        crate::app::session().details(file, move |read| {
            // No details when the read fails (the notice says why) or the file is gone.
            let details = crate::app::observe(Op::RowRead, read, |err| Notice::RowReadFailed {
                error: crate::app::error_text(&format!("{err:#}")),
            })
            .flatten();
            deliver(generation, file, details);
        });
    }

    /// A file changed, or a list is shown: what was read of it may be stale.
    pub(crate) fn forget_details(&mut self) {
        self.details.clear();
        self.details_generation += 1;
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
}

/// A file's entry as the catalog has it now; `None` when it is gone or can't be read (the inbox says why).
pub(crate) fn fresh_entry(id: FileId) -> Option<Entry> {
    crate::app::observe(Op::Reload, crate::app::get().reading().entry(id), |e| Notice::ReloadFailed {
        error: crate::app::error_text(&format!("{e:#}")),
    })
    .flatten()
}
