// What a chosen folder is for, as the folder pickers take it (FolderPicker.pick, done, openInApp, failed): the codes are
// these positions, in the order of the enum `Purpose` in crates/desktop/src/picker/state.rs (its `code`).
pragma ComponentBehavior: Bound
import QtQml

QtObject {
    enum Purpose {
        AddFolder,
        SaveCrops,
        YandexSave
    }
}
