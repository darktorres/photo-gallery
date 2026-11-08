//! The grid model: the files of the open folder, laid out in justified rows (`pg_client::layout`). QML sees one
//! list row per layout row (virtualized by ListView) and asks for each item's file id and size by index. The
//! model holds a compact entry per file (`pg_model::grid::Entry`); a file's path and size are read when it is shown
//! (`App::details`), for the selection and the viewer's file.
//!
//! The files are listed off the GUI thread (the app's catalog reader): the controller's `OpenList` command asks (the
//! listing says what), and the list arrives later (`loaded`).
//! So that nothing is renumbered under the user, the model keeps the items QML holds: the selection (`current`)
//! moves with its file when a new list arrives, and while the viewer is open (`viewing`) an arriving list waits
//! for it to close. A rating or a turn made while a list was on its way is read again into it.

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
    /// (`GalleryModel.Video`); the key map takes it as its value, the position here (`keymap.rs`).
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
        #[qproperty(i32, count, READ, WRITE, NOTIFY, FINAL)]
        /// How many of the listed files have borders found and not marked: what `saveCroppedCopies` crops. Read again
        /// with the revision (`bump_revision`).
        #[qproperty(i32, croppable, READ, WRITE, NOTIFY, FINAL)]
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
        #[qproperty(QStringList, viewed_neighbour_keys, cxx_name = "viewedNeighbourKeys", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, viewed_turns, cxx_name = "viewedTurns", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, viewed_rating, cxx_name = "viewedRating", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, viewed_full_width, cxx_name = "viewedFullWidth", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, viewed_full_height, cxx_name = "viewedFullHeight", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(MediaKind, viewed_kind, cxx_name = "viewedKind", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, viewed_player_turns, cxx_name = "viewedPlayerTurns", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i64, viewed_file_id, cxx_name = "viewedFileId", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(QString, viewed_full_url, cxx_name = "viewedFullUrl", READ, WRITE, NOTIFY, FINAL)]
        /// The thumbnails' height in device pixels (QML's, from the screen's scale): the thumbnail URLs are made for it,
        /// so a change makes every row read them again (`setThumbHeight`).
        #[qproperty(i32, thumb_height, cxx_name = "thumbHeight", READ, WRITE = set_thumb_height, FINAL)]
        #[qproperty(QString, viewed_info, cxx_name = "viewedInfo", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(QRectF, viewed_border_box, cxx_name = "viewedBorderBox", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(f64, viewed_rejected_top, cxx_name = "viewedRejectedTop", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(f64, viewed_rejected_bottom, cxx_name = "viewedRejectedBottom", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(f64, viewed_rejected_left, cxx_name = "viewedRejectedLeft", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(f64, viewed_rejected_right, cxx_name = "viewedRejectedRight", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(bool, viewed_border_marked, cxx_name = "viewedBorderMarked", READ, WRITE, NOTIFY, FINAL)]
        type GalleryModel = super::GalleryRust;
    }

    impl cxx_qt::Threading for GalleryModel {}

    unsafe extern "RustQt" {
        fn set_viewing_holding(self: Pin<&mut GalleryModel>, value: i32);
        fn set_current_item(self: Pin<&mut GalleryModel>, value: i32);

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

        /// Make this model the one the window's controller carries its model commands out on (`post`). Called once,
        /// when the window is made, before the controller's first list.
        #[qinvokable]
        #[cxx_name = "registerModel"]
        fn register_model(self: Pin<&mut GalleryModel>);

        /// Recompute the rows for a viewport width and a target row height (the runtime thumbnail size).
        #[qinvokable]
        fn relayout(self: Pin<&mut GalleryModel>, width: f64, row_height: f64, spacing: f64);

        #[qinvokable]
        #[cxx_name = "fileId"]
        fn file_id(self: &GalleryModel, item: i32) -> i64;

        /// The item showing a file, or -1.
        #[qinvokable]
        #[cxx_name = "indexOf"]
        fn index_of(self: &GalleryModel, file_id: i64) -> i32;

        /// The thumbnails' height changed: every row's thumbnail URLs are read again (`ROLE_THUMB_URLS`).
        fn set_thumb_height(self: Pin<&mut GalleryModel>, height: i32);

        /// Save cropped copies of the listed files whose borders were found (and not marked) into the folder the picker
        /// chose for them (`FolderPicker`, purpose 1), in the background (the library's crop pass). Returns how many it
        /// asked for.
        #[qinvokable]
        #[cxx_name = "saveCroppedCopiesPicked"]
        fn save_cropped_copies_picked(self: &GalleryModel) -> i32;

        #[qinvokable]
        #[cxx_name = "rowOf"]
        fn row_of(self: &GalleryModel, item: i32) -> i32;
    }
}

use core::pin::Pin;
use std::cell::RefCell;
use std::io::Write;

