//! The state of the `Library` QObject: the bridge's backing struct (the roots and the job's progress and messages).

use cxx_qt_lib::{QString, QStringList};
use pg_model::ids::RootId;

#[derive(Default)]
pub struct LibraryRust {
    pub(super) roots: QStringList,
    /// The catalog's id of each folder in `roots`, row for row: what removing a row removes.
    pub(super) root_ids: Vec<RootId>,
    pub(super) busy: bool,
    pub(super) paused: bool,
    pub(super) status: QString,
    pub(super) progress: f64,
    pub(super) message: QString,
    pub(super) message_warning: bool,
    pub(super) writes: QString,
    pub(super) write_failed: bool,
    /// The writer's count of dropped writes (`WriteStatus::dropped`) already shown: a note is shown when it grows.
    pub(super) dropped_seen: usize,
    /// Whether the note that the writer has stopped for good has been shown (it is shown once).
    pub(super) writer_stopped_shown: bool,
}
