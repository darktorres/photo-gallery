//! Where the grid looks: the listing the session was left at (the folder shown, with or without its subfolders, the sort
//! and the filter bar's query), and the saved views (a listing with a name). The listing is remembered for the next start.

use serde::{Deserialize, Serialize};

use crate::query::{Query, Sort};
use crate::rules::Merged;

/// Where the grid looks: the folder shown (the empty folder for all folders), with its subfolders, in what order,
/// filtered by what.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Listing {
    /// The folder, merged; empty for all folders.
    pub folder: Merged,
    /// Whether the folder's subfolders are listed with it.
    pub recursive: bool,
    /// The order the rows are listed in.
    pub sort: Sort,
    /// The filter bar's query (`Query` as JSON in the catalog; empty when none).
    pub query: Query,
}

/// The listing a start with nothing saved shows: every folder, with their subfolders, in the default order, unfiltered.
impl Default for Listing {
    fn default() -> Listing {
        Listing { folder: Merged::default(), recursive: true, sort: Sort::default(), query: Query::default() }
    }
}

/// A saved view (a rating app's bookmarks): a listing with a name.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct SavedView {
    pub name: String,
    #[serde(flatten)]
    pub listing: Listing,
}
