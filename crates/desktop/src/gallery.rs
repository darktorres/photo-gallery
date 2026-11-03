//! The grid model: the files of the open folder, laid out in justified rows (`pg_client::layout`). QML sees one
//! list row per layout row (virtualized by ListView) and asks for each item's file id and size by index. The
//! model holds a compact entry per file (`browse::read::Entry`); a file's path and size are read when it is shown
//! (`browse::read::details`), for the selection and the viewer's file.
//!
//! The files are listed off the GUI thread (`loader`): `openFolder` asks, and the list arrives later (`loaded`).
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
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
        include!("cxx-qt-lib/qrectf.h");
        type QRectF = cxx_qt_lib::QRectF;
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qproperty(i32, count, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, revision, READ, WRITE, NOTIFY, FINAL)]
        /// A list asked for (`openFolder`) hasn't arrived yet; the one shown stays meanwhile.
        #[qproperty(bool, loading, READ, WRITE, NOTIFY, FINAL)]
        /// The selected item (-1: none): it follows its file to a new list.
        #[qproperty(i32, current, READ, WRITE, NOTIFY, FINAL)]
        /// The viewer's item (-1: closed). While it is open, a list that arrives is held, and shown when it closes.
        #[qproperty(i32, viewing, READ, WRITE = set_viewing_holding, NOTIFY, FINAL)]
        /// Without a selection, the item a new list keeps in view (the top one): set in `aboutToLoad`.
        #[qproperty(i32, anchor, READ, WRITE, NOTIFY, FINAL)]
        type GalleryModel = super::GalleryRust;
    }

    impl cxx_qt::Threading for GalleryModel {}

    unsafe extern "RustQt" {
        fn set_viewing_holding(self: Pin<&mut GalleryModel>, value: i32);

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

        /// Show the files of a merged folder (`""` = everything), with or without its subfolders, that pass the
        /// filters (`query::Query` as JSON), in an order (`query::Sort` as JSON): `view::View`'s and
        /// `filters::Filters`'. The list arrives later (`loaded`). With `keep` (the same folder, refreshed), the
        /// selection and `anchor` stay with their files; otherwise the selection is cleared.
        #[qinvokable]
        #[cxx_name = "openFolder"]
        fn open_folder(
            self: Pin<&mut GalleryModel>,
            merged: &QString,
            recursive: bool,
            query: &QString,
            sort: &QString,
            keep: bool,
        );

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

        /// Width ÷ height, as laid out.
        #[qinvokable]
        fn aspect(self: &GalleryModel, item: i32) -> f64;

        /// What names the item's image for the providers (`<file id>@<mtime>.<rotation>.<pending turns>`, see
        /// `providers`): it changes whenever the image does, so no stale image is shown.
        #[qinvokable]
        #[cxx_name = "imageKey"]
        fn image_key(self: &GalleryModel, item: i32) -> QString;

        /// The image keys of the items up to `reach` before and after `item`, nearest first, for the viewer to
        /// preload.
        #[qinvokable]
        #[cxx_name = "keysAround"]
        fn keys_around(self: &GalleryModel, item: i32, reach: i32) -> QStringList;

        /// Upright size in pixels, as shown (pending turns included); 0 before the import.
        #[qinvokable]
        #[cxx_name = "fullWidth"]
        fn full_width(self: &GalleryModel, item: i32) -> i32;

        #[qinvokable]
        #[cxx_name = "fullHeight"]
        fn full_height(self: &GalleryModel, item: i32) -> i32;

        /// Quarter turns clockwise asked for but not yet written into the file (the images are served turned).
        #[qinvokable]
        #[cxx_name = "pendingTurns"]
        fn pending_turns(self: &GalleryModel, item: i32) -> i32;

        /// Turn a file by quarter turns (1 = clockwise, -1 = counterclockwise), losslessly, in the background.
        #[qinvokable]
        fn rotate(self: Pin<&mut GalleryModel>, item: i32, turns: i32);

        /// Files were turned: re-read the items still waiting for a turn (new image, size and version), in place,
        /// so nothing is renumbered under the viewer.
        #[qinvokable]
        #[cxx_name = "refreshTurned"]
        fn refresh_turned(self: Pin<&mut GalleryModel>);

        /// Whether the file's format holds its rating too (false: kept in the catalog only).
        #[qinvokable]
        #[cxx_name = "ratingInFile"]
        fn rating_in_file(self: &GalleryModel, item: i32) -> bool;

        #[qinvokable]
        fn rating(self: &GalleryModel, item: i32) -> i32;

        #[qinvokable]
        #[cxx_name = "setRating"]
        fn set_rating(self: Pin<&mut GalleryModel>, item: i32, rating: i32);

        /// What the grid marks on a thumbnail besides its rating: a video's length (`▶ 1:23`), the format of an
        /// animated image (`GIF`, `APNG`, `WEBP`), or nothing.
        #[qinvokable]
        fn badge(self: &GalleryModel, item: i32) -> QString;

        /// What is left of the image without its borders, as fractions of the image as shown (pending turns
        /// included); an empty rectangle without borders found. Marked ones (`borderMarked`) have theirs.
        #[qinvokable]
        #[cxx_name = "borderBox"]
        fn border_box(self: &GalleryModel, item: i32) -> QRectF;

        /// Whether the item's borders are marked as not a border (the review).
        #[qinvokable]
        #[cxx_name = "borderMarked"]
        fn border_marked(self: &GalleryModel, item: i32) -> bool;

        /// Mark the item's borders as not a border, or take the mark back. Only items with borders found.
        #[qinvokable]
        #[cxx_name = "setBorderMarked"]
        fn set_border_marked(self: Pin<&mut GalleryModel>, item: i32, marked: bool);

        /// Where the border detector's first rejected candidate at an edge of the image as shown (`top`,
        /// `bottom`, `left`, `right`) would have cut, as a fraction of the image from that edge; -1 for none.
        #[qinvokable]
        #[cxx_name = "borderRejected"]
        fn border_rejected(self: &GalleryModel, item: i32, edge: &QString) -> f64;

        /// Save cropped copies of the listed files whose borders were found (and not marked) into the folder at
        /// `url`, in the background (`pg_library::crop`). Returns how many it asked for.
        #[qinvokable]
        #[cxx_name = "saveCroppedCopies"]
        fn save_cropped_copies(self: &GalleryModel, url: &QUrl) -> i32;

        /// How many of the listed files have borders found (and not marked): what `saveCroppedCopies` crops.
        #[qinvokable]
        #[cxx_name = "borderedCount"]
        fn bordered_count(self: &GalleryModel) -> i32;

        /// `image`, `animated` or `video` (`query::MediaType`): what the viewer shows it with.
        #[qinvokable]
        #[cxx_name = "mediaType"]
        fn media_type(self: &GalleryModel, item: i32) -> QString;

        /// The file as a URL, for Qt's player (videos).
        #[qinvokable]
        #[cxx_name = "fileUrl"]
        fn file_url(self: &GalleryModel, item: i32) -> QUrl;

        /// The file's path (for `AnimationPlayer`), or `""`.
        #[qinvokable]
        #[cxx_name = "filePath"]
        fn file_path(self: &GalleryModel, item: i32) -> QString;

        /// Quarter turns clockwise Qt's players must add to show the file upright: its catalog rotation and the
        /// turns not yet written. (A video's own rotation tag is applied by the player.)
        #[qinvokable]
        #[cxx_name = "playerTurns"]
        fn player_turns(self: &GalleryModel, item: i32) -> i32;

        /// One line for the status bar: path, dimensions, file size, where the rating is kept if not in the file
        /// (the QML adds the rating's symbol), and the borders found.
        #[qinvokable]
        fn info(self: &GalleryModel, item: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "rowOf"]
        fn row_of(self: &GalleryModel, item: i32) -> i32;

        /// Keyboard navigation: the item `dx` to the side, or the one above/below (`dy`) nearest horizontally.
        #[qinvokable]
        fn neighbor(self: &GalleryModel, item: i32, dx: i32, dy: i32) -> i32;
    }
}

