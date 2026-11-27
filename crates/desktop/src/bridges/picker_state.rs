//! The state of the `FolderPicker` QObject: the bridge's backing struct, what a chosen folder is for, and the app's
//! dialog's listing.

use std::path::PathBuf;

use cxx_qt_lib::{QString, QStringList};

/// What a chosen folder is for. The code is what QML passes (`pick`, `done`, `openInApp`): the position of the value in
/// `PickerPurpose` (qml/PickerPurpose.qml), which must list these variants in this order.
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
    pub(crate) fn code(self) -> i32 {
        match self {
            Purpose::AddFolder => 0,
            Purpose::SaveCrops => 1,
            Purpose::YandexSave => 2,
        }
    }

    pub(crate) fn from_code(code: i32) -> Option<Purpose> {
        [Purpose::AddFolder, Purpose::SaveCrops, Purpose::YandexSave].into_iter().find(|p| p.code() == code)
    }
}

/// The app's dialog: the purpose, the folder it is in, and the folders in it, in order.
pub(crate) struct Listing {
    pub(crate) purpose: Purpose,
    pub(crate) here: PathBuf,
    pub(crate) folders: Vec<PathBuf>,
}

pub struct PickerRust {
    pub(crate) native: bool,
    pub(crate) names: QStringList,
    pub(crate) here: QString,
    /// Why the app's dialog could not list the folder it was asked to show (`listError`), shown inside it; empty when it
    /// lists.
    pub(crate) list_error: QString,
    /// The app's dialog while it is open: this picker's own (each `FolderPicker` lists apart).
    pub(crate) listing: Option<Listing>,
}

#[cfg(test)]
mod tests {
    use super::Purpose;

    /// `PickerPurpose` (qml/PickerPurpose.qml) lists the variants in the order of their codes, which QML passes.
    #[test]
    fn picker_purpose_lists_the_variants_in_code_order() {
        let qml = include_str!("../../qml/PickerPurpose.qml");
        let variants = [Purpose::AddFolder, Purpose::SaveCrops, Purpose::YandexSave];
        let mut at = Vec::new();
        for (code, purpose) in variants.into_iter().enumerate() {
            assert_eq!(purpose.code(), i32::try_from(code).unwrap());
            at.push(qml.find(&format!("{purpose:?}")).unwrap_or_else(|| panic!("{purpose:?} is not in PickerPurpose")));
        }
        assert!(at.windows(2).all(|w| w[0] < w[1]), "PickerPurpose's order is not the codes' order");
    }
}
