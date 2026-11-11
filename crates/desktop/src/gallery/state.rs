//! The state of the `GalleryModel` QObject: the bridge's backing struct (the property values, the viewer's item's
//! values, the grid and the details read last), and what it reads of the catalog.

use std::cell::RefCell;

use cxx_qt_lib::{QRectF, QString, QStringList};
use pg_client::grid::Grid;
use pg_model::details::Details;
use pg_model::grid::Entry;
use pg_model::ids::FileId;
use pg_model::media::MediaType;
use pg_model::notice::Notice;

/// The model's properties, the list and the details read last.
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
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
    pub(super) viewed_kind: MediaType,
    pub(crate) viewed_player_turns: i32,
    pub(crate) viewed_file_id: i64,
    pub(crate) viewed_full_url: QString,
    pub(crate) thumb_height: i32,
    pub(crate) viewed_info: QString,
    pub(crate) viewed_border_box: QRectF,
    pub(crate) viewed_rejected_top: f64,
    pub(crate) viewed_rejected_bottom: f64,
    pub(crate) viewed_rejected_left: f64,
    pub(crate) viewed_rejected_right: f64,
    pub(crate) viewed_border_marked: bool,
    /// The list, its rows and the list session (`pg_client::grid`).
    pub(super) grid: Grid,
    /// The details read last, of at most two files (the selection's and the viewer's), newest first. Emptied
    /// whenever a list is shown or a file changes.
    pub(super) details: RefCell<Vec<(FileId, Details)>>,
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
            thumb_height: 0,
            viewed_info: QString::default(),
            viewed_border_box: QRectF::default(),
            viewed_rejected_top: -1.0,
            viewed_rejected_bottom: -1.0,
            viewed_rejected_left: -1.0,
            viewed_rejected_right: -1.0,
            viewed_border_marked: false,
            grid: Grid::default(),
            details: RefCell::new(Vec::new()),
        }
    }
}

impl GalleryRust {
    /// The entry of an item as QML numbers it (`None` for -1, or past the list).
    pub(super) fn entry(&self, item: i32) -> Option<&Entry> {
        crate::rows::row_of(item).and_then(|i| self.entry(i))
    }

    /// The file of an item as QML numbers it (`None` for -1, or past the list).
    pub(super) fn file_at(&self, item: i32) -> Option<FileId> {
        self.entry(item).map(|e| e.id)
    }

    /// `f` of an item's entry and details; `None` without the item, or when its file is gone from the catalog.
    pub(super) fn with_details<R>(&self, item: i32, f: impl FnOnce(&Entry, &Details) -> R) -> Option<R> {
        let e = self.entry(item)?;
        let mut memo = self.details.borrow_mut();
        if let Some(at) = memo.iter().position(|(id, _)| *id == e.id) {
            let found = memo.remove(at);
            memo.insert(0, found);
        } else {
            let details = match crate::app::get().reading().details(e.id) {
                Ok(d) => {
                    crate::app::resolved("row-read-failed");
                    d?
                }
                Err(err) => {
                    crate::app::post(Notice::RowReadFailed { error: crate::app::error_text(&format!("{err:#}")) });
                    return None;
                }
            };
            memo.truncate(1);
            memo.insert(0, (e.id, details));
        }
        memo.first().map(|(_, details)| f(e, details))
    }

    /// A file changed, or a list is shown: what was read of it may be stale.
    pub(super) fn forget_details(&mut self) {
        self.details.get_mut().clear();
    }
}

/// A file's entry as the catalog has it now; `None` when it is gone or can't be read (the inbox says why).
pub(super) fn fresh_entry(id: FileId) -> Option<Entry> {
    match crate::app::get().reading().entry(id) {
        Ok(found) => {
            crate::app::resolved("reload-failed");
            found
        }
        Err(e) => {
            crate::app::post(Notice::ReloadFailed { error: crate::app::error_text(&format!("{e:#}")) });
            None
        }
    }
}
