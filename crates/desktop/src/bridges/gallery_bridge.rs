//! The cxx-qt bridge of [`crate::gallery`]: the declarations alone, so that the allow the macro's output needs is
//! on the `bridges` module (`lib.rs`), not on anything written here.

use crate::bridges::gallery_state::GalleryRust;

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
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qrectf.h");
        type QRectF = cxx_qt_lib::QRectF;
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
    }

    /// What the viewer shows of its item (`viewedKind`): a still, an animation or a video. QML compares it by name
    /// (`GalleryModel.Video`); the key map reads the int QML passes by these values (`keymap.rs`).
    #[qenum(GalleryModel)]
    enum MediaKind {
        Image,
        Animated,
        Video,
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        /// How many files the list holds: its row count.
        #[qproperty(i32, count, READ, WRITE, NOTIFY, FINAL)]
        /// How many of the listed files have borders found and not marked: what `saveCroppedCopies` crops. Read again
        /// with the revision (`bump_revision`).
        #[qproperty(i32, croppable, READ, WRITE, NOTIFY, FINAL)]
        /// Counts the changes to the rows and their state (`bump_revision`): a reader holding an older value knows its
        /// data is stale.
        #[qproperty(i32, revision, READ, WRITE, NOTIFY, FINAL)]
        /// A list asked for (`openList`) hasn't arrived yet; the one shown stays meanwhile.
        #[qproperty(bool, loading, READ, WRITE, NOTIFY, FINAL)]
        /// The list asked for has arrived and waits under the viewer (`Grid::is_held`): it is loaded, not shown, so
        /// it is not "loading" to the user. QML reads `loading && !holding`.
        #[qproperty(bool, holding, READ, WRITE, NOTIFY, FINAL)]
        /// The selected item (-1: none): it follows its file to a new list. Set through `set_current_item`, which
        /// reads the selection's text again.
        #[qproperty(i32, current, READ, WRITE = set_current_item, NOTIFY, FINAL)]
        /// The status bar's text for the selected item: its info, and its rating's symbol after it. Empty with none.
        /// Read again with the selection and the revision.
        #[qproperty(QString, selection_info, cxx_name = "selectionInfo", READ, WRITE, NOTIFY, FINAL)]
        /// The viewer's item (-1: closed). While it is open, a list that arrives is held, and shown when it closes.
        #[qproperty(i32, viewing, READ, WRITE = set_viewing_holding, NOTIFY, FINAL)]
        /// Without a selection, the item a new list keeps in view (the top one): set in `aboutToLoad`.
        #[qproperty(i32, anchor, READ, WRITE, NOTIFY, FINAL)]
        /// What the viewer shows of its item (`viewing`): set by `refresh_viewed` whenever the item or what it shows
        /// changes, and bound to by the viewer, which asks for none of them one by one.
        #[qproperty(QString, viewed_key, cxx_name = "viewedKey", READ, WRITE, NOTIFY, FINAL)]
        /// The keys of the items around the viewer's item, as `refresh_viewed` sets them.
        #[qproperty(QStringList, viewed_neighbour_keys, cxx_name = "viewedNeighbourKeys", READ, WRITE, NOTIFY, FINAL)]
        /// The viewer's item's quarter turns, as `refresh_viewed` sets them.
        #[qproperty(i32, viewed_turns, cxx_name = "viewedTurns", READ, WRITE, NOTIFY, FINAL)]
        /// The viewer's item's rating, as `refresh_viewed` sets it.
        #[qproperty(i32, viewed_rating, cxx_name = "viewedRating", READ, WRITE, NOTIFY, FINAL)]
        /// The viewer's item's full-size width in pixels.
        #[qproperty(i32, viewed_full_width, cxx_name = "viewedFullWidth", READ, WRITE, NOTIFY, FINAL)]
        /// The viewer's item's full-size height in pixels.
        #[qproperty(i32, viewed_full_height, cxx_name = "viewedFullHeight", READ, WRITE, NOTIFY, FINAL)]
        /// The viewer's item's media kind: an image, an animation or a video.
        #[qproperty(MediaKind, viewed_kind, cxx_name = "viewedKind", READ = read_viewed_kind, NOTIFY, FINAL)]
        /// The quarter turns Qt's player adds to show the viewer's file upright: its catalog rotation and the turns not
        /// yet written.
        #[qproperty(i32, viewed_player_turns, cxx_name = "viewedPlayerTurns", READ, WRITE, NOTIFY, FINAL)]
        /// The viewer's item's file id in the catalog.
        #[qproperty(i64, viewed_file_id, cxx_name = "viewedFileId", READ, WRITE, NOTIFY, FINAL)]
        /// The viewer's item's URL at full size.
        #[qproperty(QString, viewed_full_url, cxx_name = "viewedFullUrl", READ, WRITE, NOTIFY, FINAL)]
        /// The viewer's item's status-bar line.
        #[qproperty(QString, viewed_info, cxx_name = "viewedInfo", READ, WRITE, NOTIFY, FINAL)]
        /// What is left of the viewer's item without its borders, as fractions of the image as shown.
        #[qproperty(QRectF, viewed_border_box, cxx_name = "viewedBorderBox", READ, WRITE, NOTIFY, FINAL)]
        /// Where the border detector's first rejected candidate at the top would have cut, as a fraction of the image from
        /// that edge; -1 for none.
        #[qproperty(f64, viewed_rejected_top, cxx_name = "viewedRejectedTop", READ, WRITE, NOTIFY, FINAL)]
        /// As `viewedRejectedTop`, at the bottom edge.
        #[qproperty(f64, viewed_rejected_bottom, cxx_name = "viewedRejectedBottom", READ, WRITE, NOTIFY, FINAL)]
        /// As `viewedRejectedTop`, at the left edge.
        #[qproperty(f64, viewed_rejected_left, cxx_name = "viewedRejectedLeft", READ, WRITE, NOTIFY, FINAL)]
        /// As `viewedRejectedTop`, at the right edge.
        #[qproperty(f64, viewed_rejected_right, cxx_name = "viewedRejectedRight", READ, WRITE, NOTIFY, FINAL)]
        /// Whether the viewer's item's borders are marked as not a border (the review).
        #[qproperty(bool, viewed_border_marked, cxx_name = "viewedBorderMarked", READ, WRITE, NOTIFY, FINAL)]
        type GalleryModel = super::GalleryRust;
    }

    impl cxx_qt::Threading for GalleryModel {}

    unsafe extern "RustQt" {
        fn set_viewing_holding(self: Pin<&mut GalleryModel>, value: i32);
        fn set_current_item(self: Pin<&mut GalleryModel>, value: i32);
        fn read_viewed_kind(self: &GalleryModel) -> MediaKind;

        /// A new list is about to replace the shown one: the last moment to set `anchor`.
        #[qsignal]
        #[cxx_name = "aboutToLoad"]
        fn about_to_load(self: Pin<&mut GalleryModel>);

        /// The list asked for is shown. `index`: where `anchor`'s file is now (-1: not kept, or gone).
        #[qsignal]
        fn loaded(self: Pin<&mut GalleryModel>, index: i32);

        #[qinvokable]
        #[cxx_override]
        fn data(self: &GalleryModel, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &GalleryModel) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &GalleryModel, parent: &QModelIndex) -> i32;

        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut GalleryModel>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut GalleryModel>);

        #[inherit]
        #[cxx_name = "beginInsertRows"]
        fn begin_insert_rows(self: Pin<&mut GalleryModel>, parent: &QModelIndex, first: i32, last: i32);

        #[inherit]
        #[cxx_name = "endInsertRows"]
        fn end_insert_rows(self: Pin<&mut GalleryModel>);

        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        fn begin_remove_rows(self: Pin<&mut GalleryModel>, parent: &QModelIndex, first: i32, last: i32);

        #[inherit]
        #[cxx_name = "endRemoveRows"]
        fn end_remove_rows(self: Pin<&mut GalleryModel>);

        #[inherit]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut GalleryModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );

        #[inherit]
        fn index(self: &GalleryModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        /// Recompute the rows for a viewport width and a target row height (the runtime thumbnail size). Returns the item
        /// to keep in view: the current item, else `topItem` (the view's top item, read before the change), or -1.
        #[qinvokable]
        fn relayout(self: Pin<&mut GalleryModel>, width: f64, row_height: f64, spacing: f64, top_item: i32) -> i32;

        /// The list is shown: where the view goes (`placeRequested`), given the item the list was anchored to (-1: none).
        #[qinvokable]
        #[cxx_name = "placeList"]
        fn place_list(self: Pin<&mut GalleryModel>, anchor: i32);

        /// The view goes to `item` (-1: the top of the list): contained in the view when `contain`, else at its top.
        #[qsignal]
        #[cxx_name = "placeRequested"]
        fn place_requested(self: Pin<&mut GalleryModel>, item: i32, contain: bool);

        #[qinvokable]
        #[cxx_name = "fileId"]
        fn file_id(self: &GalleryModel, item: i32) -> i64;

        /// The item showing a file, or -1.
        #[qinvokable]
        #[cxx_name = "indexOf"]
        fn index_of(self: &GalleryModel, file_id: i64) -> i32;

        /// Save cropped copies of the listed files whose borders were found (and not marked) into the folder the picker
        /// chose for them (`FolderPicker`, `PickerPurpose.SaveCrops`), in the background (the library's crop pass). Returns how many it
        /// asked for.
        #[qinvokable]
        #[cxx_name = "saveCroppedCopiesPicked"]
        fn save_cropped_copies_picked(self: &GalleryModel) -> i32;

        /// How many files the cut dialog says its borders come off (the listed files with borders found).
        #[qinvokable]
        #[cxx_name = "cutBordersCount"]
        fn cut_borders_count(self: &GalleryModel) -> i32;

        /// The user confirmed cutting the borders off the originals (CutBordersDialog): the library rewrites those files in
        /// place, in the background. Returns how many it asked for.
        #[qinvokable]
        #[cxx_name = "cutBordersConfirmed"]
        fn cut_borders_confirmed(self: &GalleryModel) -> i32;

        #[qinvokable]
        #[cxx_name = "rowOf"]
        fn row_of(self: &GalleryModel, item: i32) -> i32;
    }

    // The model's constructor: Qt makes it, and `Initialize` (gallery.rs) registers it as the window's model. Declared
    // here, as cxx-qt requires for a custom constructor.
    impl cxx_qt::Constructor<()> for GalleryModel {}
}
