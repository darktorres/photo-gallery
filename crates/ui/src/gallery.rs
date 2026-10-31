//! The grid model: the files of the open folder, laid out in justified rows (`pg_core::layout`). QML sees one
//! list row per layout row (virtualized by ListView) and asks for each item's file id and size by index.
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

        #[qinvokable]
        fn path(self: &GalleryModel, item: i32) -> QString;

        /// What the grid marks on a thumbnail besides its rating: a video's length (`▶ 1:23`), the format of an
        /// animated image (`GIF`, `APNG`, `WEBP`), or nothing.
        #[qinvokable]
        fn badge(self: &GalleryModel, item: i32) -> QString;

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

        /// One line for the status bar: path, dimensions, file size, and where the rating is kept if not in the
        /// file (the QML adds the rating's symbol).
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

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QStringList, QUrl, QVariant};
use pg_core::browse::{self, Item};
use pg_core::layout::{self, Row};
use pg_core::media::{self, Kind};
use pg_core::query::{MediaType, Query, Sort};

use crate::loader::{self, Kind as Load};
use crate::{library, settings, state, trace};

const ROLE_FIRST: i32 = 256;
const ROLE_COUNT: i32 = 257;
const ROLE_HEIGHT: i32 = 258;

pub struct GalleryRust {
    count: i32,
    revision: i32,
    loading: bool,
    current: i32,
    viewing: i32,
    anchor: i32,
    items: Vec<Item>,
    rows: Vec<Row>,
    spacing: f64,
    width: f64,
    row_height: f64,
    /// The latest list asked for; older ones arriving are dropped.
    generation: u64,
    /// Whether it keeps the selection: every request since the last list shown was a refresh of the same folder.
    keep: bool,
    /// A list that arrived while the viewer was open (its generation, its items).
    held: Option<(u64, Vec<Item>)>,
    /// Files rated or turned since it was asked for (its snapshot may predate them), and whether turns were
    /// written meanwhile (`refreshTurned`).
    edited: Vec<i64>,
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
            items: Vec::new(),
            rows: Vec::new(),
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

fn media_type(i: &Item) -> MediaType {
    match i.kind {
        Kind::Video => MediaType::Video,
        Kind::Image if i.animated => MediaType::Animated,
        Kind::Image => MediaType::Image,
    }
}

impl GalleryRust {
    fn item(&self, i: i32) -> Option<&Item> {
        usize::try_from(i).ok().and_then(|i| self.items.get(i))
    }

    fn row_index(&self, item: usize) -> usize {
        self.rows.partition_point(|r| r.first + r.count <= item)
    }

    /// Left edge and width of an item in its row.
    fn span(&self, item: usize) -> (f64, f64) {
        let row = &self.rows[self.row_index(item)];
        let w = |i: usize| layout::clamp_aspect(self.items[i].aspect()) * row.height;
        let x: f64 = (row.first..item).map(|i| w(i) + self.spacing).sum();
        (x, w(item))
    }
}

impl qobject::GalleryModel {
    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = self.rows.get(index.row() as usize) else { return QVariant::default() };
        match role {
            ROLE_FIRST => QVariant::from(&(row.first as i32)),
            ROLE_COUNT => QVariant::from(&(row.count as i32)),
            ROLE_HEIGHT => QVariant::from(&row.height),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_FIRST, QByteArray::from("firstItem"));
        roles.insert(ROLE_COUNT, QByteArray::from("itemCount"));
        roles.insert(ROLE_HEIGHT, QByteArray::from("rowHeight"));
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
            move |c| browse::list_files(&c.conn, &path, recursive, &query, sort),
            move |items| {
                let items = items.unwrap_or_else(|e| {
                    eprintln!("listing {shown}: {e:#}");
                    Vec::new()
                });
                // Fails only when the model is gone (the window closing): nothing left to show it in.
                let _ = model.queue(move |model| model.apply(generation, items));
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
            && let Some((generation, items)) = self.as_mut().rust_mut().held.take()
        {
            self.apply(generation, items);
        }
    }

