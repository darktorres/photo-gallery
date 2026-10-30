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
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qproperty(i32, count)]
        #[qproperty(i32, revision)]
        #[qproperty(QString, folder)]
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

        /// Show the files of a merged folder (`""` = everything), with or without its subfolders.
        #[qinvokable]
        #[cxx_name = "openFolder"]
        fn open_folder(self: Pin<&mut GalleryModel>, merged: &QString, recursive: bool);

        /// Recompute the rows for a viewport width and a target row height (the runtime thumbnail size).
        #[qinvokable]
        fn relayout(self: Pin<&mut GalleryModel>, width: f64, row_height: f64, spacing: f64);

        #[qinvokable]
        #[cxx_name = "fileId"]
        fn file_id(self: &GalleryModel, item: i32) -> i64;

        /// Width ÷ height, as laid out.
        #[qinvokable]
        fn aspect(self: &GalleryModel, item: i32) -> f64;

        #[qinvokable]
        fn rating(self: &GalleryModel, item: i32) -> i32;

        #[qinvokable]
        #[cxx_name = "setRating"]
        fn set_rating(self: Pin<&mut GalleryModel>, item: i32, rating: i32);

        #[qinvokable]
        fn path(self: &GalleryModel, item: i32) -> QString;

        /// One line for the status bar: name, size, rating.
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
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use pg_core::browse::{self, Item};
use pg_core::layout::{self, Row};

use crate::state;

const ROLE_FIRST: i32 = 256;
const ROLE_COUNT: i32 = 257;
const ROLE_HEIGHT: i32 = 258;

#[derive(Default)]
pub struct GalleryRust {
    count: i32,
    revision: i32,
    folder: QString,
    items: Vec<Item>,
    rows: Vec<Row>,
    spacing: f64,
    width: f64,
    row_height: f64,
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

    fn open_folder(mut self: Pin<&mut Self>, merged: &QString, recursive: bool) {
        let path = merged.to_string();
        let items = state::with_catalog(|c| browse::list_files(&c.conn, &path, recursive)).unwrap_or_else(|e| {
            eprintln!("listing {path}: {e:#}");
            Vec::new()
        });
        let count = items.len() as i32;
        self.as_mut().rust_mut().items = items;
        self.as_mut().set_count(count);
        self.as_mut().set_folder(merged.clone());
        let (w, h, s) = (self.width, self.row_height, self.spacing);
        self.relayout(w, h, s);
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

    fn aspect(&self, item: i32) -> f64 {
        self.item(item).map_or(1.0, |i| layout::clamp_aspect(i.aspect()))
    }

    fn rating(&self, item: i32) -> i32 {
        self.item(item).map_or(0, |i| i.rating as i32)
    }

    fn set_rating(mut self: Pin<&mut Self>, item: i32, rating: i32) {
        let Some(id) = self.item(item).map(|i| i.id) else { return };
        let rating = rating.clamp(0, 5) as u8;
        if let Err(e) = state::with_catalog(|c| browse::set_rating(&c.conn, id, rating)) {
            eprintln!("rating: {e:#}");
            return;
        }
        self.as_mut().rust_mut().items[item as usize].rating = rating;
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn path(&self, item: i32) -> QString {
        self.item(item).map_or_else(QString::default, |i| QString::from(&i.path))
    }

    fn info(&self, item: i32) -> QString {
        let Some(i) = self.item(item) else { return QString::default() };
        let size = if i.width > 0 { format!("{} × {}", i.width, i.height) } else { "not imported".to_string() };
        let stars = "★".repeat(i.rating as usize);
        QString::from(&format!("{}   {size}   {stars}", i.path))
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
