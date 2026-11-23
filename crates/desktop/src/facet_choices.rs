//! One list facet of the filter bar as a model: a row per choice (a media type, a shape, an import status, a borders
//! value, a rating, or an extension of the folder), with the choice's count (how many files choosing it would show,
//! `countFacets()`, or 0 where the bar shows none) and whether the query has it. `facet` is the list (a `ChoiceFacet`
//! QML names), which `pg_client::facets::choices` turns into the rows. The query and the counts are both read from
//! `controller.rs` (the counts as `Filters` last set them): `refresh` rebuilds the rows, and the filter bar calls it when
//! the query or the counts change.

pub(crate) mod state;
use state::Row;

use crate::bridges::facet_choices_bridge::qobject::{self, ChoiceFacet};

use core::pin::Pin;
use cxx_qt::CxxQtType;

use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use pg_client::facets::{Counts, choices};
use pg_client::input::key_of;
use pg_model::query::{ListFacet, Query};

use crate::controller;
use crate::rows::qt_int;

const ROLE_KEY: i32 = 256;
const ROLE_LABEL: i32 = 257;
const ROLE_COUNT: i32 = 258;
const ROLE_CHOSEN: i32 = 259;

/// Counts saturate: the properties are ints, as QML's are. (`Filters` shows its totals with the same function.)
pub(crate) fn count(n: i64) -> i32 {
    qt_int(n.max(0))
}

/// The list facet a QML `FacetChoices.ChoiceFacet` value names (the bridge's enum, compared by its value).
pub(crate) fn list_facet(kind: ChoiceFacet) -> Option<ListFacet> {
    [
        (ChoiceFacet::Types, ListFacet::Types),
        (ChoiceFacet::Shapes, ListFacet::Shapes),
        (ChoiceFacet::Status, ListFacet::Status),
        (ChoiceFacet::Borders, ListFacet::Borders),
        (ChoiceFacet::Ratings, ListFacet::Ratings),
        (ChoiceFacet::Exts, ListFacet::Exts),
    ]
    .into_iter()
    .find(|(value, _)| *value == kind)
    .map(|(_, facet)| facet)
}

/// The rows of `facet` for a query and counts.
fn rows(facet: ListFacet, query: &Query, counts: &Counts) -> Vec<Row> {
    choices(facet, counts)
        .into_iter()
        .map(|(choice, shown)| Row {
            chosen: query.has(&choice),
            label: pg_wording::choice_label(&choice),
            count: count(shown.unwrap_or(0)),
            key: key_of(&choice),
        })
        .collect()
}

/// The QML value of a list facet: the inverse of `list_facet`.
fn choice_of(facet: ListFacet) -> ChoiceFacet {
    match facet {
        ListFacet::Types => ChoiceFacet::Types,
        ListFacet::Shapes => ChoiceFacet::Shapes,
        ListFacet::Status => ChoiceFacet::Status,
        ListFacet::Borders => ChoiceFacet::Borders,
        ListFacet::Ratings => ChoiceFacet::Ratings,
        ListFacet::Exts => ChoiceFacet::Exts,
    }
}

impl qobject::FacetChoices {
    pub(crate) fn get_facet(&self) -> ChoiceFacet {
        choice_of(self.facet)
    }

    pub(crate) fn set_facet(mut self: Pin<&mut Self>, value: ChoiceFacet) {
        self.as_mut().rust_mut().facet = list_facet(value).unwrap_or(ListFacet::Types);
        self.refresh();
    }

    pub(crate) fn refresh(mut self: Pin<&mut Self>) {
        let query = controller::listing().query;
        let counts = controller::counts();
        let rows = rows(self.facet, &query, &counts);
        // A list whose rows are as they were is not reset: the chips it shows don't redraw for a count that didn't move.
        if rows == self.rows {
            return;
        }
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows = rows;
        self.as_mut().end_reset_model();
    }

    pub(crate) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
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

    pub(crate) fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [(ROLE_KEY, "key"), (ROLE_LABEL, "label"), (ROLE_COUNT, "count"), (ROLE_CHOSEN, "chosen")] {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    pub(crate) fn row_count(&self, _parent: &QModelIndex) -> i32 {
        i32::try_from(self.rows.len()).unwrap_or(i32::MAX)
    }
}
