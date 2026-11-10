// The modal panels' kinds, as the shell takes them (Shell.panelOpened, panelClosed, panelOpenRequested): their codes are
// these positions, in the order of pg_client's PanelKind (a Rust test, keymap.rs, checks the order). Each panel reports
// its opening and its closing, so the window's shortcuts know which panel has the keys.
import QtQml

QtObject {
    enum Kind { Inbox, Log, Library, SaveViewDialog, ViewsMenu, FolderPicker, Yandex, YandexPreview }
}
