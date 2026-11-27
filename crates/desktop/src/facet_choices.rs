//! One list facet of the filter bar as a model: a row per choice (a media type, a shape, an import status, a borders
//! value, a rating, or an extension of the folder), with the choice's count (how many files choosing it would show,
//! `countFacets()`, or 0 where the bar shows none) and whether the query has it. `facet` is the list (a `ChoiceFacet`
//! QML names), which `pg_client::facets::choices` turns into the rows. The query and the counts are both read from
//! `controller.rs` (the counts as `Filters` last set them): `refresh` rebuilds the rows, and the filter bar calls it when
//! the query or the counts change.
use crate::bridges::facet_choices_state::Row;

use crate::bridges::facet_choices_bridge::qobject::{self, ChoiceFacet};

use core::pin::Pin;
use cxx_qt::CxxQtType;

use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use pg_client::facets::{Counts, choices};
use pg_client::input::key_of;
use pg_model::query::{ListFacet, Query};

use crate::controller;
use crate::rows::qt_int;

const ROLE_KEY: i32 = 256;
const ROLE_LABEL: i32 = 257;
const ROLE_COUNT: i32 = 258;
const ROLE_CHOSEN: i32 = 259;
const ROLE_PENDING: i32 = 260;

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

/// The rows of `facet` for a query and counts. With no counts yet (`None`: the folder's are being read), a count that is
/// shown is pending, not zero.
fn rows(facet: ListFacet, query: &Query, counts: Option<&Counts>) -> Vec<Row> {
    let empty = Counts::default();
    let pending = counts.is_none();
    choices(facet, counts.unwrap_or(&empty))
        .into_iter()
        .map(|(choice, shown)| Row {
            chosen: query.has(&choice),
            label: pg_wording::choice_label(&choice),
            count: count(shown.unwrap_or(0)),
            pending: pending && shown.is_some(),
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
        let rows = rows(self.facet, &query, counts.as_ref());
        // A list whose rows are as they were is not reset: the chips it shows don't redraw for a count that didn't move.
        if rows == self.rows {
            return;
        }
        // The same choices in the same order (only a count or a chosen mark moved): the rows are signalled as changed, not
        // reset. A reset destroys the chip delegates, and with them the chip that has the keyboard focus.
        let same_choices = rows.len() == self.rows.len()
            && rows.iter().zip(&self.rows).all(|(new, old)| new.key == old.key && new.label == old.label);
        if same_choices {
            let last = rows.len().checked_sub(1);
            self.as_mut().rust_mut().rows = rows;
            if let Some(last) = last {
                let root = QModelIndex::default();
                let (top, bottom) = (self.index(0, 0, &root), self.index(qt_int(last), 0, &root));
                let roles = [ROLE_COUNT, ROLE_CHOSEN, ROLE_PENDING];
                self.as_mut().data_changed(&top, &bottom, &QList::from(roles.as_slice()));
            }
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
            ROLE_PENDING => QVariant::from(&row.pending),
            _ => QVariant::default(),
        }
    }

    pub(crate) fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [
            (ROLE_KEY, "key"),
            (ROLE_LABEL, "label"),
            (ROLE_COUNT, "count"),
            (ROLE_CHOSEN, "chosen"),
            (ROLE_PENDING, "pending"),
        ] {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    pub(crate) fn row_count(&self, _parent: &QModelIndex) -> i32 {
        i32::try_from(self.rows.len()).unwrap_or(i32::MAX)
    }
}
