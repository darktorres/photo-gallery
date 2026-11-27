//! The state of the `FacetChoices` QObject: the bridge's backing struct, and the rows it shows.

use pg_model::query::ListFacet;

/// One row: its key (the value the bar sends back, `key_of`), its label, its count (0 where none is shown or none is read
/// yet), whether it's chosen, and whether its count is pending (the folder's counts are being read: `count` is not one).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) count: i32,
    pub(crate) chosen: bool,
    pub(crate) pending: bool,
}

pub struct FacetChoicesRust {
    /// The list the rows are (the model's `ListFacet`; QML's `ChoiceFacet` is converted at the bridge); Types until QML
    /// sets it.
    pub(crate) facet: ListFacet,
    pub(crate) rows: Vec<Row>,
}

impl Default for FacetChoicesRust {
    fn default() -> Self {
        Self { facet: ListFacet::Types, rows: Vec::new() }
    }
}