use core::pin::Pin;
use std::cell::RefCell;
use std::io::Write;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QPointF, QRectF, QString, QStringList, QUrl,
    QVariant,
};
use pg_catalog::browse::{self};
use pg_catalog::query::from_json;
use pg_client::grid::{Arrival, Changed, Grid, Plan, Shape};
use pg_client::layout;
use pg_client::text::{self, FoundBorders};
use pg_model::details::Details;
use pg_model::detection::Class;
use pg_model::grid::{Borders, Entry};
use pg_model::ids::FileId;
use pg_model::query::{Query, Sort};
use pg_model::rating::Rating;
use pg_model::turns::Turns;

use crate::loader::{self, Kind as Load};
use crate::{library, settings, state};

/// A row's items, (first, count) as a point: one value, so a row changed or reused is given its items at once.
const ROLE_SPAN: i32 = 256;
const ROLE_HEIGHT: i32 = 257;
/// A row's version: a new one, never given before, whenever the items the row holds change without its span
/// changing (rated, turned, marked, rewritten, a new list). Its cells read their items again on it, not on every
/// change anywhere (`revision`, which the viewer and the toolbar follow); and a delegate the view reuses for a row
/// knows from it whether its cells show that row as it is.
const ROLE_VERSION: i32 = 258;

pub struct GalleryRust {
    count: i32,
    revision: i32,
    loading: bool,
    current: i32,
    viewing: i32,
    anchor: i32,
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
            loading: false,
            current: -1,
            viewing: -1,
            anchor: -1,
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
            let details = match state::with_catalog(|c| browse::read::details(c, e.id)) {
                Ok(d) => d?,
                Err(err) => {
                    eprintln!("reading {}: {err:#}", e.id.get());
                    return None;
                }
            };
            memo.truncate(1);
            memo.insert(0, (e.id, details));
        }
        Some(f(e, &memo[0].1))
    }

    /// A file changed, or a list is shown: what was read of it may be stale.
    fn forget_details(&mut self) {
        self.details.get_mut().clear();
    }
}

