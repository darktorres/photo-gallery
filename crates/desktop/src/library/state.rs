//! The state of the `Library` QObject: the bridge's backing struct (the roots, the job's progress and the writer's line).

use cxx_qt_lib::{QString, QStringList};
use pg_model::ids::RootId;
use pg_model::rules::Path as RulePath;

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge's generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct LibraryRust {
    pub(crate) roots: QStringList,
    pub(crate) kept_removals: QStringList,
    pub(crate) removing_removals: QStringList,
    /// The catalog's id of each folder in `roots`, row for row: what removing a row removes.
    pub(super) root_ids: Vec<RootId>,
    /// The folder of each row, as the catalog has it: what the removal is asked for (`Catalog::begin_removal` checks it).
    pub(super) root_paths: Vec<RulePath>,
    pub(crate) busy: bool,
    pub(crate) writes: QString,
    pub(crate) write_failed: bool,
}
