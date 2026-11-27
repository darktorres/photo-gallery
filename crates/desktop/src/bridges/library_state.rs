//! The state of the `Library` QObject: the bridge's backing struct (the roots, the job's progress and the writer's line).

use cxx_qt_lib::{QString, QStringList};
use pg_model::ids::RootId;
use pg_model::rules::Path as RulePath;

#[derive(Default)]
pub struct LibraryRust {
    pub(crate) roots: QStringList,
    pub(crate) kept_removals: QStringList,
    pub(crate) removing_removals: QStringList,
    /// The catalog's id of each folder in `roots`, row for row: what removing a row removes.
    pub(crate) root_ids: Vec<RootId>,
    /// The folder of each row, as the catalog has it: what the removal is asked for (`Catalog::begin_removal` checks it).
    pub(crate) root_paths: Vec<RulePath>,
    /// The exact path of each folder in `kept_removals`, row for row: what Discard is asked for by its row. The list's text
    /// is lossy, so two removals can show the same text; the path is not.
    pub(crate) kept_removal_paths: Vec<RulePath>,
    pub(crate) busy: bool,
    /// A scan was asked for and its job has not shown its progress, or ended, yet (see `scanPending`).
    pub(crate) scan_pending: bool,
    pub(crate) writes: QString,
    pub(crate) write_failed: bool,
    /// Whether this object has started the library (`start`). The library itself lives on past a window made again (live
    /// reload), so the window's own object keys its start on this, not on the library being running.
    pub(crate) started: bool,
}
