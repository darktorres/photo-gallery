//! The grid model: the files of the open folder, laid out in justified rows (`pg_core::layout`). QML sees one
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
        /// `url`, in the background (`pg_ingest::crop`). Returns how many it asked for.
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
use pg_catalog::browse::{
    self,
    read::{Borders, Details, Entry},
};
use pg_catalog::query::{MediaType, Query, Sort};
use pg_core::layout::{self, Row, RowChange};
use pg_model::detection::Class;
use pg_model::ids::FileId;
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

/// What changed in the items since the rows were last shown (`lay_out`).
enum Changed<'a> {
    /// Nothing: a new size.
    Nothing,
    /// These items (turned, rewritten).
    Items(&'a [usize]),
    /// The list was read again (the same folder): what it was.
    List(&'a [Entry]),
    /// Another list: every row is made again (a reset, which takes the view to the top).
    All,
}

pub struct GalleryRust {
    count: i32,
    revision: i32,
    loading: bool,
    current: i32,
    viewing: i32,
    anchor: i32,
    entries: Vec<Entry>,
    /// The details read last, of at most two files (the selection's and the viewer's), newest first. Emptied
    /// whenever a list is shown or a file changes.
    details: RefCell<Vec<(FileId, Details)>>,
    rows: Vec<Row>,
    /// The last layout's rows, kept for their capacity: a relayout is made into it, then swapped in.
    spare_rows: Vec<Row>,
    /// Per row (`ROLE_VERSION`).
    versions: Vec<u32>,
    /// The last version given.
    last_version: u32,
    spacing: f64,
    width: f64,
    row_height: f64,
    /// The latest list asked for; older ones arriving are dropped.
    generation: u64,
    /// Whether it keeps the selection: every request since the last list shown was a refresh of the same folder.
    keep: bool,
    /// A list that arrived while the viewer was open (its generation, its entries).
    held: Option<(u64, Vec<Entry>)>,
    /// Files rated or turned since it was asked for (its snapshot may predate them), and whether turns were
    /// written meanwhile (`refreshTurned`).
    edited: Vec<FileId>,
    turned: bool,
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
            entries: Vec::new(),
            details: RefCell::new(Vec::new()),
            rows: Vec::new(),
            spare_rows: Vec::new(),
            versions: Vec::new(),
            last_version: 0,
            spacing: 0.0,
            width: 0.0,
            row_height: 0.0,
            generation: 0,
            keep: false,
            held: None,
            edited: Vec::new(),
            turned: false,
        }
    }
}

impl GalleryRust {
    fn item(&self, i: i32) -> Option<&Entry> {
        usize::try_from(i).ok().and_then(|i| self.entries.get(i))
    }

