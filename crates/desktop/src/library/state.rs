//! The state of the `Library` QObject: the bridge's backing struct (the roots, the job's progress and the writer's line).

use cxx_qt_lib::{QString, QStringList};
use pg_model::ids::RootId;
use pg_model::notice::Notice;

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
    pub(crate) progress: f64,
    pub(crate) writes: QString,
    pub(crate) write_failed: bool,
    /// The writer's count of dropped writes (`WriteStatus::dropped`) already told: a notice is posted when it grows.
    pub(super) dropped_seen: usize,
    /// Whether the notice that the writer has stopped for good has been posted (it is posted once).
    pub(super) writer_stopped_shown: bool,
    /// The writer's failure the inbox was last told of: the same one again is not posted.
    pub(super) write_failure: Option<Notice>,
}