    /// Show a list `open_folder` asked for, unless a newer one was asked for since; while the viewer is open, when
    /// it closes.
    fn apply(mut self: Pin<&mut Self>, generation: u64, mut items: Vec<Item>) {
        if generation != self.generation {
            return;
        }
        if self.viewing >= 0 {
            self.as_mut().rust_mut().held = Some((generation, items));
            return;
        }
        self.as_mut().about_to_load();
        let keep = self.keep;
        let (current, anchor) = (self.file_id(self.current), self.file_id(self.anchor));
        // What was rated or turned since the list was read, as the catalog has it now (this thread's connection
        // sees its own writes).
        let edited = std::mem::take(&mut self.as_mut().rust_mut().edited);
        for id in edited {
            if let Some(slot) = items.iter_mut().find(|i| i.id == id) {
                match state::with_catalog(|c| browse::reload_item(&c.conn, id)) {
                    Ok(Some(fresh)) => *slot = fresh,
                    Ok(None) => {}
                    Err(e) => eprintln!("reloading {id}: {e:#}"),
                }
            }
        }
        let count = items.len() as i32;
        self.as_mut().rust_mut().items = items;
        self.as_mut().set_count(count);
        let (w, h, s) = (self.width, self.row_height, self.spacing);
        self.as_mut().relayout(w, h, s);
        // Before the turns are read again: they aren't on their way any more.
        self.as_mut().set_loading(false);
        if std::mem::take(&mut self.as_mut().rust_mut().turned) {
            self.as_mut().refresh_turned();
        }
        // The selection stays with its file; if the file left the list (rated out of "Unrated"), what took its place.
        let new_current = match (keep, self.current) {
            (false, _) | (true, -1) => -1,
            (true, old) => {
                let i = self.index_of(current);
                if i >= 0 || count == 0 { i } else { old.min(count - 1) }
            }
        };
        let kept = if keep && anchor >= 0 { self.index_of(anchor) } else { -1 };
        self.as_mut().set_current(new_current);
        self.as_mut().set_anchor(-1);
        let revision = self.revision + 1;
        self.as_mut().set_revision(revision);
        trace::pg_stage_list_shown();
        self.loaded(kept);
    }

    fn relayout(mut self: Pin<&mut Self>, width: f64, row_height: f64, spacing: f64) {
        self.as_mut().begin_reset_model();
        {
            let mut this = self.as_mut().rust_mut();
            this.width = width;
            this.row_height = row_height;
            this.spacing = spacing;
            let aspects: Vec<f64> = this.items.iter().map(Item::aspect).collect();
            this.rows = if width > 0.0 && row_height > 0.0 {
                layout::justify(&aspects, width, row_height, spacing)
            } else {
                Vec::new()
            };
        }
        self.end_reset_model();
    }

    fn file_id(&self, item: i32) -> i64 {
        self.item(item).map_or(-1, |i| i.id)
    }

    fn index_of(&self, file_id: i64) -> i32 {
        self.items.iter().position(|i| i.id == file_id).map_or(-1, |i| i as i32)
    }

    fn aspect(&self, item: i32) -> f64 {
        self.item(item).map_or(1.0, |i| layout::clamp_aspect(i.aspect()))
    }