use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QPointF, QRectF, QString, QStringList, QVariant,
};
use pg_client::grid::{Arrival, Changed, Grid, Plan, Shape};
use pg_client::layout;
use pg_client::text::{self, FoundBorders};
use pg_model::details::Details;
use pg_model::detection::Class;
use pg_model::grid::{Borders, Entry};
use pg_model::ids::FileId;
use pg_model::image::{ImageKey, ImageRequest, Version};
use pg_model::query::MediaType;
use pg_model::rating::Rating;
use pg_model::rules::PathRules;
use pg_model::turns::Turns;

/// A row's items, (first, count) as a point: one value, so a row changed or reused is given its items at once.
const ROLE_SPAN: i32 = 256;
const ROLE_HEIGHT: i32 = 257;
/// A row's version: a new one, never given before, whenever the items the row holds change without its span
/// changing (rated, turned, marked, rewritten, a new list). Its cells read their items again on it, not on every
/// change anywhere (`revision`, which the viewer and the toolbar follow); and a delegate the view reuses for a row
/// knows from it whether its cells show that row as it is.
const ROLE_VERSION: i32 = 258;
/// The values a row's cells show, one role per field: a list with an entry per item of the row, in order. They are
/// typed lists (`QStringList`, `QList<i32>`, `QList<bool>`, `QList<f64>`), which QML's compiler reads without `var`
/// (`GridRow`). Each comes with `ROLE_SPAN` and `ROLE_VERSION`, and before them (`show`, `items_changed`). A cell
/// needs no image key of its own: its thumbnail URLs carry it, and the viewer reads its item's (`viewedKey`).
const ROLE_BADGES: i32 = 260;
const ROLE_RATINGS: i32 = 261;
const ROLE_BORDERS_MARKED: i32 = 262;
const ROLE_IMAGE_ASPECTS: i32 = 263;
const ROLE_ASPECTS: i32 = 264;
const ROLE_CONTENT_X: i32 = 265;
const ROLE_CONTENT_Y: i32 = 266;
const ROLE_CONTENT_WIDTH: i32 = 267;
const ROLE_CONTENT_HEIGHT: i32 = 268;
const ROLE_THUMB_URLS: i32 = 269;
const ROLE_THUMBHASH_URLS: i32 = 270;
/// The cells' roles, and their names in QML.
const CELL_ROLES: [(i32, &str); 11] = [
    (ROLE_THUMB_URLS, "thumbUrls"),
    (ROLE_THUMBHASH_URLS, "thumbhashUrls"),
    (ROLE_BADGES, "badges"),
    (ROLE_RATINGS, "ratings"),
    (ROLE_BORDERS_MARKED, "bordersMarked"),
    (ROLE_IMAGE_ASPECTS, "imageAspects"),
    (ROLE_ASPECTS, "aspects"),
    (ROLE_CONTENT_X, "contentX"),
    (ROLE_CONTENT_Y, "contentY"),
    (ROLE_CONTENT_WIDTH, "contentWidth"),
    (ROLE_CONTENT_HEIGHT, "contentHeight"),
];

thread_local! {
    /// The model's thread handle, kept at `register`. Only the GUI thread touches it.
    static MODEL: RefCell<Option<CxxQtThread<qobject::GalleryModel>>> = const { RefCell::new(None) };
}

/// Carry out `run` on the grid model, after the events already queued to it: the controller's model commands (rate,
/// turn, list, border marks) come through here. Nothing runs before `register` (the window is not made yet).
pub(crate) fn post(run: impl FnOnce(Pin<&mut qobject::GalleryModel>) + Send + 'static) {
    MODEL.with(|model| {
        if let Some(thread) = model.borrow().as_ref() {
            let _ = thread.queue(run);
        }
    });
}

pub struct GalleryRust {
    count: i32,
    revision: i32,
    croppable: i32,
    loading: bool,
    holding: bool,
    current: i32,
    selection_info: QString,
    viewing: i32,
    anchor: i32,
    // The viewer's item's values (`refresh_viewed`): the qproperties' backing fields. Before any refresh they are the
    // values of no item, which `refresh_viewed` gives with `viewing` out of range.
    viewed_key: QString,
    viewed_neighbour_keys: QStringList,
    viewed_turns: i32,
    viewed_rating: i32,
    viewed_full_width: i32,
    viewed_full_height: i32,
    viewed_kind: qobject::MediaKind,
    viewed_player_turns: i32,
    viewed_file_id: i64,
    viewed_full_url: QString,
    thumb_height: i32,
    viewed_info: QString,
    viewed_border_box: QRectF,
    viewed_rejected_top: f64,
    viewed_rejected_bottom: f64,
    viewed_rejected_left: f64,
    viewed_rejected_right: f64,
    viewed_border_marked: bool,
    /// The list, its rows and the list session (`pg_client::grid`).
    grid: Grid,
    /// The details read last, of at most two files (the selection's and the viewer's), newest first. Emptied
    /// whenever a list is shown or a file changes.
    details: RefCell<Vec<(FileId, Details)>>,
}

