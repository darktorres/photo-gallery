//! The state of the `Library` QObject: the bridge's backing struct (the roots, the job's progress and the writer's line).

use cxx_qt_lib::{QString, QStringList};
use pg_model::ids::RootId;

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct LibraryRust {
    pub(crate) roots: QStringList,
    /// The catalog's id of each folder in `roots`, row for row: what removing a row removes.
    pub(super) root_ids: Vec<RootId>,
    pub(crate) busy: bool,
    pub(crate) paused: bool,
    pub(crate) status: QString,
    /// One row per lane that works or is paused: its status (see `lanes` in the bridge), and its pause ("1" or "0").
    pub(crate) lanes: QStringList,
    pub(crate) lane_paused: QStringList,
    /// The lane number and pause of each row in `lanes`, row for row: what a toggle of that row names.
    pub(super) lane_rows: Vec<(usize, bool)>,
    pub(crate) progress: f64,
    /// The pass's failed count ("2 failed"; empty when none), the link to its files, and the pass it is of (a
    /// `FailedFiles` pass code; meaningless while `failed` is empty).
    pub(crate) failed: QString,
    pub(crate) failed_pass: i32,
    pub(crate) writes: QString,
    pub(crate) write_failed: bool,
}
