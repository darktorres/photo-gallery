//! The cxx-qt bridge of [`crate::yandex`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::yandex::state::YandexSearchRust;

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
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qproperty(bool, busy, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(QString, status, READ, WRITE, NOTIFY, FINAL)]
        /// The results are the model's rows: the other sizes first, then the similar images.
        #[qproperty(i32, other_count, cxx_name = "otherCount", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, similar_count, cxx_name = "similarCount", READ, WRITE, NOTIFY, FINAL)]
        /// Results picked to be saved, and how many of each group are (its header's count).
        #[qproperty(i32, picked_count, cxx_name = "pickedCount", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, other_picked, cxx_name = "otherPicked", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, similar_picked, cxx_name = "similarPicked", READ, WRITE, NOTIFY, FINAL)]
        /// The width the results are laid out in: the panel sets it to its list's width, and they are laid out again.
        #[qproperty(f64, layout_width, cxx_name = "layoutWidth", READ, WRITE = set_layout_width, NOTIFY, FINAL)]
        /// The space between the results: the gallery's gap, so the two agree.
        #[qproperty(f64, gap, READ, WRITE = set_gap, NOTIFY, FINAL)]
        /// The height of each group's block of frames: the other sizes', and the similar images'.
        #[qproperty(f64, other_height, cxx_name = "otherHeight", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(f64, similar_height, cxx_name = "similarHeight", READ, WRITE, NOTIFY, FINAL)]
        /// The result the preview shows (-1: none). Setting it refreshes the four properties below.
        #[qproperty(i32, preview_item, cxx_name = "previewItem", READ, WRITE = set_preview_item, NOTIFY, FINAL)]
        /// What the preview shows of its item: its picture's URL (`""` for none), its size in pixels as Yandex gives
        /// it (0 when it doesn't), and whether it is picked.
        #[qproperty(QString, preview_url, cxx_name = "previewUrl", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, preview_width, cxx_name = "previewWidth", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, preview_height, cxx_name = "previewHeight", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(bool, preview_picked, cxx_name = "previewPicked", READ, WRITE, NOTIFY, FINAL)]
        type YandexSearch = super::YandexSearchRust;
    }

    impl cxx_qt::Threading for YandexSearch {}

    unsafe extern "RustQt" {
        fn set_preview_item(self: Pin<&mut YandexSearch>, value: i32);

        fn set_layout_width(self: Pin<&mut YandexSearch>, value: f64);

        fn set_gap(self: Pin<&mut YandexSearch>, value: f64);

        /// Upload the catalog's file `file_id` (the photo) and read its other sizes and similar images.
        #[qinvokable]
        #[cxx_name = "searchFile"]
        fn search_file(self: Pin<&mut YandexSearch>, file_id: i64);

        /// Upload the photo the command line names (`--find-similar=<path>`), as `searchFile` does for a catalog file.
        #[qinvokable]
        #[cxx_name = "searchCommandLine"]
        fn search_command_line(self: Pin<&mut YandexSearch>);

        #[qinvokable]
        #[cxx_override]
        fn data(self: &YandexSearch, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &YandexSearch) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &YandexSearch, parent: &QModelIndex) -> i32;

        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut YandexSearch>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut YandexSearch>);

        #[inherit]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut YandexSearch>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );

        #[inherit]
        fn index(self: &YandexSearch, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        /// Pick a result to be saved, or unpick it. It becomes the anchor of the next `pickTo`.
        #[qinvokable]
        #[cxx_name = "togglePick"]
        fn toggle_pick(self: Pin<&mut YandexSearch>, item: i32);

        /// Shift-click: every result from the anchor (the last one toggled) to `item` takes the anchor's state, picked
        /// or not. Without an anchor, toggles `item`.
        #[qinvokable]
        #[cxx_name = "pickTo"]
        fn pick_to(self: Pin<&mut YandexSearch>, item: i32);

        /// Pick (or unpick) the `count` results from `first`: a group's Select all and Select none.
        #[qinvokable]
        #[cxx_name = "setPicked"]
        fn set_picked(self: Pin<&mut YandexSearch>, first: i32, count: i32, picked: bool);

        /// Show `text` as the status line: the save's folder dialog failed (the picker's `failed`), and says why.
        #[qinvokable]
        #[cxx_name = "showFailure"]
        fn show_failure(self: Pin<&mut YandexSearch>, text: &QString);

        /// Save the picked results into the folder the picker chose for them (`FolderPicker`, purpose 2).
        #[qinvokable]
        #[cxx_name = "downloadPicked"]
        fn download_picked(self: Pin<&mut YandexSearch>);
    }
}
