//! The cxx-qt bridge of [`crate::picker`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::picker::state::PickerRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        include!("pg-desktop/cpp/picker.h");
        /// The app's window's native handle (its HWND on Windows), 0 when none is shown: the folder dialog's owner.
        #[namespace = "pg"]
        fn main_window_handle() -> usize;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// Whether the system's dialog is used (Windows). Otherwise `openInApp` asks QML to show the app's own dialog.
        #[qproperty(bool, native, READ, FINAL)]
        /// The app's dialog: the folders of the folder it is in (display names, in order).
        #[qproperty(QStringList, names, READ, WRITE, NOTIFY, FINAL)]
        /// The app's dialog: the folder it is in (display text).
        #[qproperty(QString, here, READ, WRITE, NOTIFY, FINAL)]
        type FolderPicker = super::PickerRust;
    }

    unsafe extern "RustQt" {
        /// A folder was chosen for `purpose` (see `Purpose`): the action takes it.
        #[qsignal]
        fn done(self: Pin<&mut FolderPicker>, purpose: i32);

        /// The app's own dialog should open for `purpose` (not on Windows, where the system's does).
        #[qsignal]
        #[cxx_name = "openInApp"]
        fn open_in_app(self: Pin<&mut FolderPicker>, purpose: i32);

        /// A folder dialog couldn't open, or couldn't give or list a folder, for `purpose`: `message` says why, and
        /// nothing was chosen.
        #[qsignal]
        fn failed(self: Pin<&mut FolderPicker>, purpose: i32, message: QString);

        /// Pick a folder for `purpose`: the system's dialog, or the app's (`openInApp`).
        #[qinvokable]
        fn pick(self: Pin<&mut FolderPicker>, purpose: i32);

        /// The folder from the command line for `purpose` (`--add-folder=…` for 0, `--save-crops=…` for 1), chosen as the
        /// dialog would choose it (`done`), with no dialog.
        #[qinvokable]
        #[cxx_name = "chooseCommandLine"]
        fn choose_command_line(self: Pin<&mut FolderPicker>, purpose: i32);

        /// The app's dialog: open the listed folder at `row`.
        #[qinvokable]
        fn enter(self: Pin<&mut FolderPicker>, row: i32);

        /// The app's dialog: the folder above the one it is in.
        #[qinvokable]
        fn up(self: Pin<&mut FolderPicker>);

        /// The app's dialog: choose the listed folder at `row`.
        #[qinvokable]
        fn choose(self: Pin<&mut FolderPicker>, row: i32);

        /// The app's dialog: choose the folder it is in.
        #[qinvokable]
        #[cxx_name = "chooseHere"]
        fn choose_here(self: Pin<&mut FolderPicker>);

        /// The app's dialog: closed without a choice.
        #[qinvokable]
        fn cancel(self: Pin<&mut FolderPicker>);
    }
}
