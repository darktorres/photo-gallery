//! One list facet of the filter bar as a model: a row per choice (a media type, a shape, an import status, a borders
//! value, a rating, or an extension of the folder), with the choice's count (how many files choosing it would show,
//! `countFacets()`, or 0 where the bar shows none) and whether the query has it. `facet` is the list's index in
//! `pg_client::facets::Facet`, whose `choices` decides the rows. The query and the counts are both read from
//! `controller.rs` (the counts as `Filters` last set them): `refresh` rebuilds the rows, and the filter bar calls it when
//! the query or the counts change.

pub(crate) mod state;
use state::Row;

use crate::facet_choices_bridge::qobject;

use core::pin::Pin;
use cxx_qt::CxxQtType;

use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use pg_client::facets::{Counts, Facet, choices};
use pg_client::input::key_of;
use pg_client::text;
use pg_model::query::Query;

use crate::controller;

const ROLE_KEY: i32 = 256;
const ROLE_LABEL: i32 = 257;
const ROLE_COUNT: i32 = 258;
const ROLE_CHOSEN: i32 = 259;

/// Counts saturate: the properties are ints, as QML's are. (`Filters` shows its totals with the same function.)
pub(crate) fn count(n: i64) -> i32 {
    i32::try_from(n.max(0)).unwrap_or(i32::MAX)
}

/// The rows of `facet` for a query and counts.
fn rows(facet: Facet, query: &Query, counts: &Counts) -> Vec<Row> {
    choices(facet, counts)
        .into_iter()
        .map(|(choice, shown)| Row {
            chosen: query.has(&choice),
            label: text::choice_label(&choice),
            count: count(shown.unwrap_or(0)),
            key: key_of(&choice),
        })
        .collect()
}

impl qobject::FacetChoices {
    pub(crate) fn set_facet(mut self: Pin<&mut Self>, value: i32) {
        self.as_mut().rust_mut().facet = value;
        self.refresh();
    }

    pub(crate) fn refresh(mut self: Pin<&mut Self>) {
        let query = controller::listing().query;
        let counts = controller::counts();
        let facet = crate::rows::row_of(self.facet).and_then(Facet::from_index);
        let rows = facet.map_or_else(Vec::new, |facet| rows(facet, &query, &counts));
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