    /// `f` of an item's entry and details; `None` without the item, or when its file is gone from the catalog.
    fn with_details<R>(&self, item: i32, f: impl FnOnce(&Entry, &Details) -> R) -> Option<R> {
        let e = self.item(item)?;
        let mut memo = self.details.borrow_mut();
        if let Some(at) = memo.iter().position(|(id, _)| *id == e.id) {
            let found = memo.remove(at);
            memo.insert(0, found);
        } else {
            let details = match state::with_catalog(|c| browse::read::details(&c.conn, e.id)) {
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

    fn new_version(&mut self) -> u32 {
        self.last_version += 1;
        self.last_version
    }

    fn row_index(&self, item: usize) -> usize {
        self.rows.partition_point(|r| r.first + r.count <= item)
    }

    /// Left edge and width of an item in its row.
    fn span(&self, item: usize) -> (f64, f64) {
        let row = &self.rows[self.row_index(item)];
        let w = |i: usize| layout::clamp_aspect(self.entries[i].aspect()) * row.height;
        let x: f64 = (row.first..item).map(|i| w(i) + self.spacing).sum();
        (x, w(item))
    }
}

impl qobject::GalleryModel {
    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = self.rows.get(index.row() as usize) else { return QVariant::default() };
        match role {
            ROLE_SPAN => QVariant::from(&QPointF::new(row.first as f64, row.count as f64)),
            ROLE_HEIGHT => QVariant::from(&row.height),
            ROLE_VERSION => QVariant::from(&(self.versions[index.row() as usize] as i32)),
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
        self.rows.len() as i32
    }

    fn open_folder(
        mut self: Pin<&mut Self>,
        merged: &QString,
        recursive: bool,
        query: &QString,
        sort: &QString,
        keep: bool,
    ) {
        let generation = {
            let mut this = self.as_mut().rust_mut();
            this.generation += 1;
            // A folder change still on its way stays one: a refresh asked meanwhile mustn't keep a selection made
            // in another folder.
            this.keep = keep && (this.keep || !this.loading);
            this.edited.clear();
            this.turned = false;
            this.generation
        };
        self.as_mut().set_loading(true);
        let path = merged.to_string();
        let (query, sort) = (Query::from_json(&query.to_string()), Sort::from_json(&sort.to_string()));
        let model = self.qt_thread();
        let shown = path.clone();
        loader::submit(
            Load::Grid,
            move |c| browse::read::list_grid(&c.conn, &path, recursive, &query, sort),
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
            && let Some((generation, entries)) = self.as_mut().rust_mut().held.take()
        {
            self.apply(generation, entries);
        }
    }

    /// Show a list `open_folder` asked for, unless a newer one was asked for since; while the viewer is open, when
    /// it closes.
    fn apply(mut self: Pin<&mut Self>, generation: u64, mut entries: Vec<Entry>) {
        if generation != self.generation {
            return;
        }
        if self.viewing >= 0 {
            self.as_mut().rust_mut().held = Some((generation, entries));
            return;
        }
        self.as_mut().about_to_load();
        let keep = self.keep;
        let (current, anchor) = (self.id_at(self.current), self.id_at(self.anchor));
        // What was rated or turned since the list was read, as the catalog has it now (this thread's connection
        // sees its own writes).
        let edited = std::mem::take(&mut self.as_mut().rust_mut().edited);
        for id in edited {
            if let Some(slot) = entries.iter_mut().find(|e| e.id == id) {
                match state::with_catalog(|c| browse::read::entry(&c.conn, id)) {
                    Ok(Some(fresh)) => *slot = fresh,
                    Ok(None) => {}
                    Err(e) => eprintln!("reloading {}: {e:#}", id.get()),
                }
            }
        }
        let count = entries.len() as i32;
        let shown = {
            let mut this = self.as_mut().rust_mut();
            this.forget_details();
            std::mem::replace(&mut this.entries, entries)
        };
        self.as_mut().set_count(count);
        // The same folder read again: only the rows that changed change.
        self.as_mut().lay_out(if keep { Changed::List(&shown) } else { Changed::All });
        // Before the turns are read again: they aren't on their way any more.
        self.as_mut().set_loading(false);
        if std::mem::take(&mut self.as_mut().rust_mut().turned) {
            self.as_mut().refresh_turned();
        }
        // The selection stays with its file; if the file left the list (rated out of "Unrated"), what took its place.
        let new_current = match (keep, self.current) {
            (false, _) | (true, -1) => -1,
            (true, old) => {
                let i = self.position_of(current);
                if i >= 0 || count == 0 { i } else { old.min(count - 1) }
            }
        };
        let kept = if keep { self.position_of(anchor) } else { -1 };
        self.as_mut().set_current(new_current);
        self.as_mut().set_anchor(-1);
        let revision = self.revision + 1;
        self.as_mut().set_revision(revision);
        pg_trace::pg_stage_list_shown();
        self.loaded(kept);
    }

    fn relayout(mut self: Pin<&mut Self>, width: f64, row_height: f64, spacing: f64) {
        {
            let mut this = self.as_mut().rust_mut();
            (this.width, this.row_height, this.spacing) = (width, row_height, spacing);
        }
        self.lay_out(Changed::Nothing);
    }

    /// Lay the items out again at the current size, and show what changed. Another list is a reset. Otherwise only
    /// the rows that changed change (in place, inserted or removed: `layout::RowChange`), and the others keep their
    /// delegates: ListView keeps the first row in view where it is through them, where a reset takes the view to
    /// the top and fills it there.
    fn lay_out(mut self: Pin<&mut Self>, changed: Changed) {
        let mut rows = std::mem::take(&mut self.as_mut().rust_mut().spare_rows);
        if self.width > 0.0 && self.row_height > 0.0 {
            let (width, height, spacing) = (self.width, self.row_height, self.spacing);
            layout::justify_into(&mut rows, self.entries.iter().map(Entry::aspect), width, height, spacing);
        } else {
            rows.clear();
        }
        if let Changed::All = changed {
            self.as_mut().begin_reset_model();
            {
                let mut this = self.as_mut().rust_mut();
                this.spare_rows = std::mem::replace(&mut this.rows, rows);
                let (count, version) = (this.rows.len(), this.new_version());
                this.versions.clear();
                this.versions.resize(count, version);
            }
            self.end_reset_model();
            return;
        }
        let change = RowChange::between(&self.rows, &rows);
        let updates = change.updates(&self.rows, &rows, |span| match changed {
            Changed::Nothing | Changed::All => false,
            Changed::Items(items) => items.iter().any(|i| span.contains(i)),
            Changed::List(shown) => shown.get(span.clone()) != self.entries.get(span),
        });
        let version = self.as_mut().rust_mut().new_version();
        let root = QModelIndex::default();
        let at = change.changed.end;
        let swap = |mut this: Pin<&mut Self>, rows: Vec<Row>| {
            let mut this = this.as_mut().rust_mut();
            this.spare_rows = std::mem::replace(&mut this.rows, rows);
        };
        if change.inserted > 0 {
            let last = at + change.inserted - 1;
            self.as_mut().begin_insert_rows(&root, at as i32, last as i32);
            swap(self.as_mut(), rows);
            self.as_mut().rust_mut().versions.splice(at..at, std::iter::repeat_n(version, change.inserted));
            self.as_mut().end_insert_rows();
        } else if change.removed > 0 {
            let last = at + change.removed - 1;
            self.as_mut().begin_remove_rows(&root, at as i32, last as i32);
            swap(self.as_mut(), rows);
            self.as_mut().rust_mut().versions.drain(at..=last);
            self.as_mut().end_remove_rows();
        } else {
            swap(self.as_mut(), rows);
        }
        // A row's new span and its new version can come in one change: the QML reads its items again either way, so
        // every role that changed is signalled, in the same change.
        for (range, update) in updates {
            let mut roles = Vec::with_capacity(3);
            if update.span {
                roles.push(ROLE_SPAN);
            }
            if update.items {
                roles.push(ROLE_VERSION);
            }
            if update.span || !update.items || update.height {
                roles.push(ROLE_HEIGHT);
            }
            if update.items {
                self.as_mut().rust_mut().versions[range.clone()].fill(version);
            }
            let (top, bottom) = (self.index(range.start as i32, 0, &root), self.index(range.end as i32 - 1, 0, &root));
            self.as_mut().data_changed(&top, &bottom, &QList::from(roles.as_slice()));
        }
    }

    /// Items changed (rated, turned, marked) without changing shape: the cells of their rows read them again
    /// (`ROLE_VERSION`).
    fn items_changed(mut self: Pin<&mut Self>, items: &[usize]) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        let mut rows: Vec<usize> = items.iter().map(|&i| self.row_index(i).min(last)).collect();
        rows.sort_unstable();
        rows.dedup();
        let version = self.as_mut().rust_mut().new_version();
        for row in rows {
            self.as_mut().rust_mut().versions[row] = version;
            let index = self.index(row as i32, 0, &QModelIndex::default());
            self.as_mut().data_changed(&index, &index, &QList::from(&[ROLE_VERSION][..]));
        }
    }

    /// The file of an item, `None` without the item.
    fn id_at(&self, item: i32) -> Option<FileId> {
        self.item(item).map(|i| i.id)
    }

    /// The position of a file in the list, -1 when it is not there (or `None`).
    fn position_of(&self, id: Option<FileId>) -> i32 {
        id.and_then(|id| self.entries.iter().position(|e| e.id == id)).map_or(-1, |i| i as i32)
    }

    fn file_id(&self, item: i32) -> i64 {
        self.id_at(item).map_or(-1, FileId::get)
    }

    fn index_of(&self, file_id: i64) -> i32 {
        self.position_of(Some(FileId::new(file_id)))
    }

    fn aspect(&self, item: i32) -> f64 {
        self.item(item).map_or(1.0, |i| layout::clamp_aspect(i.aspect()))
    }

    fn image_key(&self, item: i32) -> QString {
        self.item(item).map_or_else(QString::default, |e| {
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
        if self.item(item).is_none() {
            return QStringList::default();
        }
        (1..=reach)
            .flat_map(|d| [item - d, item + d])
            .filter(|&i| self.item(i).is_some())
            .map(|i| self.image_key(i))
            .collect()
    }

    fn full_width(&self, item: i32) -> i32 {
        self.item(item).map_or(0, |e| e.shown_size().0 as i32)
    }

    fn full_height(&self, item: i32) -> i32 {
        self.item(item).map_or(0, |e| e.shown_size().1 as i32)
    }

    fn pending_turns(&self, item: i32) -> i32 {
        self.item(item).map_or(0, |i| i.pending_turns.get() as i32)
    }

    fn rotate(mut self: Pin<&mut Self>, item: i32, turns: i32) {
        let Some(id) = self.id_at(item) else { return };
        let turns = Turns::new(turns.rem_euclid(4) as u8);
        if turns == Turns::NONE {
            return;
        }
        if let Err(e) = state::with_catalog(|c| pg_catalog::writes::queue_rotation(&c.conn, id, turns)) {
            eprintln!("rotating: {e:#}");
            return;
        }
        library::wake_writer();
        {
            let mut this = self.as_mut().rust_mut();
            let e = &mut this.entries[item as usize];
            e.pending_turns = e.pending_turns.plus(turns);
            this.forget_details();
            if this.loading {
                this.edited.push(id);
            }
        }
        // An odd turn changes the item's shape, and so the rows.
        if pg_model::orientation::turns_swap(turns) {
            self.as_mut().lay_out(Changed::Items(&[item as usize]));
        } else {
            self.as_mut().items_changed(&[item as usize]);
        }
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn refresh_turned(mut self: Pin<&mut Self>) {
        // The list on its way may have been read before these turns were written: do it again when it arrives.
        if self.loading {
            self.as_mut().rust_mut().turned = true;
        }
        // Not the queued turns (`file_writes`): the writer deletes a turn's row once it's written, so these are
        // exactly the entries that still show one.
        let waiting: Vec<usize> =
            (0..self.entries.len()).filter(|&i| self.entries[i].pending_turns != Turns::NONE).collect();
        if waiting.is_empty() {
            return;
        }
        // A written turn changes the file's size too.
        self.as_mut().rust_mut().forget_details();
        let mut changed = Vec::new();
        let mut reshaped = false;
        for i in waiting {
            let id = self.entries[i].id;
            match state::with_catalog(|c| browse::read::entry(&c.conn, id)) {
                Ok(Some(fresh)) if fresh != self.entries[i] => {
                    reshaped |= fresh.aspect() != self.entries[i].aspect();
                    self.as_mut().rust_mut().entries[i] = fresh;
                    changed.push(i);
                }
                Ok(_) => {}
                Err(e) => eprintln!("reloading {}: {e:#}", id.get()),
            }
        }
        if changed.is_empty() {
            return;
        }
        if reshaped {
            self.as_mut().lay_out(Changed::Items(&changed));
        } else {
            self.as_mut().items_changed(&changed);
        }
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn rating_in_file(&self, item: i32) -> bool {
        self.with_details(item, |_, d| d.rating_in_file).unwrap_or(false)
    }

    fn rating(&self, item: i32) -> i32 {
        self.item(item).map_or(0, |i| i.rating as i32)
    }

    fn set_rating(mut self: Pin<&mut Self>, item: i32, rating: i32) {
        let Some(id) = self.id_at(item) else { return };
        let stars = rating.clamp(0, 5) as u8;
        // Zero stars is no rating: the catalog keeps no row for it.
        let rating = Rating::new(stars).filter(|r| r.get() > 0);
        let write_back = settings::prefs().write_back;
        if let Err(e) = state::with_catalog(|c| pg_catalog::writes::set_rating(&c.conn, id, rating, write_back)) {
            eprintln!("rating: {e:#}");
            return;
        }
        if write_back {
            library::wake_writer();
        }
        {
            let mut this = self.as_mut().rust_mut();
            this.entries[item as usize].rating = stars;
            this.forget_details();
            if this.loading {
                this.edited.push(id);
            }
        }
        self.as_mut().items_changed(&[item as usize]);
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn badge(&self, item: i32) -> QString {
        let Some(e) = self.item(item) else { return QString::default() };
        let text = match e.media_type() {
            MediaType::Video => match e.duration_ms() {
                Some(ms) => {
                    let s = (ms + 500) / 1000;
                    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
                    if h > 0 { format!("▶ {h}:{m:02}:{s:02}") } else { format!("▶ {m}:{s:02}") }
                }
                None => "▶".to_string(),
            },
            MediaType::Animated => match e.ext.name() {
                "png" => "APNG".to_string(),
                ext => ext.to_uppercase(),
            },
            MediaType::Image => String::new(),
        };
        QString::from(&text)
    }

    fn border_box(&self, item: i32) -> QRectF {
        match self.item(item).and_then(Entry::content) {
            Some([x, y, w, h]) => QRectF::new(x, y, w, h),
            None => QRectF::default(),
        }
    }

    fn border_marked(&self, item: i32) -> bool {
        self.item(item).is_some_and(|e| e.borders == Borders::Marked)
    }

    fn set_border_marked(mut self: Pin<&mut Self>, item: i32, marked: bool) {
        let Some(e) = self.item(item).copied() else { return };
        if !matches!(e.borders, Borders::Found | Borders::Marked) || (e.borders == Borders::Marked) == marked {
            return;
        }
        if let Err(err) = state::with_catalog(|c| pg_catalog::Catalog::set_border_mark(&c.conn, e.id, marked)) {
            eprintln!("marking borders: {err:#}");
            return;
        }
        {
            let mut this = self.as_mut().rust_mut();
            this.entries[item as usize].borders = if marked { Borders::Marked } else { Borders::Found };
            if this.loading {
                this.edited.push(e.id);
            }
        }
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
        let ids: Vec<FileId> = self.entries.iter().filter(|e| e.borders == Borders::Found).map(|e| e.id).collect();
        let folder = url.to_local_file().map_or_else(|| url.to_qstring().to_string(), |p| p.to_string());
        let n = ids.len() as i32;
        if n > 0 {
            library::save_crops(ids, std::path::PathBuf::from(folder.replace('/', "\\")));
        }
        n
    }

    fn bordered_count(&self) -> i32 {
        self.entries.iter().filter(|e| e.borders == Borders::Found).count() as i32
    }

    fn media_type(&self, item: i32) -> QString {
        QString::from(self.item(item).map_or("image", |e| e.media_type().key()))
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
        self.item(item).map_or(0, |e| e.rotation.plus(e.pending_turns).get() as i32)
    }

    fn info(&self, item: i32) -> QString {
        self.with_details(item, |e, d| {
            let (w, h) = e.shown_size();
            let dims = if w > 0 { format!("{w} × {h}") } else { "not imported".to_string() };
            let bytes = d.size as f64;
            let size = if bytes >= 1e6 { format!("{:.1} MB", bytes / 1e6) } else { format!("{:.0} KB", bytes / 1e3) };
            let kept = if e.rating > 0 && !d.rating_in_file { "   rating kept in the catalog" } else { "" };
            QString::from(&format!("{}   {dims}   {size}{kept}{}", d.path, borders_info(e, d)))
        })
        .unwrap_or_default()
    }

    fn row_of(&self, item: i32) -> i32 {
        if self.item(item).is_none() || self.rows.is_empty() {
            return -1;
        }
        self.row_index(item as usize) as i32
    }

    fn neighbor(&self, item: i32, dx: i32, dy: i32) -> i32 {
        let n = self.entries.len() as i32;
        if n == 0 || self.rows.is_empty() {
            return -1;
        }
        let item = item.clamp(0, n - 1);
        if dy == 0 {
            return (item + dx).clamp(0, n - 1);
        }
        let row = self.row_index(item as usize) as i32;
        let target = (row + dy).clamp(0, self.rows.len() as i32 - 1);
        if target == row {
            return item;
        }
        let (x, w) = self.span(item as usize);
        let center = x + w / 2.0;
        let r = self.rows[target as usize];
        (r.first..r.first + r.count)
            .find(|&i| {
                let (x, w) = self.span(i);
                center < x + w + self.spacing
            })
            .unwrap_or(r.first + r.count - 1) as i32
    }
}

/// The status line's part about borders: what a crop would cut, in stored pixels, and their classes.
fn borders_info(e: &Entry, d: &Details) -> String {
    if !matches!(e.borders, Borders::Found | Borders::Marked) {
        return String::new();
    }
    let Some([l, t, r, b]) = e.shown_cut() else { return String::new() };
    let mut classes: Vec<&str> = Vec::new();
    for c in d.border_sides.iter().filter(|s| s.accepted()).map(|s| s.class()) {
        let name = match c {
            Class::Black => "black",
            Class::White => "white",
            Class::Color => "colored",
            Class::Blur => "blurred",
        };
        if !classes.contains(&name) {
            classes.push(name);
        }
    }
    let marked = if e.borders == Borders::Marked { " — marked not a border" } else { "" };
    format!("   borders {} (L {l} T {t} R {r} B {b} px){marked}", classes.join(", "))
}
