//! The state of the `Filters` QObject: the bridge's backing struct, which the filter bar's properties are kept in.

use cxx_qt_lib::QString;
use pg_app::reexport::Facets;

pub(super) struct FiltersRust {
    pub(super) active: i32,
    pub(super) name: QString,
    pub(super) duplicates_index: i32,
    pub(super) border_marked_index: i32,
    pub(super) total: i32,
    pub(super) matching: i32,
    pub(super) description: QString,
    pub(super) facets: Facets,
    /// The latest counts asked for; older ones arriving are dropped.
    pub(super) generation: u64,
}
