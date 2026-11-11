//! What the viewer's details panel and the border overlay show of one file: its path, size and borders, read when
//! the file is shown (`pg_catalog::Catalog::details`).

use crate::detection;
use crate::rules;

/// What a file's grid entry leaves out, for the file shown (the selection, the viewer's).
#[derive(Clone, PartialEq, Debug)]
pub struct Details {
    /// The file's path, exact (the viewer's details show it as text, `PathRules::display`).
    pub path: rules::Path,
    pub size: i64,
    /// Whether the file can hold its rating too (`false`: kept in the catalog only). Unknown before the import.
    pub rating_in_file: bool,
    /// Every side the border detector found, accepted and rejected (`borders::detect`).
    pub border_sides: detection::Sides,
}
