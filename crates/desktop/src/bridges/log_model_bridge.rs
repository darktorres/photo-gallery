//! The cxx-qt bridge of [`crate::log_model`]: the declarations alone, so that the allow the macro's output needs is
//! on the `bridges` module (`lib.rs`), not on anything written here.

use crate::log_model::state::LogRust;

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
        /// The records have been read (the files' and this run's): the view shows them from then on.
        #[qproperty(bool, ready, READ, NOTIFY, FINAL)]
        /// The selected row in the view (-1: none). The selection is held here, by the row's number, so it follows its
        /// record as the oldest rows go; the panel reads what it shows from these properties.
        #[qproperty(i32, selected_row, cxx_name = "selectedRow", READ, NOTIFY, FINAL)]
        /// The selected row's number (its `seq` role), -1 when none is selected.
        #[qproperty(f64, selected_seq, cxx_name = "selectedSeq", READ, NOTIFY, FINAL)]
        /// The selected record's text: its message, with its detail below it.
        #[qproperty(QString, selected_text, cxx_name = "selectedText", READ, NOTIFY, FINAL)]
        type LogModel = super::LogRust;
    }

    impl cxx_qt::Threading for LogModel {}

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &LogModel, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &LogModel) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &LogModel, parent: &QModelIndex) -> i32;

        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut LogModel>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut LogModel>);

        #[inherit]
        #[cxx_name = "beginInsertRows"]
        fn begin_insert_rows(self: Pin<&mut LogModel>, parent: &QModelIndex, first: i32, last: i32);

        #[inherit]
        #[cxx_name = "endInsertRows"]
        fn end_insert_rows(self: Pin<&mut LogModel>);

        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        fn begin_remove_rows(self: Pin<&mut LogModel>, parent: &QModelIndex, first: i32, last: i32);

        #[inherit]
        #[cxx_name = "endRemoveRows"]
        fn end_remove_rows(self: Pin<&mut LogModel>);

        /// Read the log and follow it: the records of the files and this run's, then each new one as it is logged. Once;
        /// later calls do nothing.
        #[qinvokable]
        fn start(self: Pin<&mut LogModel>);

        /// Select the row at `row` (-1 or a row past the list: none).
        #[qinvokable]
        fn select(self: Pin<&mut LogModel>, row: i32);

        /// Clear the selection.
        #[qinvokable]
        fn unselect(self: Pin<&mut LogModel>);
    }
}
