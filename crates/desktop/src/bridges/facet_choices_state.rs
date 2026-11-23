//! The state of the `FacetChoices` QObject: the bridge's backing struct, and the rows it shows.

use pg_model::query::ListFacet;

/// One row: its key (the value the bar sends back, `key_of`), its label, its count (0 where none is shown), and whether
/// it's chosen.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) count: i32,
    pub(crate) chosen: bool,
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
