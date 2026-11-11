//! The sort box (the top bar's): its choices in order, and the sort each one is. Name and file type read A→Z; the others
//! start with the largest, newest, best or longest, so they are descending. The box shows the key, whichever way it sorts.

use pg_model::query::{Sort, SortKey};

/// The sort box's keys, in its order (the names are TopBar's `sortNames`).
pub const KEYS: [SortKey; 14] = [
    SortKey::Name,
    SortKey::Extension,
    SortKey::Size,
    SortKey::Modified,
    SortKey::Taken,
    SortKey::Created,
    SortKey::Rating,
    SortKey::Megapixels,
    SortKey::Width,
    SortKey::Height,
    SortKey::AspectRatio,
    SortKey::Duration,
    SortKey::Frames,
    SortKey::Borders,
];

/// The sort the box's choice `index` makes: its key, descending except for name and file type. `None` out of range.
pub fn sort_choice(index: usize) -> Option<Sort> {
    let key = *KEYS.get(index)?;
    Some(Sort { key, descending: !matches!(key, SortKey::Name | SortKey::Extension) })
}

/// The box's index for a sort's key.
pub fn choice_index(key: SortKey) -> usize {
    KEYS.iter().position(|&k| k == key).unwrap_or(0)
}
