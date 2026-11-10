//! The state of the `Shell` QObject: the bridge's backing struct (the properties the shell mirrors from the controller).

use cxx_qt_lib::{QString, QStringList};

/// The properties the shell mirrors from the controller. Everything else is the controller's.
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct ShellRust {
    pub(crate) folder: QString,
    pub(crate) recursive: bool,
    pub(crate) folder_kind: i32,
    pub(crate) descending: bool,
    pub(crate) saved_views: QStringList,
    pub(crate) filters_open: bool,
    pub(crate) show_borders: bool,
    pub(crate) row_height: i32,
    pub(crate) thumb_height: i32,
}