    fn image_key(&self, item: i32) -> QString {
        self.item(item).map_or_else(QString::default, |i| {
            QString::from(&format!("{}@{}.{}.{}", i.id, i.mtime, i.rotation, i.pending_turns))
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
        self.item(item).map_or(0, |i| if i.pending_turns % 2 == 1 { i.height } else { i.width } as i32)
    }

    fn full_height(&self, item: i32) -> i32 {
        self.item(item).map_or(0, |i| if i.pending_turns % 2 == 1 { i.width } else { i.height } as i32)
    }

    fn pending_turns(&self, item: i32) -> i32 {
        self.item(item).map_or(0, |i| i.pending_turns as i32)
    }

    fn rotate(mut self: Pin<&mut Self>, item: i32, turns: i32) {
        let Some(id) = self.item(item).map(|i| i.id) else { return };
        let turns = turns.rem_euclid(4) as u8;
        if turns == 0 {
            return;
        }
        if let Err(e) = state::with_catalog(|c| browse::queue_rotation(&c.conn, id, turns)) {
            eprintln!("rotating: {e:#}");
            return;
        }
        library::wake_writer();
        {
            let mut this = self.as_mut().rust_mut();
            let i = &mut this.items[item as usize];
            i.pending_turns = (i.pending_turns + turns) % 4;
            if this.loading {
                this.edited.push(id);
            }
        }
        // An odd turn changes the item's shape, and so the rows.
        if turns % 2 == 1 {
            let (w, h, s) = (self.width, self.row_height, self.spacing);
            self.as_mut().relayout(w, h, s);
        }
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn refresh_turned(mut self: Pin<&mut Self>) {
        // The list on its way may have been read before these turns were written: do it again when it arrives.
        if self.loading {
            self.as_mut().rust_mut().turned = true;
        }
        let waiting: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].pending_turns != 0).collect();
        if waiting.is_empty() {
            return;
        }
        let mut reshaped = false;
        for i in waiting {
            let id = self.items[i].id;
            match state::with_catalog(|c| browse::reload_item(&c.conn, id)) {
                Ok(Some(fresh)) => {
                    reshaped |= fresh.aspect() != self.items[i].aspect();
                    self.as_mut().rust_mut().items[i] = fresh;
                }
                Ok(None) => {}
                Err(e) => eprintln!("reloading {id}: {e:#}"),
            }
        }
        if reshaped {
            let (w, h, s) = (self.width, self.row_height, self.spacing);
            self.as_mut().relayout(w, h, s);
        }
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn rating_in_file(&self, item: i32) -> bool {
        self.item(item).is_some_and(|i| i.rating_in_file)
    }

    fn rating(&self, item: i32) -> i32 {
        self.item(item).map_or(0, |i| i.rating as i32)
    }

    fn set_rating(mut self: Pin<&mut Self>, item: i32, rating: i32) {
        let Some(id) = self.item(item).map(|i| i.id) else { return };
        let rating = rating.clamp(0, 5) as u8;
        let write_back = settings::prefs().write_back;
        if let Err(e) = state::with_catalog(|c| browse::set_rating(&c.conn, id, rating, write_back)) {
            eprintln!("rating: {e:#}");
            return;
        }
        if write_back {
            library::wake_writer();
        }
        {
            let mut this = self.as_mut().rust_mut();
            this.items[item as usize].rating = rating;
            if this.loading {
                this.edited.push(id);
            }
        }
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn path(&self, item: i32) -> QString {
        self.item(item).map_or_else(QString::default, |i| QString::from(&i.path))
    }

    fn badge(&self, item: i32) -> QString {
        let Some(i) = self.item(item) else { return QString::default() };
        let text = match media_type(i) {
            MediaType::Video => match i.duration_ms {
                Some(ms) => {
                    let s = (ms + 500) / 1000;
                    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
                    if h > 0 { format!("▶ {h}:{m:02}:{s:02}") } else { format!("▶ {m}:{s:02}") }
                }
                None => "▶".to_string(),
            },
            MediaType::Animated => match media::extension(&i.path).as_str() {
                "png" => "APNG".to_string(),
                ext => ext.to_uppercase(),
            },
            MediaType::Image => String::new(),
        };
        QString::from(&text)
    }

    fn media_type(&self, item: i32) -> QString {
        QString::from(self.item(item).map_or("image", |i| media_type(i).key()))
    }

    fn file_url(&self, item: i32) -> QUrl {
        // fromUserInput makes a local path a file URL, escaping what URLs reserve (names here contain '#').
        self.item(item)
            .map_or_else(QUrl::default, |i| QUrl::from_user_input(&QString::from(&i.path), &QString::default()))
    }

    fn file_path(&self, item: i32) -> QString {
        self.item(item).map_or_else(QString::default, |i| QString::from(&i.path))
    }

    fn player_turns(&self, item: i32) -> i32 {
        self.item(item).map_or(0, |i| ((i.rotation + i.pending_turns) % 4) as i32)
    }

    fn info(&self, item: i32) -> QString {
        let Some(i) = self.item(item) else { return QString::default() };
        let dims = if i.width > 0 {
            format!("{} × {}", self.full_width(item), self.full_height(item))
        } else {
            "not imported".to_string()
        };
        let bytes = i.size as f64;
        let size = if bytes >= 1e6 { format!("{:.1} MB", bytes / 1e6) } else { format!("{:.0} KB", bytes / 1e3) };
        let kept = if i.rating > 0 && !i.rating_in_file { "   rating kept in the catalog" } else { "" };
        QString::from(&format!("{}   {dims}   {size}{kept}", i.path))
    }

    fn row_of(&self, item: i32) -> i32 {
        if self.item(item).is_none() || self.rows.is_empty() {
            return -1;
        }
        self.row_index(item as usize) as i32
    }

    fn neighbor(&self, item: i32, dx: i32, dy: i32) -> i32 {
        let n = self.items.len() as i32;
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
