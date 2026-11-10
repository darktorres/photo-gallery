//! The state of the `GalleryModel` QObject: the bridge's backing struct (the property values, the viewer's item's
//! values, the grid and the details read last).

use std::cell::RefCell;

use cxx_qt_lib::{QRectF, QString, QStringList};
use pg_client::grid::Grid;
use pg_model::details::Details;
use pg_model::ids::FileId;
use pg_model::query::MediaType;

/// The model's properties, the list and the details read last.
pub struct GalleryRust {
    pub(super) count: i32,
    pub(super) revision: i32,
    pub(super) croppable: i32,
    pub(super) loading: bool,
    pub(super) holding: bool,
    pub(super) current: i32,
    pub(super) selection_info: QString,
    pub(super) viewing: i32,
    pub(super) anchor: i32,
    // The viewer's item's values (`refresh_viewed`): the qproperties' backing fields. Before any refresh they are the
    // values of no item, which `refresh_viewed` gives with `viewing` out of range.
    pub(super) viewed_key: QString,
    pub(super) viewed_neighbour_keys: QStringList,
    pub(super) viewed_turns: i32,
    pub(super) viewed_rating: i32,
    pub(super) viewed_full_width: i32,
    pub(super) viewed_full_height: i32,
    /// `viewedKind`'s value, as the domain's type: the bridge's `MediaKind` is read from it (`read_viewed_kind`).
    pub(super) viewed_kind: MediaType,
    pub(super) viewed_player_turns: i32,
    pub(super) viewed_file_id: i64,
    pub(super) viewed_full_url: QString,
    pub(super) thumb_height: i32,
    pub(super) viewed_info: QString,
    pub(super) viewed_border_box: QRectF,
    pub(super) viewed_rejected_top: f64,
    pub(super) viewed_rejected_bottom: f64,
    pub(super) viewed_rejected_left: f64,
    pub(super) viewed_rejected_right: f64,
    pub(super) viewed_border_marked: bool,
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
