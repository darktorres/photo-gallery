//! The state of the `Filters` QObject: the bridge's backing struct, which the filter bar's properties are kept in.

use cxx_qt_lib::QString;
use pg_app::reexport::Facets;

#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct FiltersRust {
    pub(crate) active: i32,
    pub(crate) name: QString,
    pub(crate) duplicates_index: i32,
    pub(crate) border_marked_index: i32,
    pub(crate) total: i32,
    pub(crate) matching: i32,
    pub(crate) description: QString,
    pub(super) facets: Facets,
    /// The latest counts asked for; older ones arriving are dropped.
    pub(super) generation: u64,
}
