//! The grid model: the files of the open folder, laid out in justified rows (`pg_core::layout`). QML sees one
//! list row per layout row (virtualized by ListView) and asks for each item's file id and size by index.

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
        type GalleryModel = super::GalleryRust;
    }

    unsafe extern "RustQt" {
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
        /// `filters::Filters`'. Returns where the file that was item `keep` is now: -1 if it isn't shown (or `keep`
        /// is -1).
        #[qinvokable]
        #[cxx_name = "openFolder"]
        fn open_folder(
            self: Pin<&mut GalleryModel>,
            merged: &QString,
            recursive: bool,
            query: &QString,
            sort: &QString,
            keep: i32,
        ) -> i32;

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

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QStringList, QUrl, QVariant};
use pg_core::browse::{self, Item};
use pg_core::layout::{self, Row};
use pg_core::media::{self, Kind};
use pg_core::query::{MediaType, Query, Sort};

use crate::{library, settings, state};

const ROLE_FIRST: i32 = 256;
const ROLE_COUNT: i32 = 257;
const ROLE_HEIGHT: i32 = 258;

#[derive(Default)]
pub struct GalleryRust {
    count: i32,
    revision: i32,
    items: Vec<Item>,
    rows: Vec<Row>,
    spacing: f64,
    width: f64,
    row_height: f64,
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
        keep: i32,
    ) -> i32 {
        let kept = self.file_id(keep);
        let path = merged.to_string();
        let (query, sort) = (Query::from_json(&query.to_string()), Sort::from_json(&sort.to_string()));
        let items = state::with_catalog(|c| browse::list_files(&c.conn, &path, recursive, &query, sort))
            .unwrap_or_else(|e| {
                eprintln!("listing {path}: {e:#}");
                Vec::new()
            });
        let count = items.len() as i32;
        self.as_mut().rust_mut().items = items;
        self.as_mut().set_count(count);
        let (w, h, s) = (self.width, self.row_height, self.spacing);
        self.as_mut().relayout(w, h, s);
        if kept < 0 { -1 } else { self.index_of(kept) }
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
        self.as_mut().rust_mut().items[item as usize].rating = rating;
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
