//! The cxx-qt bridge of [`crate::folders`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::folders::state::FolderRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtCore/QAbstractListModel>);
        type QAbstractListModel;

        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        /// The tree has been read at least once.
        #[qproperty(bool, ready, READ, WRITE, NOTIFY, FINAL)]
        type FolderModel = super::FolderRust;
    }

    impl cxx_qt::Threading for FolderModel {}

    unsafe extern "RustQt" {
        /// The tree `reload` asked for is shown.
        #[qsignal]
        fn loaded(self: Pin<&mut FolderModel>);

        #[qinvokable]
        #[cxx_override]
        fn data(self: &FolderModel, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &FolderModel) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &FolderModel, parent: &QModelIndex) -> i32;

        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut FolderModel>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut FolderModel>);

        #[inherit]
        #[cxx_name = "beginInsertRows"]
        fn begin_insert_rows(self: Pin<&mut FolderModel>, parent: &QModelIndex, first: i32, last: i32);

        #[inherit]
        #[cxx_name = "endInsertRows"]
        fn end_insert_rows(self: Pin<&mut FolderModel>);

        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        fn begin_remove_rows(self: Pin<&mut FolderModel>, parent: &QModelIndex, first: i32, last: i32);

        #[inherit]
        #[cxx_name = "endRemoveRows"]
        fn end_remove_rows(self: Pin<&mut FolderModel>);

        #[inherit]
        #[cxx_name = "dataChanged"]
        fn data_changed(self: Pin<&mut FolderModel>, top_left: &QModelIndex, bottom_right: &QModelIndex);

        #[inherit]
        fn index(self: &FolderModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        /// Read the tree from the catalog (at the start, after a scan); it is shown when it arrives (`loaded`).
        #[qinvokable]
        fn reload(self: Pin<&mut FolderModel>);

        /// Expand or collapse a row.
        #[qinvokable]
        fn toggle(self: Pin<&mut FolderModel>, row: i32);

        /// The tree arrived (`loaded`): the row of the listed folder. The first arrival opens the folders above it (the
        /// start's reveal).
        #[qinvokable]
        #[cxx_name = "treeLoaded"]
        fn tree_loaded(self: Pin<&mut FolderModel>) -> i32;

        /// The row of the listed folder once the tree is in: `first` opens the folders above it (the start's reveal).
        /// The row, or -1 for a folder under one the user collapsed, or -2 for a folder the tree doesn't have.
        #[qinvokable]
        #[cxx_name = "revealListing"]
        fn reveal_listing(self: Pin<&mut FolderModel>, first: bool) -> i32;
    }
}
