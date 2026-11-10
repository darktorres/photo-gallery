//! The state of the `Shell` QObject: the bridge's backing struct (the properties the shell mirrors from the controller).

use cxx_qt_lib::{QString, QStringList};

/// The properties the shell mirrors from the controller. Everything else is the controller's.
pub struct ShellRust {
    pub(super) folder: QString,
    pub(super) recursive: bool,
    pub(super) folder_kind: i32,
    pub(super) descending: bool,
    pub(super) saved_views: QStringList,
    pub(super) filters_open: bool,
    pub(super) show_borders: bool,
    pub(super) row_height: i32,
    pub(super) thumb_height: i32,
}
