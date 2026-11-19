//! The cxx-qt bridge of [`crate::library`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::library::state::LibraryRust;

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
        #[qproperty(QStringList, roots, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(bool, busy, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(bool, paused, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(QString, status, READ, WRITE, NOTIFY, FINAL)]
        /// Done ÷ total of the current pass; 0 when the total isn't known (`indeterminate` says so).
        #[qproperty(f64, progress, READ, WRITE, NOTIFY, FINAL)]
        /// The current pass's total isn't known (scanning): its bar is indeterminate while it runs.
        #[qproperty(bool, indeterminate, READ, WRITE, NOTIFY, FINAL)]
        /// How many files the current pass failed on ("2 failed"), empty when none: the link to the inbox's entry of its
        /// failed files.
        #[qproperty(QString, failed, READ, WRITE, NOTIFY, FINAL)]
        /// The pass `failed` is of: 0 the thumbnails, 1 the borders, 2 the import, -1 one without a failure ledger.
        #[qproperty(i32, failed_pass, cxx_name = "failedPass", READ, WRITE, NOTIFY, FINAL)]
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

        /// Scan every library folder again (new, changed, moved and deleted files).
        #[qinvokable]
        fn rescan(self: Pin<&mut Library>);

        #[qinvokable]
        #[cxx_name = "togglePause"]
        fn toggle_pause(self: Pin<&mut Library>);

        /// Pauses (`paused`) or resumes the lane numbered `number` (a row's `number`), leaving the other drives running.
        #[qinvokable]
        #[cxx_name = "setLanePaused"]
        fn set_lane_paused(self: Pin<&mut Library>, number: i32, paused: bool);
    }
}
