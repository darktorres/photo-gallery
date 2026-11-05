//! The sort box (the top bar's): its choices in order, and the sort each one is. Name reads A→Z; the others start with the
//! largest, newest or longest, so they are descending. The box shows the key, whichever way it sorts.

use pg_model::query::{Sort, SortKey};

/// The sort box's keys, in its order (the names are TopBar's `sortNames`).
pub const KEYS: [SortKey; 7] = [
    SortKey::Name,
    SortKey::Size,
    SortKey::Modified,
    SortKey::Taken,
    SortKey::Megapixels,
    SortKey::Duration,
    SortKey::Borders,
];

/// The sort the box's choice `index` makes: its key, descending except for the first (name). `None` out of range.
pub fn sort_choice(index: i32) -> Option<Sort> {
    let index = usize::try_from(index).ok()?;
    let key = *KEYS.get(index)?;
    Some(Sort { key, descending: index != 0 })
}

/// The box's index for a sort's key.
pub fn choice_index(key: SortKey) -> i32 {
    KEYS.iter().position(|&k| k == key).map_or(0, |i| i as i32)
}
