//! The state of the `FacetChoices` QObject: the bridge's backing struct, and the rows it shows.

/// One row: its key (the value the bar sends back, `key_of`), its label, its count (0 where none is shown), and whether
/// it's chosen.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    pub(super) key: String,
    pub(super) label: String,
    pub(super) count: i32,
    pub(super) chosen: bool,
}

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct FacetChoicesRust {
    /// The facet: the list's index in `pg_client::facets::ALL`; 0 (the default) is Types.
    pub(crate) facet: i32,
    pub(super) rows: Vec<Row>,
}
