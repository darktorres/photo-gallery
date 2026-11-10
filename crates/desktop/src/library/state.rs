//! The state of the `Library` QObject: the bridge's backing struct (the roots and the job's progress and messages).

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
    pub(crate) progress: f64,
    pub(crate) message: QString,
    pub(crate) message_warning: bool,
    pub(crate) writes: QString,
    pub(crate) write_failed: bool,
    /// The writer's count of dropped writes (`WriteStatus::dropped`) already shown: a note is shown when it grows.
    pub(super) dropped_seen: usize,
    /// Whether the note that the writer has stopped for good has been shown (it is shown once).
    pub(super) writer_stopped_shown: bool,
}
