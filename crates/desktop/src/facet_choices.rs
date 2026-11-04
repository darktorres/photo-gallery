//! One list facet of the filter bar as a model: a row per choice (a media type, a shape, an import status, a borders
//! value, a rating, or an extension of the folder), with the choice's count (how many files choosing it would show,
//! `countFacets()`, or 0 where the bar shows none) and whether the query has it. `facet` names the list; `query` (the
//! query as JSON) and `counts` (`Counts`, from `Filters`) are set from the filter bar's properties, and each change
//! rebuilds the rows. So a chip's bindings read the rows' roles, and nothing depends on a revision number.

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
        /// Which list: `types`, `orientations`, `status`, `borders`, `ratings` or `exts`. Set it once.
        #[qproperty(QString, facet, READ, WRITE = set_facet, FINAL)]
        /// The query as JSON (`Filters.query`). Setting it rebuilds the rows.
        #[qproperty(QString, query, READ, WRITE = set_query, FINAL)]
        /// The counts as JSON (`Filters.counts`). Setting it rebuilds the rows.
        #[qproperty(QString, counts, READ, WRITE = set_counts, FINAL)]
        type FacetChoices = super::FacetChoicesRust;
    }

    unsafe extern "RustQt" {
        fn set_facet(self: Pin<&mut FacetChoices>, value: &QString);
        fn set_query(self: Pin<&mut FacetChoices>, value: &QString);
        fn set_counts(self: Pin<&mut FacetChoices>, value: &QString);

        #[qinvokable]
        #[cxx_override]
        fn data(self: &FacetChoices, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &FacetChoices) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &FacetChoices, parent: &QModelIndex) -> i32;

        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut FacetChoices>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut FacetChoices>);
    }
}

use core::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use pg_catalog::query::from_json;
use pg_client::input::choice;
use pg_client::text;
use pg_model::query::{BorderFacet, MediaType, Query, Shape, Status};
use serde::{Deserialize, Serialize};

const ROLE_KEY: i32 = 256;
const ROLE_LABEL: i32 = 257;
const ROLE_COUNT: i32 = 258;
const ROLE_CHOSEN: i32 = 259;

/// The counts `Filters` publishes for the list facets: a media type's and a borders value's (by key), and the
/// folder's extensions in the order the bar shows them (most files first, then any chosen one it doesn't have, at 0).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Counts {
    pub types: Vec<(String, i64)>,
    pub borders: Vec<(String, i64)>,
    pub exts: Vec<(String, i64)>,
}

/// One row: its key (the value `choice` reads), its label, its count (0 where none is shown), and whether it's chosen.
#[derive(Clone, Debug)]
struct Row {
    key: String,
    label: String,
    count: i32,
    chosen: bool,
}

#[derive(Default)]
pub struct FacetChoicesRust {
    facet: QString,
    query: QString,
    counts: QString,
    rows: Vec<Row>,
}

/// The choices of a list facet, in the bar's order, each with its count (`None` where the bar shows none).
fn choices(facet: &str, counts: &Counts) -> Vec<(String, Option<i64>)> {
    let tally = |list: &[(String, i64)], key: &str| list.iter().find(|(k, _)| k == key).map_or(0, |(_, n)| *n);
    match facet {
        "types" => [MediaType::Image, MediaType::Animated, MediaType::Video]
            .map(|t| (t.key().to_string(), Some(tally(&counts.types, t.key()))))
            .to_vec(),
        "orientations" => {
            [Shape::Landscape, Shape::Portrait, Shape::Square].map(|s| (s.key().to_string(), None)).to_vec()
        }
        "status" => [Status::Imported, Status::Pending, Status::Failed].map(|s| (s.key().to_string(), None)).to_vec(),
        "borders" => [BorderFacet::None, BorderFacet::Black, BorderFacet::White, BorderFacet::Color, BorderFacet::Blur]
            .map(|b| (b.key().to_string(), Some(tally(&counts.borders, b.key()))))
            .to_vec(),
        "ratings" => (0..6).map(|n| (n.to_string(), None)).collect(),
        "exts" => counts.exts.iter().map(|(e, n)| (e.clone(), Some(*n))).collect(),
        _ => Vec::new(),
    }
}

/// The rows of `facet` for a query (JSON) and counts (JSON).
fn rows(facet: &str, query: &str, counts: &str) -> Vec<Row> {
    let query: Query = from_json(query);
    let counts: Counts = serde_json::from_str(counts).unwrap_or_default();
    choices(facet, &counts)
        .into_iter()
        .map(|(key, count)| Row {
            chosen: choice(facet, &key).is_some_and(|c| query.has(&c)),
            label: text::facet_label(facet, &key),
            count: crate::filters::count(count.unwrap_or(0)),
            key,
        })
        .collect()
}

impl qobject::FacetChoices {
    fn set_facet(mut self: Pin<&mut Self>, value: &QString) {
        self.as_mut().rust_mut().facet = value.clone();
        self.refresh();
    }

    fn set_query(mut self: Pin<&mut Self>, value: &QString) {
        self.as_mut().rust_mut().query = value.clone();
        self.refresh();
    }

    fn set_counts(mut self: Pin<&mut Self>, value: &QString) {
        self.as_mut().rust_mut().counts = value.clone();
        self.refresh();
    }

    fn refresh(mut self: Pin<&mut Self>) {
        let rows = rows(&self.facet.to_string(), &self.query.to_string(), &self.counts.to_string());
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows = rows;
        self.as_mut().end_reset_model();
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = usize::try_from(index.row()).ok().and_then(|i| self.rows.get(i)) else {
            return QVariant::default();
        };
        match role {
            ROLE_KEY => QVariant::from(&QString::from(&row.key)),
            ROLE_LABEL => QVariant::from(&QString::from(&row.label)),
            ROLE_COUNT => QVariant::from(&row.count),
            ROLE_CHOSEN => QVariant::from(&row.chosen),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [(ROLE_KEY, "key"), (ROLE_LABEL, "label"), (ROLE_COUNT, "count"), (ROLE_CHOSEN, "chosen")] {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }
}