impl qobject::GalleryModel {
    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = self.grid.rows().get(index.row() as usize) else { return QVariant::default() };
        match role {
            ROLE_SPAN => QVariant::from(&QPointF::new(row.first as f64, row.count as f64)),
            ROLE_HEIGHT => QVariant::from(&row.height),
            ROLE_VERSION => QVariant::from(&(self.grid.version(index.row() as usize) as i32)),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_SPAN, QByteArray::from("span"));
        roles.insert(ROLE_HEIGHT, QByteArray::from("rowHeight"));
        roles.insert(ROLE_VERSION, QByteArray::from("version"));
        roles
    }

    fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.grid.rows().len() as i32
    }

    fn open_folder(
        mut self: Pin<&mut Self>,
        merged: &QString,
        recursive: bool,
        query: &QString,
        sort: &QString,
        keep: bool,
    ) {
        let loading = self.loading;
        let generation = self.as_mut().rust_mut().grid.request(keep, loading);
        self.as_mut().set_loading(true);
        let path = merged.to_string();
        let (query, sort) = (from_json::<Query>(&query.to_string()), from_json::<Sort>(&sort.to_string()));
        let model = self.qt_thread();
        let shown = path.clone();
        loader::submit(
            Load::Grid,
            move |c| browse::read::list_grid(c, &path, recursive, &query, sort),
            move |entries| {
                let entries = entries.unwrap_or_else(|e| {
                    eprintln!("listing {shown}: {e:#}");
                    Vec::new()
                });
                // Fails only when the model is gone (the window closing): nothing left to show it in.
                let _ = model.queue(move |model| model.apply(generation, entries));
            },
        );
    }

    fn set_viewing_holding(mut self: Pin<&mut Self>, value: i32) {
        if self.viewing == value {
            return;
        }
        self.as_mut().rust_mut().viewing = value;
        self.as_mut().viewing_changed();
        if value < 0
            && let Some((generation, entries)) = self.as_mut().rust_mut().grid.release_held()
        {
            self.apply(generation, entries);
        }
    }

    /// Show a list `open_folder` asked for, unless a newer one was asked for since; while the viewer is open, when
    /// it closes.
    fn apply(mut self: Pin<&mut Self>, generation: u64, entries: Vec<Entry>) {
        let viewing = self.viewing >= 0;
        let entries = match self.as_mut().rust_mut().grid.arrive(generation, entries, viewing, fresh_entry) {
            Arrival::Ready(entries) => entries,
            Arrival::Stale | Arrival::Held => return,
        };
        self.as_mut().about_to_load();
        let keep = self.grid.keep();
        let (current, anchor) = (self.grid.id_at(self.current), self.grid.id_at(self.anchor));
        let count = entries.len() as i32;
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
        self.as_mut().set_current(selection.current);
        self.as_mut().set_anchor(-1);
        let revision = self.revision + 1;
        self.as_mut().set_revision(revision);
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
                self.as_mut().begin_insert_rows(&root, rows.start as i32, rows.end as i32 - 1);
                let updates = self.as_mut().rust_mut().grid.commit(plan);
                self.as_mut().end_insert_rows();
                updates
            }
            Shape::Remove(rows) => {
                self.as_mut().begin_remove_rows(&root, rows.start as i32, rows.end as i32 - 1);
                let updates = self.as_mut().rust_mut().grid.commit(plan);
                self.as_mut().end_remove_rows();
                updates
            }
            Shape::Same => self.as_mut().rust_mut().grid.commit(plan),
        };
        // A row's new span and its new version can come in one change: the QML reads its items again either way, so
        // every role that changed is signalled, in the same change.
        for update in updates {
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
            let (top, bottom) =
                (self.index(update.rows.start as i32, 0, &root), self.index(update.rows.end as i32 - 1, 0, &root));
            self.as_mut().data_changed(&top, &bottom, &QList::from(roles.as_slice()));
        }
    }

    /// Items changed (rated, turned, marked) without changing shape: the cells of their rows read them again
    /// (`ROLE_VERSION`).
    fn items_changed(mut self: Pin<&mut Self>, items: &[usize]) {
        let rows = self.as_mut().rust_mut().grid.touch(items);
        for row in rows {
            let index = self.index(row as i32, 0, &QModelIndex::default());
            self.as_mut().data_changed(&index, &index, &QList::from(&[ROLE_VERSION][..]));
        }
    }

    fn file_id(&self, item: i32) -> i64 {
        self.grid.id_at(item).map_or(-1, FileId::get)
    }

    fn index_of(&self, file_id: i64) -> i32 {
        self.grid.position_of(Some(FileId::new(file_id)))
    }

    fn aspect(&self, item: i32) -> f64 {
        self.grid.item(item).map_or(1.0, |i| layout::clamp_aspect(i.aspect()))
    }

    fn image_key(&self, item: i32) -> QString {
        self.grid.item(item).map_or_else(QString::default, |e| {
            // Formatted on the stack: the id and mtime at most 20 bytes each, the rotation and the turns one digit each
            // (0–3), so 20 + 1 + 20 + 1 + 1 + 1 + 1 = 45 bytes.
            let mut buf = [0u8; 64];
            let mut rest = &mut buf[..];
            write!(rest, "{}@{}.{}.{}", e.id.get(), e.mtime, e.rotation.get(), e.pending_turns.get())
                .expect("64 bytes hold a key");
            let len = 64 - rest.len();
            QString::from(std::str::from_utf8(&buf[..len]).expect("ASCII"))
        })
    }

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

    fn full_width(&self, item: i32) -> i32 {
        self.grid.item(item).map_or(0, |e| e.shown_size().0 as i32)
    }

    fn full_height(&self, item: i32) -> i32 {
        self.grid.item(item).map_or(0, |e| e.shown_size().1 as i32)
    }

    fn pending_turns(&self, item: i32) -> i32 {
        self.grid.item(item).map_or(0, |i| i.pending_turns.get() as i32)
    }

    fn rotate(mut self: Pin<&mut Self>, item: i32, turns: i32) {
        let Some(id) = self.grid.id_at(item) else { return };
        let turns = Turns::new(turns.rem_euclid(4) as u8);
        if turns == Turns::NONE {
            return;
        }
        if let Err(e) = state::with_catalog(|c| pg_catalog::writes::queue_rotation(c, id, turns)) {
            eprintln!("rotating: {e:#}");
            return;
        }
        library::wake_writer();
        let loading = self.loading;
        {
            let mut this = self.as_mut().rust_mut();
            this.grid.edit(item, loading, |e| e.pending_turns = e.pending_turns.plus(turns));
            this.forget_details();
        }
        // An odd turn changes the item's shape, and so the rows.
        if pg_model::orientation::turns_swap(turns) {
            let plan = self.as_mut().rust_mut().grid.plan(Changed::Items(&[item as usize]));
            self.as_mut().show(plan);
        } else {
            self.as_mut().items_changed(&[item as usize]);
        }
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn refresh_turned(mut self: Pin<&mut Self>) {
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
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn rating_in_file(&self, item: i32) -> bool {
        self.with_details(item, |_, d| d.rating_in_file).unwrap_or(false)
    }

    fn rating(&self, item: i32) -> i32 {
        self.grid.item(item).and_then(|i| i.rating).map_or(0, |r| r.get() as i32)
    }

    fn set_rating(mut self: Pin<&mut Self>, item: i32, rating: i32) {
        let Some(id) = self.grid.id_at(item) else { return };
        let stars = rating.clamp(0, 5) as u8;
        // Zero stars is no rating: the catalog keeps no row for it.
        let rating = Rating::new(stars).filter(|r| r.get() > 0);
        let write_back = settings::prefs().write_back;
        if let Err(e) = state::with_catalog(|c| pg_catalog::writes::set_rating(c, id, rating, write_back)) {
            eprintln!("rating: {e:#}");
            return;
        }
        if write_back {
            library::wake_writer();
        }
        let loading = self.loading;
        {
            let mut this = self.as_mut().rust_mut();
            this.grid.edit(item, loading, |e| e.rating = rating);
            this.forget_details();
        }
        self.as_mut().items_changed(&[item as usize]);
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn badge(&self, item: i32) -> QString {
        let Some(e) = self.grid.item(item) else { return QString::default() };
        QString::from(&text::badge(e.media_type(), e.duration_ms(), e.ext.name()))
    }

    fn border_box(&self, item: i32) -> QRectF {
        match self.grid.item(item).and_then(Entry::content) {
            Some([x, y, w, h]) => QRectF::new(x, y, w, h),
            None => QRectF::default(),
        }
    }

    fn border_marked(&self, item: i32) -> bool {
        self.grid.item(item).is_some_and(|e| e.borders == Borders::Marked)
    }

    fn set_border_marked(mut self: Pin<&mut Self>, item: i32, marked: bool) {
        let Some(e) = self.grid.item(item).copied() else { return };
        if !matches!(e.borders, Borders::Found | Borders::Marked) || (e.borders == Borders::Marked) == marked {
            return;
        }
        if let Err(err) = state::with_catalog(|c| c.set_border_mark(e.id, marked)) {
            eprintln!("marking borders: {err:#}");
            return;
        }
        let loading = self.loading;
        self.as_mut()
            .rust_mut()
            .grid
            .edit(item, loading, |entry| entry.borders = if marked { Borders::Marked } else { Borders::Found });
        self.as_mut().items_changed(&[item as usize]);
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn border_rejected(&self, item: i32, edge: &QString) -> f64 {
        let edge = edge.to_string();
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

    fn save_cropped_copies(&self, url: &QUrl) -> i32 {
        let ids: Vec<FileId> =
            self.grid.entries().iter().filter(|e| e.borders == Borders::Found).map(|e| e.id).collect();
        let folder = url.to_local_file().map_or_else(|| url.to_qstring().to_string(), |p| p.to_string());
        let n = ids.len() as i32;
        if n > 0 {
            library::save_crops(ids, std::path::PathBuf::from(folder.replace('/', "\\")));
        }
        n
    }

    fn bordered_count(&self) -> i32 {
        self.grid.entries().iter().filter(|e| e.borders == Borders::Found).count() as i32
    }

    fn media_type(&self, item: i32) -> QString {
        QString::from(self.grid.item(item).map_or("image", |e| e.media_type().key()))
    }

    fn file_url(&self, item: i32) -> QUrl {
        // fromUserInput makes a local path a file URL, escaping what URLs reserve (names here contain '#').
        self.with_details(item, |_, d| QUrl::from_user_input(&QString::from(&d.path), &QString::default()))
            .unwrap_or_default()
    }

    fn file_path(&self, item: i32) -> QString {
        self.with_details(item, |_, d| QString::from(&d.path)).unwrap_or_default()
    }

    fn player_turns(&self, item: i32) -> i32 {
        self.grid.item(item).map_or(0, |e| e.rotation.plus(e.pending_turns).get() as i32)
    }

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
            QString::from(&text::info(&d.path, e.shown_size(), u64::try_from(d.size).unwrap_or(0), kept, borders))
        })
        .unwrap_or_default()
    }

    fn row_of(&self, item: i32) -> i32 {
        if self.grid.item(item).is_none() || self.grid.rows().is_empty() {
            return -1;
        }
        self.grid.row_index(item as usize) as i32
    }

    fn neighbor(&self, item: i32, dx: i32, dy: i32) -> i32 {
        self.grid.placement().neighbor(item, dx, dy)
    }
}

/// A file's entry as the catalog has it now; `None` when it is gone or can't be read (the error is printed).
fn fresh_entry(id: FileId) -> Option<Entry> {
    match state::with_catalog(|c| browse::read::entry(c, id)) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("reloading {}: {e:#}", id.get());
            None
        }
    }
}
