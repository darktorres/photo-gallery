//! The state of the `FolderPicker` QObject: the bridge's backing struct.

use cxx_qt_lib::{QString, QStringList};

pub struct PickerRust {
    pub(super) native: bool,
    pub(super) names: QStringList,
    pub(super) here: QString,
}
