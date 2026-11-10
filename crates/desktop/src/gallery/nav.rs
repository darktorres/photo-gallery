//! Navigation: an item's row, its neighbours, and whether it is an item of the list.

use super::qobject;

impl qobject::GalleryModel {
    pub(super) fn row_of(&self, item: i32) -> i32 {
        let Ok(at) = usize::try_from(item) else { return -1 };
        if self.grid.item(item).is_none() || self.grid.rows().is_empty() {
            return -1;
        }
        i32::try_from(self.grid.row_index(at)).unwrap_or(-1)
    }

    /// Keyboard navigation: the item `dx` to the side, or the one above/below (`dy`) nearest horizontally.
    pub(super) fn neighbor(&self, item: i32, dx: i32, dy: i32) -> i32 {
        self.grid.placement().neighbor(item, dx, dy)
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
