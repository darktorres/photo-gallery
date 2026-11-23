//! The cxx-qt bridge of [`crate::library`]: the declarations alone, so that the allow the macro's output needs is
//! on the `bridges` module (`lib.rs`), not on anything written here.

use crate::bridges::library_state::LibraryRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// The library's folders as paths, one row each; `load_roots` fills it, and the catalog ids run alongside.
        #[qproperty(QStringList, roots, READ, WRITE, NOTIFY, FINAL)]
        /// The folders whose removal is not finished and waits for an unfinished write (Discard gives it up).
        #[qproperty(QStringList, kept_removals, cxx_name = "keptRemovals", READ, WRITE, NOTIFY, FINAL)]
        /// The folders whose removal is not finished and is still going.
        #[qproperty(QStringList, removing_removals, cxx_name = "removingRemovals", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(bool, busy, READ, WRITE, NOTIFY, FINAL)]
        /// Ratings and turns still to be written into the files, and the latest failure; empty when none.
        #[qproperty(QString, writes, cxx_name = "writeStatus", READ, WRITE, NOTIFY, FINAL)]
        /// Whether the latest write failed (the status says why).
        #[qproperty(bool, write_failed, cxx_name = "writeFailed", READ, WRITE, NOTIFY, FINAL)]
        type Library = super::LibraryRust;
    }

    impl cxx_qt::Threading for Library {}

    unsafe extern "RustQt" {
        #[qsignal]
        fn scanned(self: Pin<&mut Library>);

        #[qsignal]
        fn imported(self: Pin<&mut Library>);

        /// The job's progress was read again (a pass, a pause): the lane rows (`LaneModel`) refresh from it.
        #[qsignal]
        fn progressed(self: Pin<&mut Library>);

        /// Files were turned (rewritten or turned in the catalog): their images and sizes changed.
        #[qsignal]
        #[cxx_name = "filesChanged"]
        fn files_changed(self: Pin<&mut Library>);

        /// Start the background job and the writer; they first finish whatever is pending.
        #[qinvokable]
        fn start(self: Pin<&mut Library>);

        /// Add the folder the picker chose for adding a folder (`FolderPicker`, purpose 0). The choice is taken here, not
        /// passed through QML.
        #[qinvokable]
        #[cxx_name = "addPicked"]
        fn add_picked(self: Pin<&mut Library>);

        /// Remove the library folder at `row` (its row in `roots`).
        #[qinvokable]
        #[cxx_name = "removeFolder"]
        fn remove_folder(self: Pin<&mut Library>, row: i32);

        /// The folder at `row` of `roots`, as the list shows it (`""` for none): the removal's dialog names it.
        #[qinvokable]
        #[cxx_name = "folderAt"]
        fn folder_at(self: &Library, row: i32) -> QString;

        /// Give up the removal of the folder `folder` (as `keptRemovals` lists it) that waits for unfinished writes: the
        /// changes are dropped.
        #[qinvokable]
        #[cxx_name = "discardRemoval"]
        fn discard_removal(self: Pin<&mut Library>, folder: QString);

        /// Scan every library folder again (new, changed, moved and deleted files).
        #[qinvokable]
        fn rescan(self: Pin<&mut Library>);

        /// Pauses (`paused`) or resumes the lane numbered `number` (a row's `number`), leaving the other drives running.
        #[qinvokable]
        #[cxx_name = "setLanePaused"]
        fn set_lane_paused(self: Pin<&mut Library>, number: i32, paused: bool);
    }
}
