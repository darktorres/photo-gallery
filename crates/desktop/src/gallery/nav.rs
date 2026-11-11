//! Navigation: an item's row, its neighbours, and whether it is an item of the list.

use super::qobject;
use crate::rows::{row_of, to_row};

impl qobject::GalleryModel {
    pub(crate) fn row_of(&self, item: i32) -> i32 {
        let Some(at) = row_of(item).filter(|&at| self.grid.item(at).is_some()) else { return -1 };
        if self.grid.rows().is_empty() {
            return -1;
        }
        to_row(Some(self.grid.row_index(at)))
    }

    /// Keyboard navigation: the item `dx` to the side, or the one above/below (`dy`) nearest horizontally.
    pub(super) fn neighbor(&self, item: i32, dx: i32, dy: i32) -> i32 {
        // An item before the first is the first.
        to_row(self.grid.placement().neighbor(row_of(item).unwrap_or(0), dx, dy))
    }

    /// Whether `item` is an item of the list (the selection can only be one of them).
    pub(crate) fn is_item(&self, item: i32) -> bool {
        (0..self.count).contains(&item)
    }

    /// The neighbour of the selection `dx` columns and `dy` rows away; the first item when none is selected.
    pub(crate) fn neighbor_of_selection(&self, dx: i32, dy: i32) -> i32 {
        self.neighbor(self.current.max(0), dx, dy)
    }
}
