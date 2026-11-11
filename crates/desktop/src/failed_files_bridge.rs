//! The cxx-qt bridge of [`crate::failed_files`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::failed_files::state::FailedFilesRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtCore/QAbstractListModel>);
        type QAbstractListModel;

        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        /// How many rows the ledger shown holds (the list shows the newest of them), as its read counted them.
        #[qproperty(i32, total, READ, NOTIFY, FINAL)]
        /// The list asked for is being read (off the window's thread): the list says so until it is delivered.
        #[qproperty(bool, loading, READ, NOTIFY, FINAL)]
        /// Why the list couldn't be read, worded; empty when it was (or is being read).
        #[qproperty(QString, error, READ, NOTIFY, FINAL)]
        type FailedFiles = super::FailedFilesRust;
    }

    impl cxx_qt::Threading for FailedFiles {}

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &FailedFiles, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &FailedFiles) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &FailedFiles, parent: &QModelIndex) -> i32;

        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut FailedFiles>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut FailedFiles>);

        /// Show what a ledger lists (a `Notices` row's `pass`: 0 the thumbnails, 1 the borders, 2 the import, 3 the scans'
        /// folders): asked for now, shown when its read is delivered (`loading` meanwhile); -1 shows none. Asked again
        /// for the ledger shown, it is read again (its newest failure changed) and its rows stay until then.
        #[qinvokable]
        fn show(self: Pin<&mut FailedFiles>, pass: i32);
    }
}
