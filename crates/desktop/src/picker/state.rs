//! The state of the `FolderPicker` QObject: the bridge's backing struct, what a chosen folder is for, and the app's
//! dialog's listing.

use std::path::PathBuf;

use cxx_qt_lib::{QString, QStringList};

/// What a chosen folder is for. The code is what QML passes (`pick`, `done`, `openInApp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// A folder added to the library.
    AddFolder,
    /// The folder the cropped copies are saved into.
    SaveCrops,
    /// The folder the Yandex pictures are saved into.
    YandexSave,
}

impl Purpose {
    pub(super) fn code(self) -> i32 {
        match self {
            Purpose::AddFolder => 0,
            Purpose::SaveCrops => 1,
            Purpose::YandexSave => 2,
        }
    }

    pub(super) fn from_code(code: i32) -> Option<Purpose> {
        [Purpose::AddFolder, Purpose::SaveCrops, Purpose::YandexSave].into_iter().find(|p| p.code() == code)
    }
}

/// The app's dialog: the purpose, the folder it is in, and the folders in it, in order.
pub(super) struct Listing {
    pub(super) purpose: Purpose,
    pub(super) here: PathBuf,
    pub(super) folders: Vec<PathBuf>,
}

#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct PickerRust {
    pub(crate) native: bool,
    pub(crate) names: QStringList,
    pub(crate) here: QString,
    /// The app's dialog while it is open: this picker's own (each `FolderPicker` lists apart).
    pub(super) listing: Option<Listing>,
}