impl Default for GalleryRust {
    fn default() -> Self {
        GalleryRust {
            count: 0,
            revision: 0,
            croppable: 0,
            loading: false,
            holding: false,
            current: -1,
            selection_info: QString::default(),
            viewing: -1,
            anchor: -1,
            viewed_key: QString::default(),
            viewed_neighbour_keys: QStringList::default(),
            viewed_turns: 0,
            viewed_rating: 0,
            viewed_full_width: 0,
            viewed_full_height: 0,
            viewed_kind: qobject::MediaKind::Image,
            viewed_player_turns: 0,
            viewed_file_id: 0,
            viewed_full_url: QString::default(),
            thumb_height: 0,
            viewed_info: QString::default(),
            viewed_border_box: QRectF::default(),
            viewed_rejected_top: -1.0,
            viewed_rejected_bottom: -1.0,
            viewed_rejected_left: -1.0,
            viewed_rejected_right: -1.0,
            viewed_border_marked: false,
            grid: Grid::default(),
            details: RefCell::new(Vec::new()),
        }
    }
}

impl GalleryRust {
    /// `f` of an item's entry and details; `None` without the item, or when its file is gone from the catalog.
    fn with_details<R>(&self, item: i32, f: impl FnOnce(&Entry, &Details) -> R) -> Option<R> {
        let e = self.grid.item(item)?;
        let mut memo = self.details.borrow_mut();
        if let Some(at) = memo.iter().position(|(id, _)| *id == e.id) {
            let found = memo.remove(at);
            memo.insert(0, found);
        } else {
            let details = match crate::app::get().details(e.id) {
                Ok(d) => d?,
                Err(err) => {
                    crate::panic_log::note(&format!("reading {}: {err:#}", e.id.get()));
                    return None;
                }
            };
            memo.truncate(1);
            memo.insert(0, (e.id, details));
        }
        memo.first().map(|(_, details)| f(e, details))
    }

    /// A file changed, or a list is shown: what was read of it may be stale.
    fn forget_details(&mut self) {
        self.details.get_mut().clear();
    }
}

