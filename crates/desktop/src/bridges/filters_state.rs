//! The state of the `Filters` QObject: the bridge's backing struct, which the filter bar's properties are kept in.

use cxx_qt_lib::QString;
use pg_model::facets::Facets;

pub struct FiltersRust {
    pub(crate) active: i32,
    pub(crate) name: QString,
    pub(crate) duplicates_index: i32,
    pub(crate) border_marked_index: i32,
    pub(crate) total: i32,
    pub(crate) matching: i32,
    pub(crate) description: QString,
    pub(crate) facets: Facets,
    /// The latest counts asked for; older ones arriving are dropped.
    pub(crate) generation: u64,
    /// The filter bar is open (the window reports it): the counts are kept while it is.
    pub(crate) bar_open: bool,
}
