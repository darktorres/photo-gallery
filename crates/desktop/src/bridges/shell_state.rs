//! The state of the `Shell` QObject: the bridge's backing struct (the properties the shell mirrors from the controller).

use cxx_qt_lib::{QString, QStringList};

/// The properties the shell mirrors from the controller. Everything else is the controller's.
pub struct ShellRust {
    pub(crate) folder: QString,
    pub(crate) recursive: bool,
    pub(crate) folder_kind: i32,
    pub(crate) descending: bool,
    pub(crate) saved_views: QStringList,
    pub(crate) filters_open: bool,
    pub(crate) show_borders: bool,
    pub(crate) row_height: i32,
    /// The row heights the grid takes: the size controls' bounds (`pg_model::size::ROW_HEIGHT`).
    pub(crate) min_row_height: i32,
    pub(crate) max_row_height: i32,
    pub(crate) dev_mode: bool,
    /// Whether a Reveal's folder is being read (`revealing`).
    pub(crate) revealing: bool,
    /// The message of the last Reveal that could not show its file (`revealError`): empty when none.
    pub(crate) reveal_error: QString,
}
