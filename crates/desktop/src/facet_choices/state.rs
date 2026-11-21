//! The state of the `FacetChoices` QObject: the bridge's backing struct, and the rows it shows.

use pg_model::query::ListFacet;

/// One row: its key (the value the bar sends back, `key_of`), its label, its count (0 where none is shown), and whether
/// it's chosen.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    pub(super) key: String,
    pub(super) label: String,
    pub(super) count: i32,
    pub(super) chosen: bool,
}

#[expect(
    unreachable_pub,
    reason = "the bridge's generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct FacetChoicesRust {
    /// The list the rows are (the model's `ListFacet`; QML's `ChoiceFacet` is converted at the bridge); Types until QML
    /// sets it.
    pub(crate) facet: ListFacet,
    pub(super) rows: Vec<Row>,
}

impl Default for FacetChoicesRust {
    fn default() -> Self {
        Self { facet: ListFacet::Types, rows: Vec::new() }
    }
}
