//! One list facet of the filter bar as a model: a row per choice (a media type, a shape, an import status, a borders
//! value, a rating, or an extension of the folder), with the choice's count (how many files choosing it would show,
//! `countFacets()`, or 0 where the bar shows none) and whether the query has it. `facet` is the list's index in
//! `pg_client::facets::Facet`. The query is the listing's (`controller.rs`) and the counts are the ones `Filters` last
//! published (`set_counts`), both read here, typed: `refresh` rebuilds the rows, and the filter bar calls it when the
//! query or the counts change.

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
        /// Which list, by its index in `pg_client::facets::Facet` (the filter bar names them). Set it once.
        #[qproperty(i32, facet, READ, WRITE = set_facet, FINAL)]
        type FacetChoices = super::FacetChoicesRust;
    }

    unsafe extern "RustQt" {
        fn set_facet(self: Pin<&mut FacetChoices>, value: i32);

        /// Rebuild the rows from the listing's query and the counts (after a change to either).
        #[qinvokable]
        fn refresh(self: Pin<&mut FacetChoices>);

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
use std::cell::RefCell;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use pg_client::facets::Facet;
use pg_client::input::key_of;
use pg_client::text;
use pg_model::query::{BorderFacet, Choice, MediaType, Query, RatingFilter, Shape, Status};

use crate::controller;

const ROLE_KEY: i32 = 256;
const ROLE_LABEL: i32 = 257;
const ROLE_COUNT: i32 = 258;
const ROLE_CHOSEN: i32 = 259;

/// The counts `Filters` publishes for the list facets: a media type's and a borders value's, and the folder's
/// extensions in the order the bar shows them (most files first, then any chosen one it doesn't have, at 0).
#[derive(Clone, Debug, Default)]
pub struct Counts {
    pub types: Vec<(MediaType, i64)>,
    pub borders: Vec<(BorderFacet, i64)>,
    pub exts: Vec<(String, i64)>,
}

thread_local! {
    /// The counts `Filters` last published (`set_counts`); the GUI thread's only copy.
    static COUNTS: RefCell<Counts> = RefCell::new(Counts::default());
}

/// Publish the counts the list facets show (`Filters` does, when they are in or the query changed).
pub(crate) fn set_counts(counts: Counts) {
    COUNTS.with(|c| *c.borrow_mut() = counts);
}

fn counts() -> Counts {
    COUNTS.with(|c| c.borrow().clone())
}

/// One row: its key (the value the bar sends back, `key_of`), its label, its count (0 where none is shown), and whether
/// it's chosen.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    key: String,
    label: String,
    count: i32,
    chosen: bool,
}

#[derive(Default)]
pub struct FacetChoicesRust {
    /// The list's index in `Facet`; out of range (the default) shows no rows.
    facet: i32,
    rows: Vec<Row>,
}

/// The choices of a list facet, in the bar's order, each with its count (`None` where the bar shows none).
fn choices(facet: Facet, counts: &Counts) -> Vec<(Choice, Option<i64>)> {
    let media = |t: MediaType| counts.types.iter().find(|(x, _)| *x == t).map_or(0, |(_, n)| *n);
    let border = |b: BorderFacet| counts.borders.iter().find(|(x, _)| *x == b).map_or(0, |(_, n)| *n);
    match facet {
        Facet::Types => [MediaType::Image, MediaType::Animated, MediaType::Video]
            .map(|t| (Choice::Type(t), Some(media(t))))
            .to_vec(),
        Facet::Orientations => {
            [Shape::Landscape, Shape::Portrait, Shape::Square].map(|s| (Choice::Shape(s), None)).to_vec()
        }
        Facet::Status => {
            [Status::Imported, Status::Pending, Status::Failed].map(|s| (Choice::Status(s), None)).to_vec()
        }
        Facet::Borders => {
            [BorderFacet::None, BorderFacet::Black, BorderFacet::White, BorderFacet::Color, BorderFacet::Blur]
                .map(|b| (Choice::Border(b), Some(border(b))))
                .to_vec()
        }
        Facet::Ratings => {
            (0u8..6).filter_map(|n| RatingFilter::try_from(n).ok()).map(|r| (Choice::Rating(r), None)).collect()
        }
        Facet::Exts => counts.exts.iter().map(|(e, n)| (Choice::Ext(e.clone()), Some(*n))).collect(),
    }
}

/// The rows of `facet` for a query and counts.
fn rows(facet: Facet, query: &Query, counts: &Counts) -> Vec<Row> {
    choices(facet, counts)
        .into_iter()
        .map(|(choice, count)| Row {
            chosen: query.has(&choice),
            label: text::choice_label(&choice),
            count: crate::filters::count(count.unwrap_or(0)),
            key: key_of(&choice),
        })
        .collect()
}

impl qobject::FacetChoices {
    fn set_facet(mut self: Pin<&mut Self>, value: i32) {
        self.as_mut().rust_mut().facet = value;
        self.refresh();
    }

    fn refresh(mut self: Pin<&mut Self>) {
        let query = controller::listing().query;
        let counts = counts();
        let rows = Facet::from_index(self.facet).map_or_else(Vec::new, |facet| rows(facet, &query, &counts));
        // A list whose rows are as they were is not reset: the chips it shows don't redraw for a count that didn't move.
        if rows == self.rows {
            return;
        }
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
