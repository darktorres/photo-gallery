//! Where the grid looks: the place the session was left at (the folder shown, with or without its subfolders, the sort
//! and the filter bar's query), and the saved views (a place with a name). The place is remembered for the next start.

use crate::query::{Query, Sort};

/// Where the user was: the folder shown (`""` for all folders), with its subfolders, in what order, filtered by what.
#[derive(Clone, Debug)]
pub struct Place {
    /// Merged path; `""` for all folders.
    pub folder: String,
    pub recursive: bool,
    pub sort: Sort,
    /// The filter bar's query (`Query` as JSON in the catalog; empty when none).
    pub query: Query,
}

/// A saved view (ImageRater's bookmarks): where, which files, in what order.
#[derive(Clone, PartialEq, Debug)]
pub struct SavedView {
    pub name: String,
    /// Merged path; `""` for all folders.
    pub folder: String,
    pub recursive: bool,
    pub query: Query,
    pub sort: Sort,
}
