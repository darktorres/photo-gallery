//! The state of the `FolderPicker` QObject: the bridge's backing struct.

use cxx_qt_lib::{QString, QStringList};

#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct PickerRust {
    pub(crate) native: bool,
    pub(crate) names: QStringList,
    pub(crate) here: QString,
}
