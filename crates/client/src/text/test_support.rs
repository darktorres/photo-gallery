//! What the tests of the words and of the messages share.

use super::words::ENGLISH;

/// The ids of the entries in the English file: the lines `id = ...` that start a message (not a comment, not a
/// variant's line of a select expression, which is indented).
pub(super) fn entries_in_file() -> Vec<&'static str> {
    entries_of(ENGLISH)
}

/// The ids of the entries in the Fluent source `source`, as [`entries_in_file`] reads them.
pub(super) fn entries_of(source: &'static str) -> Vec<&'static str> {
    source
        .lines()
        .filter(|line| !line.starts_with(' ') && !line.starts_with('#'))
        .filter_map(|line| line.split_once(" =").map(|(id, _)| id))
        .collect()
}
