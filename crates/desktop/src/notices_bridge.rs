//! The cxx-qt bridge of [`crate::notices`]: the declarations alone, so that the allow the macro's output needs is on
//! this module and on nothing written by hand (`lib.rs`).

use crate::notices::state::NoticesRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtCore/QAbstractListModel>);
        type QAbstractListModel;

        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
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
        /// How many entries are unread: the bell's badge.
        #[qproperty(i32, unread, READ, NOTIFY, FINAL)]
        /// The ledgers the inbox lists something about, a bit each (bit `pass`: 0 the thumbnails, 1 the borders, 2 the
        /// import, 3 the scans' folders): a ledger's own entry, or a pass's unreadable files. The status bar links a
        /// pass's "N failed" only while its bit is set (a binding to a call would not follow it).
        #[qproperty(i32, passes, READ, NOTIFY, FINAL)]
        /// The toast's words (a notice's title), empty when no toast is up.
        #[qproperty(QString, toast, READ, NOTIFY, FINAL)]
        /// The toast's level: 0 info (it goes by itself), 1 warning, 2 error (they stay until dismissed or replaced).
        #[qproperty(i32, toast_level, cxx_name = "toastLevel", READ, NOTIFY, FINAL)]
        type Notices = super::NoticesRust;
    }

    impl cxx_qt::Threading for Notices {}

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &Notices, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &Notices) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &Notices, parent: &QModelIndex) -> i32;

        #[inherit]
        fn index(self: &Notices, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut Notices>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut Notices>);

        #[inherit]
        #[cxx_name = "beginInsertRows"]
        fn begin_insert_rows(self: Pin<&mut Notices>, parent: &QModelIndex, first: i32, last: i32);

        #[inherit]
        #[cxx_name = "endInsertRows"]
        fn end_insert_rows(self: Pin<&mut Notices>);

        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        fn begin_remove_rows(self: Pin<&mut Notices>, parent: &QModelIndex, first: i32, last: i32);

        #[inherit]
        #[cxx_name = "endRemoveRows"]
        fn end_remove_rows(self: Pin<&mut Notices>);

        #[inherit]
        #[cxx_name = "beginMoveRows"]
        fn begin_move_rows(
            self: Pin<&mut Notices>,
            source_parent: &QModelIndex,
            source_first: i32,
            source_last: i32,
            destination_parent: &QModelIndex,
            destination_child: i32,
        ) -> bool;

        #[inherit]
        #[cxx_name = "endMoveRows"]
        fn end_move_rows(self: Pin<&mut Notices>);

        #[inherit]
        #[cxx_name = "dataChanged"]
        fn data_changed(self: Pin<&mut Notices>, top_left: &QModelIndex, bottom_right: &QModelIndex, roles: &QList_i32);

        /// Follow the inbox: what it lists, and its toasts. Once; later calls do nothing.
        #[qinvokable]
        fn start(self: Pin<&mut Notices>);

        /// The inbox panel is `open` or shut, in a window `active` or not: while it is open the ledgers' counts are read;
        /// while it is open in an active window, what is shown is marked read as it is shown.
        #[qinvokable]
        #[cxx_name = "setViewing"]
        fn set_viewing(self: Pin<&mut Notices>, open: bool, active: bool);

        /// Dismiss the entry at `row`, as it was displayed.
        #[qinvokable]
        fn dismiss(self: Pin<&mut Notices>, row: i32);

        /// Dismiss the entries shown ("clear all"), as they were displayed.
        #[qinvokable]
        #[cxx_name = "dismissAll"]
        fn dismiss_all(self: Pin<&mut Notices>);

        /// Put the entry at `row` on the clipboard: its words, and a ledger's files or folders (read off the window's
        /// thread first).
        #[qinvokable]
        fn copy(self: &Notices, row: i32);

        /// Take the toast down.
        #[qinvokable]
        #[cxx_name = "dismissToast"]
        fn dismiss_toast(self: Pin<&mut Notices>);

        /// Look for what another process changed (the window was activated).
        #[qinvokable]
        fn poke(self: &Notices);

        /// The row of the entry with this `key` (the `key` role), -1 when none is listed.
        #[qinvokable]
        #[cxx_name = "rowOfKey"]
        fn row_of_key(self: &Notices, key: &QString) -> i32;

        /// The key of a ledger's entry (a `pass` code: 0 the thumbnails, 1 the borders, 2 the import, 3 the scans'
        /// folders), "" for none.
        #[qinvokable]
        #[cxx_name = "passKey"]
        fn pass_key(self: &Notices, pass: i32) -> QString;

        /// The `pass` code of a ledger's code (`thumbs`, `borders`, `import`, `scan-folders`), -1 for none.
        #[qinvokable]
        #[cxx_name = "passOfCode"]
        fn pass_of_code(self: &Notices, code: &QString) -> i32;

        /// Try the failed files of the entry at `row` again (a ledger's entry whose `canRetry`).
        #[qinvokable]
        fn retry(self: &Notices, row: i32);
    }
}