impl qobject::GalleryModel {
    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Ok(at) = usize::try_from(index.row()) else { return QVariant::default() };
        let Some(row) = self.grid.rows().get(at) else { return QVariant::default() };
        // The row's items as Qt's ints, which the item getters take.
        let (Ok(first), Ok(end)) = (i32::try_from(row.first), i32::try_from(row.first + row.count)) else {
            return QVariant::default();
        };
        let items = first..end;
        match role {
            ROLE_SPAN => {
                QVariant::from(&QPointF::new(pg_model::num::count(row.first), pg_model::num::count(row.count)))
            }
            ROLE_HEIGHT => QVariant::from(&row.height),
            // The version's bits are the tag Qt compares: as the catalog's counter, it wraps where it wraps.
            ROLE_VERSION => QVariant::from(&(self.grid.version(at).cast_signed())),
            ROLE_THUMB_URLS => QVariant::from(
                &items.map(|i| self.thumb_url(&self.image_key(i), self.thumb_height)).collect::<QStringList>(),
            ),
            ROLE_THUMBHASH_URLS => {
                QVariant::from(&items.map(|i| self.thumbhash_url(&self.image_key(i))).collect::<QStringList>())
            }
            ROLE_BADGES => QVariant::from(&items.map(|i| self.badge(i)).collect::<QStringList>()),
            ROLE_RATINGS => QVariant::from(&items.map(|i| self.rating(i)).collect::<QList<i32>>()),
            ROLE_BORDERS_MARKED => QVariant::from(&items.map(|i| self.border_marked(i)).collect::<QList<bool>>()),
            ROLE_IMAGE_ASPECTS => QVariant::from(&items.map(|i| self.image_aspect(i)).collect::<QList<f64>>()),
            ROLE_ASPECTS => QVariant::from(&items.map(|i| self.aspect(i)).collect::<QList<f64>>()),
            ROLE_CONTENT_X => QVariant::from(&items.map(|i| self.border_box(i).x()).collect::<QList<f64>>()),
            ROLE_CONTENT_Y => QVariant::from(&items.map(|i| self.border_box(i).y()).collect::<QList<f64>>()),
            ROLE_CONTENT_WIDTH => QVariant::from(&items.map(|i| self.border_box(i).width()).collect::<QList<f64>>()),
            ROLE_CONTENT_HEIGHT => QVariant::from(&items.map(|i| self.border_box(i).height()).collect::<QList<f64>>()),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_SPAN, QByteArray::from("span"));
        roles.insert(ROLE_HEIGHT, QByteArray::from("rowHeight"));
        roles.insert(ROLE_VERSION, QByteArray::from("version"));
        for (role, name) in CELL_ROLES {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    /// The image's aspect as shown (pending turns included); 1 without its height (`ROLE_IMAGE_ASPECTS`).
    fn image_aspect(&self, item: i32) -> f64 {
        let (w, h) = (self.full_width(item), self.full_height(item));
        if h > 0 { f64::from(w) / f64::from(h) } else { 1.0 }
    }

    fn row_count(&self, _parent: &QModelIndex) -> i32 {
        qt_int(self.grid.rows().len())
    }

    fn register_model(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        MODEL.with(|model| *model.borrow_mut() = Some(thread));
    }

    /// Show the files of `listing` (a merged folder, with or without its subfolders, that pass its query, in its
    /// order). The list arrives later (`loaded`). With `keep` (the same listing, refreshed), the selection and `anchor`
    /// stay with their files; otherwise the selection is cleared.
    pub(crate) fn open_listing(mut self: Pin<&mut Self>, listing: pg_model::view::Listing, keep: bool) {
        let loading = self.loading;
        let generation = self.as_mut().rust_mut().grid.request(keep, loading);
        // A list held for the viewer is stale now: the one asked for is loading.
        let holding = self.grid.is_held();
        self.as_mut().set_holding(holding);
        self.as_mut().set_loading(true);
        let pg_model::view::Listing { folder, recursive, sort, query } = listing;
        let model = self.qt_thread();
        let shown = pg_model::rules::PathRules::NATIVE.display_merged(&folder);
        crate::app::get().session().grid(folder, recursive, query, sort, move |entries| {
            let entries = entries.unwrap_or_else(|e| {
                crate::panic_log::note(&format!("listing {shown}: {e:#}"));
                Vec::new()
            });
            // Fails only when the model is gone (the window closing): nothing left to show it in.
            let _ = model.queue(move |model| model.apply(generation, entries));
        });
    }

    fn set_viewing_holding(mut self: Pin<&mut Self>, value: i32) {
        if self.viewing == value {
            return;
        }
        self.as_mut().rust_mut().viewing = value;
        self.as_mut().viewing_changed();
        self.as_mut().refresh_viewed();
        if value < 0
            && let Some((generation, entries)) = self.as_mut().rust_mut().grid.release_held()
        {
            self.apply(generation, entries);
        }
    }

    /// Show a list `open_listing` asked for, unless a newer one was asked for since; while the viewer is open, when
    /// it closes.
    fn apply(mut self: Pin<&mut Self>, generation: u64, entries: Vec<Entry>) {
        let viewing = self.viewing >= 0;
        let arrival = self.as_mut().rust_mut().grid.arrive(generation, entries, viewing, fresh_entry);
        let holding = self.grid.is_held();
        self.as_mut().set_holding(holding);
        let entries = match arrival {
            Arrival::Ready(entries) => entries,
            Arrival::Stale | Arrival::Held => return,
        };
        self.as_mut().about_to_load();
        let keep = self.grid.keep();
        let (current, anchor) = (self.grid.id_at(self.current), self.grid.id_at(self.anchor));
        let count = qt_int(entries.len());
        let shown = {
            let mut this = self.as_mut().rust_mut();
            this.forget_details();
            this.grid.swap_entries(entries)
        };
        self.as_mut().set_count(count);
        // The same folder read again: only the rows that changed change.
        let changed = if keep { Changed::List(&shown) } else { Changed::All };
        let plan = self.as_mut().rust_mut().grid.plan(changed);
        self.as_mut().show(plan);
        // Before the turns are read again: they aren't on their way any more.
        self.as_mut().set_loading(false);
        if self.as_mut().rust_mut().grid.take_turned() {
            self.as_mut().refresh_turned();
        }
        // The selection stays with its file; if the file left the list (rated out of "Unrated"), what took its place.
        let selection = self.grid.selection_after(keep, self.current, current, anchor);
        self.as_mut().set_current_item(selection.current);
        self.as_mut().set_anchor(-1);
        self.as_mut().bump_revision();
        pg_trace::pg_stage_list_shown();
        self.loaded(selection.anchor);
    }

    fn relayout(mut self: Pin<&mut Self>, width: f64, row_height: f64, spacing: f64) {
        let plan = {
            let mut this = self.as_mut().rust_mut();
            this.grid.set_size(width, row_height, spacing);
            this.grid.plan(Changed::Nothing)
        };
        self.show(plan);
    }

    /// Puts a laid-out list in (`Grid::plan`). A reset, an insert or a remove is made around the change of the rows,
    /// as Qt wants it; then each row that changed in place is sent its changed roles, in one change per run of rows.
    /// The others keep their delegates: ListView keeps the first row in view where it is through them, where a reset
    /// takes the view to the top and fills it there.
    fn show(mut self: Pin<&mut Self>, plan: Plan) {
        let root = QModelIndex::default();
        let updates = match plan.shape() {
            Shape::Reset => {
                self.as_mut().begin_reset_model();
                let updates = self.as_mut().rust_mut().grid.commit(plan);
                self.as_mut().end_reset_model();
                updates
            }
            Shape::Insert(rows) => {
                self.as_mut().begin_insert_rows(&root, qt_int(rows.start), qt_int(rows.end) - 1);
                let updates = self.as_mut().rust_mut().grid.commit(plan);
                self.as_mut().end_insert_rows();
                updates
            }
            Shape::Remove(rows) => {
                self.as_mut().begin_remove_rows(&root, qt_int(rows.start), qt_int(rows.end) - 1);
                let updates = self.as_mut().rust_mut().grid.commit(plan);
                self.as_mut().end_remove_rows();
                updates
            }
            Shape::Same => self.as_mut().rust_mut().grid.commit(plan),
        };
        // A row's new span and its new version can come in one change: the QML reads its items again either way, so
        // every role that changed is signalled. The cells' lists go first, in a change of their own: the row takes
        // its values when its span or version arrives, and they must all be in by then.
        for update in updates {
            let (top, bottom) =
                (self.index(qt_int(update.rows.start), 0, &root), self.index(qt_int(update.rows.end) - 1, 0, &root));
            if update.span || update.version {
                self.as_mut().data_changed(&top, &bottom, &QList::from(cell_roles().as_slice()));
            }
            let mut roles = Vec::with_capacity(3);
            if update.span {
                roles.push(ROLE_SPAN);
            }
            if update.version {
                roles.push(ROLE_VERSION);
            }
            if update.height {
                roles.push(ROLE_HEIGHT);
            }
            self.as_mut().data_changed(&top, &bottom, &QList::from(roles.as_slice()));
        }
    }

    /// The thumbnails' height changed: each row gets a new version and its cells' lists are read again (the URLs are
    /// made for the new height), in the same order as `items_changed`.
    fn set_thumb_height(mut self: Pin<&mut Self>, height: i32) {
        if height == self.thumb_height {
            return;
        }
        self.as_mut().rust_mut().thumb_height = height;
        let rows = self.as_mut().rust_mut().grid.touch_all();
        for row in 0..rows {
            let index = self.index(qt_int(row), 0, &QModelIndex::default());
            self.as_mut().data_changed(&index, &index, &QList::from(cell_roles().as_slice()));
            self.as_mut().data_changed(&index, &index, &QList::from(&[ROLE_VERSION][..]));
        }
    }

    /// Items changed (rated, turned, marked) without changing shape: the cells of their rows read them again (the
    /// cells' lists, then `ROLE_VERSION`).
    fn items_changed(mut self: Pin<&mut Self>, items: &[usize]) {
        let rows = self.as_mut().rust_mut().grid.touch(items);
        for row in rows {
            let index = self.index(qt_int(row), 0, &QModelIndex::default());
            self.as_mut().data_changed(&index, &index, &QList::from(cell_roles().as_slice()));
            self.as_mut().data_changed(&index, &index, &QList::from(&[ROLE_VERSION][..]));
        }
    }

    /// The selection (`current`) changed, from QML or from a new list: its text is read again.
    pub(crate) fn set_current_item(mut self: Pin<&mut Self>, value: i32) {
        if value == self.current {
            return;
        }
        self.as_mut().rust_mut().current = value;
        self.as_mut().current_changed();
        self.as_mut().refresh_derived();
    }

    /// The grid's values changed (the list, a turn, a rating, a border mark): what the views read from them is read
    /// again first, then the revision moves on, which the views follow.
    fn bump_revision(mut self: Pin<&mut Self>) {
        self.as_mut().refresh_viewed();
        self.as_mut().refresh_derived();
        let revision = self.revision + 1;
        self.as_mut().set_revision(revision);
    }

    /// The status bar's selection text and the bordered count, the values the views used to call for.
    fn refresh_derived(mut self: Pin<&mut Self>) {
        let selection = if self.current >= 0 {
            let rating = self.rating(self.current);
            let symbol = if rating > 0 {
                text::rating_symbol(u8::try_from(rating.clamp(0, 5)).unwrap_or(0)).to_string()
            } else {
                String::new()
            };
            format!("{}   {symbol}", self.info(self.current))
        } else {
            String::new()
        };
        self.as_mut().set_selection_info(QString::from(&selection));
        let bordered = self.bordered_count();
        self.as_mut().set_croppable(bordered);
    }

    /// Sets what the viewer shows of its item (`viewing`), the viewed properties. Read again wherever what they show
    /// may have changed: when the item is set (`set_viewing_holding`), and before each change of `revision`. With no
    /// item (`-1`, the viewer closed) each reads as the item-less value (`""`, `0`, an empty rectangle, `-1` for a
    /// border line), which is what the viewer shows nothing with.
    fn refresh_viewed(mut self: Pin<&mut Self>) {
        let item = self.viewing;
        let key = self.image_key(item);
        let full = self.full_url(&key);
        self.as_mut().set_viewed_key(key);
        self.as_mut().set_viewed_full_url(full);
        let neighbours = self.keys_around(item, 4);
        self.as_mut().set_viewed_neighbour_keys(neighbours);
        let turns = self.pending_turns(item);
        self.as_mut().set_viewed_turns(turns);
        let rating = self.rating(item);
        self.as_mut().set_viewed_rating(rating);
        let width = self.full_width(item);
        self.as_mut().set_viewed_full_width(width);
        let height = self.full_height(item);
        self.as_mut().set_viewed_full_height(height);
        let kind = self.media_kind(item);
        self.as_mut().set_viewed_kind(kind);
        let player_turns = self.player_turns(item);
        self.as_mut().set_viewed_player_turns(player_turns);
        let file_id = self.file_id(item);
        self.as_mut().set_viewed_file_id(file_id);
        let info = self.info(item);
        self.as_mut().set_viewed_info(info);
        let border_box = self.border_box(item);
        self.as_mut().set_viewed_border_box(border_box);
        let top = self.border_rejected(item, "top");
        self.as_mut().set_viewed_rejected_top(top);
        let bottom = self.border_rejected(item, "bottom");
        self.as_mut().set_viewed_rejected_bottom(bottom);
        let left = self.border_rejected(item, "left");
        self.as_mut().set_viewed_rejected_left(left);
        let right = self.border_rejected(item, "right");
        self.as_mut().set_viewed_rejected_right(right);
        let marked = self.border_marked(item);
        self.as_mut().set_viewed_border_marked(marked);
    }

    fn file_id(&self, item: i32) -> i64 {
        self.grid.id_at(item).map_or(-1, FileId::get)
    }

    fn index_of(&self, file_id: i64) -> i32 {
        self.grid.position_of(Some(FileId::new(file_id)))
    }

    /// Width ÷ height, as laid out (`ROLE_ASPECTS`).
    fn aspect(&self, item: i32) -> f64 {
        self.grid.item(item).map_or(1.0, |i| layout::clamp_aspect(i.aspect()))
    }

    /// What names the item's image for the providers (`ImageKey`: `<file id>@<mtime>.<rotation>.<pending turns>`, see
    /// `providers`): it changes whenever the image does, so no stale image is shown.
    fn image_key(&self, item: i32) -> QString {
        self.grid.item(item).map_or_else(QString::default, |e| {
            let key = ImageKey {
                file: e.id,
                version: Some(Version { mtime: e.mtime, rotation: e.rotation, turns: e.pending_turns }),
            };
            // Formatted on the stack: the id and mtime at most 20 bytes each, the rotation and the turns one digit each
            // (0–3), so 20 + 1 + 20 + 1 + 1 + 1 + 1 = 45 bytes.
            let mut buf = [0u8; 64];
            let mut rest = &mut buf[..];
            if write!(rest, "{key}").is_err() {
                return QString::default();
            }
            let len = 64 - rest.len();
            let Some(Ok(text)) = buf.get(..len).map(std::str::from_utf8) else { return QString::default() };
            QString::from(text)
        })
    }

    /// The image keys of the items up to `reach` before and after `item`, nearest first, for the viewer to preload.
    fn keys_around(&self, item: i32, reach: i32) -> QStringList {
        if self.grid.item(item).is_none() {
            return QStringList::default();
        }
        (1..=reach)
            .flat_map(|d| [item - d, item + d])
            .filter(|&i| self.grid.item(i).is_some())
            .map(|i| self.image_key(i))
            .collect()
    }

    /// The URL of the full-size image of a file, the loupe's and the 1:1 view's (`image://full/<key>/full`).
    fn full_url(&self, key: &QString) -> QString {
        crate::viewport::full_url(key, pg_model::image::View::Full)
    }

    /// The URL of a file's thumbnail at a device-pixel height (`image://thumb/<key>/<height>`), or empty when the key
    /// doesn't read.
    fn thumb_url(&self, key: &QString, height: i32) -> QString {
        // The request's own grammar: the height is clamped to the thumbnails there is (`ImageRequest::parse`).
        let id = format!("{key}/{height}");
        ImageRequest::parse("thumb", &id).map_or_else(QString::default, |r| QString::from(&r.url()))
    }

    /// The URL of a file's placeholder (`image://thumbhash/<key>`), or empty when the key doesn't read.
    fn thumbhash_url(&self, key: &QString) -> QString {
        ImageRequest::parse("thumbhash", &key.to_string()).map_or_else(QString::default, |r| QString::from(&r.url()))
    }

    /// Upright size in pixels, as shown (pending turns included); 0 before the import.
    fn full_width(&self, item: i32) -> i32 {
        self.grid.item(item).map_or(0, |e| i32::try_from(e.shown_size().0).unwrap_or(i32::MAX))
    }

    fn full_height(&self, item: i32) -> i32 {
        self.grid.item(item).map_or(0, |e| i32::try_from(e.shown_size().1).unwrap_or(i32::MAX))
    }

    /// Quarter turns clockwise asked for but not yet written into the file (the images are served turned).
    fn pending_turns(&self, item: i32) -> i32 {
        self.grid.item(item).map_or(0, |i| i32::from(i.pending_turns.get()))
    }

    /// Turn a file by quarter turns (1 = clockwise, -1 = counterclockwise), losslessly, in the background: the
    /// controller's `Turn` command.
    pub(crate) fn rotate(mut self: Pin<&mut Self>, item: i32, turns: i32) {
        let Ok(at) = usize::try_from(item) else { return };
        let Some(id) = self.grid.id_at(item) else { return };
        let turns = Turns::new(u8::try_from(turns.rem_euclid(4)).unwrap_or(u8::MAX));
        if turns == Turns::NONE {
            return;
        }
        if let Err(e) = crate::app::get().turn(id, turns) {
            crate::panic_log::note(&format!("rotating: {e:#}"));
            return;
        }
        let loading = self.loading;
        {
            let mut this = self.as_mut().rust_mut();
            this.grid.edit(item, loading, |e| e.pending_turns = e.pending_turns.plus(turns));
            this.forget_details();
        }
        // An odd turn changes the item's shape, and so the rows.
        if pg_model::orientation::turns_swap(turns) {
            let plan = self.as_mut().rust_mut().grid.plan(Changed::Items(&[at]));
            self.as_mut().show(plan);
        } else {
            self.as_mut().items_changed(&[at]);
        }
        self.as_mut().bump_revision();
    }

    /// Files were turned: re-read the items still waiting for a turn (new image, size and version), in place, so
    /// nothing is renumbered under the viewer (the controller's `RefreshTurned` command, and a list that arrives).
    pub(crate) fn refresh_turned(mut self: Pin<&mut Self>) {
        // The list on its way may have been read before these turns were written: do it again when it arrives.
        let loading = self.loading;
        self.as_mut().rust_mut().grid.note_turns_written(loading);
        // Not the queued turns (`file_writes`): the writer deletes a turn's row once it's written, so these are
        // exactly the entries that still show one.
        let turned = self.as_mut().rust_mut().grid.refresh_turned(fresh_entry);
        if turned.changed.is_empty() {
            return;
        }
        // A written turn changes the file's size too: what was read of it is stale.
        self.as_mut().rust_mut().forget_details();
        if turned.reshaped {
            let plan = self.as_mut().rust_mut().grid.plan(Changed::Items(&turned.changed));
            self.as_mut().show(plan);
        } else {
            self.as_mut().items_changed(&turned.changed);
        }
        self.as_mut().bump_revision();
    }

    /// The item's rating, 0 for none (`ROLE_RATINGS`, the status bar's symbol).
    fn rating(&self, item: i32) -> i32 {
        self.grid.item(item).and_then(|i| i.rating).map_or(0, |r| i32::from(r.get()))
    }

    /// Rate the item (0 takes its rating away), in the catalog and, through the writer, in its file: the controller's
    /// `Rate` command.
    pub(crate) fn set_rating(mut self: Pin<&mut Self>, item: i32, rating: i32) {
        let Ok(at) = usize::try_from(item) else { return };
        let Some(id) = self.grid.id_at(item) else { return };
        let stars = u8::try_from(rating.clamp(0, 5)).unwrap_or(0);
        // Zero stars is no rating: the catalog keeps no row for it.
        let rating = Rating::new(stars).filter(|r| r.get() > 0);
        if let Err(e) = crate::app::get().rate(id, rating) {
            crate::panic_log::note(&format!("rating: {e:#}"));
            return;
        }
        let loading = self.loading;
        {
            let mut this = self.as_mut().rust_mut();
            this.grid.edit(item, loading, |e| e.rating = rating);
            this.forget_details();
        }
        self.as_mut().items_changed(&[at]);
        self.as_mut().bump_revision();
    }

    /// What the grid marks on a thumbnail besides its rating (`ROLE_BADGES`): a video's length (`▶ 1:23`), the
    /// format of an animated image (`GIF`, `APNG`, `WEBP`), or nothing.
    fn badge(&self, item: i32) -> QString {
        let Some(e) = self.grid.item(item) else { return QString::default() };
        QString::from(&text::badge(e.media_type(), e.duration_ms(), e.ext.name()))
    }

    /// What is left of the image without its borders, as fractions of the image as shown (pending turns included);
    /// an empty rectangle without borders found. Marked ones (`borderMarked`) have theirs.
    fn border_box(&self, item: i32) -> QRectF {
        match self.grid.item(item).and_then(Entry::content) {
            Some([x, y, w, h]) => QRectF::new(x, y, w, h),
            None => QRectF::default(),
        }
    }

    /// Whether the item's borders are marked as not a border (the review).
    pub(crate) fn border_marked(&self, item: i32) -> bool {
        self.grid.item(item).is_some_and(|e| e.borders == Borders::Marked)
    }

    /// Mark the item's borders as not a border, or take the mark back. Only items with borders found.
    pub(crate) fn set_border_marked(mut self: Pin<&mut Self>, item: i32, marked: bool) {
        let Ok(at) = usize::try_from(item) else { return };
        let Some(e) = self.grid.item(item).copied() else { return };
        if !matches!(e.borders, Borders::Found | Borders::Marked) || (e.borders == Borders::Marked) == marked {
            return;
        }
        if let Err(err) = crate::app::get().set_border_mark(e.id, marked) {
            crate::panic_log::note(&format!("marking borders: {err:#}"));
            return;
        }
        let loading = self.loading;
        self.as_mut()
            .rust_mut()
            .grid
            .edit(item, loading, |entry| entry.borders = if marked { Borders::Marked } else { Borders::Found });
        self.as_mut().items_changed(&[at]);
        self.as_mut().bump_revision();
    }

    /// Where the border detector's first rejected candidate at an edge of the image as shown (`top`, `bottom`, `left`,
    /// `right`) would have cut, as a fraction of the image from that edge; -1 for none.
    fn border_rejected(&self, item: i32, edge: &str) -> f64 {
        self.with_details(item, |e, d| {
            // A side's line, as the shown edge it is nearest (borders cut less than half a side) and the distance.
            let shown = |[x, y, w, _]: [f64; 4]| match (w == 0.0, x < 0.5, y < 0.5) {
                (true, true, _) => ("left", x),
                (true, false, _) => ("right", 1.0 - x),
                (false, _, true) => ("top", y),
                (false, _, false) => ("bottom", 1.0 - y),
            };
            d.border_sides
                .iter()
                .filter(|s| !s.accepted())
                .filter_map(|s| e.side_line(s.edge, s.depth).map(shown))
                .find(|(at, _)| *at == edge)
                .map_or(-1.0, |(_, d)| d)
        })
        .unwrap_or(-1.0)
    }

    fn save_cropped_copies_picked(&self) -> i32 {
        let Some(folder) = crate::picker::take(crate::picker::Purpose::SaveCrops) else {
            // The picker says `done` only with its folder, so this can't happen; if it does, the log records it.
            crate::panic_log::note("save cropped copies: no folder was chosen");
            return 0;
        };
        let ids: Vec<FileId> =
            self.grid.entries().iter().filter(|e| e.borders == Borders::Found).map(|e| e.id).collect();
        let n = qt_int(ids.len());
        if n > 0 {
            crate::app::get().save_crops(ids, folder);
        }
        n
    }

    /// How many of the listed files have borders found (and not marked): what `saveCroppedCopies` crops.
    fn bordered_count(&self) -> i32 {
        i32::try_from(self.grid.bordered()).unwrap_or(i32::MAX)
    }

    /// What the viewer shows the item with (`viewedKind`): its media type.
    fn media_kind(&self, item: i32) -> qobject::MediaKind {
        match self.grid.item(item).map(|e| e.media_type()) {
            Some(MediaType::Animated) => qobject::MediaKind::Animated,
            Some(MediaType::Video) => qobject::MediaKind::Video,
            Some(MediaType::Image) | None => qobject::MediaKind::Image,
        }
    }

    /// Quarter turns clockwise Qt's players must add to show the file upright: its catalog rotation and the turns not
    /// yet written. (A video's own rotation tag is applied by the player.)
    fn player_turns(&self, item: i32) -> i32 {
        self.grid.item(item).map_or(0, |e| i32::from(e.rotation.plus(e.pending_turns).get()))
    }

    /// One line for the status bar: path, dimensions, file size, where the rating is kept if not in the file (the
    /// rating's symbol is added after it, `refresh_derived`), and the borders found.
    fn info(&self, item: i32) -> QString {
        self.with_details(item, |e, d| {
            let classes: Vec<Class> = d.border_sides.iter().filter(|s| s.accepted()).map(|s| s.class()).collect();
            let borders = match (e.borders, e.shown_cut()) {
                (Borders::Found | Borders::Marked, Some(cut)) => {
                    Some(FoundBorders { marked: e.borders == Borders::Marked, cut, classes: &classes })
                }
                _ => None,
            };
            let kept = e.rating.is_some() && !d.rating_in_file;
            let path = PathRules::NATIVE.display(&d.path);
            QString::from(&text::info(&path, e.shown_size(), u64::try_from(d.size).unwrap_or(0), kept, borders))
        })
        .unwrap_or_default()
    }

    fn row_of(&self, item: i32) -> i32 {
        let Ok(at) = usize::try_from(item) else { return -1 };
        if self.grid.item(item).is_none() || self.grid.rows().is_empty() {
            return -1;
        }
        i32::try_from(self.grid.row_index(at)).unwrap_or(-1)
    }

    /// Keyboard navigation: the item `dx` to the side, or the one above/below (`dy`) nearest horizontally.
    fn neighbor(&self, item: i32, dx: i32, dy: i32) -> i32 {
        self.grid.placement().neighbor(item, dx, dy)
    }

    /// Whether `item` is an item of the list (the selection can only be one of them).
    pub(crate) fn is_item(&self, item: i32) -> bool {
        (0..self.count).contains(&item)
    }

    /// The neighbour of the selection `dx` columns and `dy` rows away; the first item when none is selected.
    pub(crate) fn neighbor_of_selection(&self, dx: i32, dy: i32) -> i32 {
        self.neighbor(self.current.max(0), dx, dy)
    }
}

/// A count or a row as Qt's int, saturating: a Qt model holds no more rows than an int counts.
fn qt_int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// The cells' roles (`CELL_ROLES`), for a change of their values.
fn cell_roles() -> Vec<i32> {
    CELL_ROLES.iter().map(|&(role, _)| role).collect()
}

/// A file's entry as the catalog has it now; `None` when it is gone or can't be read (the error is printed).
fn fresh_entry(id: FileId) -> Option<Entry> {
    match crate::app::get().entry(id) {
        Ok(found) => found,
        Err(e) => {
            crate::panic_log::note(&format!("reloading {}: {e:#}", id.get()));
            None
        }
    }
}
