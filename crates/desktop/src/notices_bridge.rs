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
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut Notices>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut Notices>);

        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        fn begin_remove_rows(self: Pin<&mut Notices>, parent: &QModelIndex, first: i32, last: i32);

        #[inherit]
        #[cxx_name = "endRemoveRows"]
        fn end_remove_rows(self: Pin<&mut Notices>);

        /// Follow the inbox: what it lists, and its toasts. Once; later calls do nothing.
        #[qinvokable]
        fn start(self: Pin<&mut Notices>);

        /// Mark every entry read (the inbox was opened).
        #[qinvokable]
        #[cxx_name = "markAllRead"]
        fn mark_all_read(self: Pin<&mut Notices>);

        /// Dismiss the entry at `row`.
        #[qinvokable]
        fn dismiss(self: Pin<&mut Notices>, row: i32);

        /// Dismiss every entry.
        #[qinvokable]
        #[cxx_name = "dismissAll"]
        fn dismiss_all(self: Pin<&mut Notices>);

        /// Put the entry at `row` on the clipboard: its title, its details, where it comes from and when.
        #[qinvokable]
        fn copy(self: &Notices, row: i32);

        /// Take the toast down.
        #[qinvokable]
        #[cxx_name = "dismissToast"]
        fn dismiss_toast(self: Pin<&mut Notices>);

        /// Look for what another process changed (the window was activated).
        #[qinvokable]
        fn poke(self: &Notices);
    }
}
