//! The filter bar's query: the files a `query::Query` keeps, edited through typed calls
//! (`pg_model::query`'s editing API), so the QML never builds or reads it, and the counts next to the choices
//! (`browse::read::facets`). The query is the listing's (`controller.rs`): this object edits it and shows its properties,
//! and the list facets' rows are `FacetChoices` (facet_choices.rs), which read the query and the counts directly. FilterBar.qml
//! makes one and is the only QML that calls into it.
use crate::bridges::filters_state::FiltersRust;

use crate::bridges::filters_bridge::qobject::{self, ListFacetKind};

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use pg_client::input::parse_number;
use pg_model::date::Day;
use pg_model::facets::Facets;
use pg_model::query::{ListFacet, Query};

use pg_client::facets::Counts;
use pg_client::input::{Edge, Field, bound_is_valid, choice, tri_index, tri_of_index};

use crate::controller;
use crate::facet_choices::count;
use crate::rows::{row_of, to_row};

impl Default for FiltersRust {
    /// The query the window starts with (the listing's).
    fn default() -> FiltersRust {
        let q = controller::listing().query;
        FiltersRust {
            active: i32::try_from(q.active()).unwrap_or(i32::MAX),
            name: QString::from(&q.name),
            duplicates_index: duplicates_index(q.duplicates),
            border_marked_index: review_index(q.border_marked),
            total: 0,
            matching: 0,
            counts_pending: true,
            description: QString::from(&pg_wording::describe(&q)),
            facets: Facets::default(),
            generation: 0,
            bar_open: false,
        }
    }
}

/// The duplicates box lists "with a duplicate" first.
const DUPLICATES_FIRST: bool = true;
/// The review box lists "borders not marked" first.
const REVIEW_FIRST: bool = false;

/// The duplicates box's index for a query's choice: 0 either way, 1 with a duplicate, 2 without one.
fn duplicates_index(d: Option<bool>) -> i32 {
    to_row(Some(tri_index(d, DUPLICATES_FIRST)))
}

/// The review box's index for a query's choice: 0 either way, 1 borders not marked, 2 borders marked.
fn review_index(marked: Option<bool>) -> i32 {
    to_row(Some(tri_index(marked, REVIEW_FIRST)))
}

impl qobject::Filters {
    /// After the query changed: the properties that show it, and the counts the choices show.
    fn sync(mut self: Pin<&mut Self>) {
        let q = controller::listing().query;
        // While the counts of the folder shown are pending (`reload`), the facets' counts wait for the new ones
        // (`apply_facets`), which are for the query then shown.
        if !self.counts_pending {
            publish_counts(&self.facets, &q);
        }
        // The generated setters notify only on a change.
        self.as_mut().set_name(QString::from(&q.name));
        self.as_mut().set_duplicates_index(duplicates_index(q.duplicates));
        self.as_mut().set_border_marked_index(review_index(q.border_marked));
        let description = QString::from(&pg_wording::describe(&q));
        self.as_mut().set_active(i32::try_from(q.active()).unwrap_or(i32::MAX));
        self.as_mut().set_description(description);
    }

    /// The user changed the query: the listing takes it (its policy, `listing::dispatch`), then the bar shows it and
    /// `edited` asks for the list after its pause. The session is saved when that list is shown, not per key.
    fn edit(mut self: Pin<&mut Self>, change: impl FnOnce(&mut Query)) {
        let before = controller::listing().query;
        let mut query = before.clone();
        change(&mut query);
        // A change that leaves the query as it was (a bound typed to the value it had) asks for nothing: no new list.
        if query != before {
            self.as_mut().set_query(query);
        }
    }

    fn set_query(mut self: Pin<&mut Self>, query: Query) {
        controller::query_changed(query);
        self.as_mut().sync();
        self.edited();
    }

    pub(crate) fn reload(mut self: Pin<&mut Self>) {
        // The folder or the query may have changed: the counts shown are pending until the new ones are in, and a count
        // asked for before this is dropped (its generation is older), so it can't stand for the new listing.
        self.as_mut().rust_mut().generation += 1;
        self.as_mut().set_counts_pending(true);
        controller::set_counts(None);
        self.as_mut().sync();
        self.as_mut().loaded();
        self.as_mut().count_if_needed();
    }

    pub(crate) fn edit_name(self: Pin<&mut Self>, value: &QString) {
        let name = value.to_string().trim().to_string();
        if name != controller::listing().query.name {
            self.edit(|q| q.name = name);
        }
    }

    pub(crate) fn choose_duplicates(self: Pin<&mut Self>, index: i32) {
        let duplicates = row_of(index).and_then(|i| tri_of_index(i, DUPLICATES_FIRST));
        if duplicates != controller::listing().query.duplicates {
            self.edit(|q| q.duplicates = duplicates);
        }
    }

    pub(crate) fn choose_border_marked(self: Pin<&mut Self>, index: i32) {
        let marked = row_of(index).and_then(|i| tri_of_index(i, REVIEW_FIRST));
        if marked != controller::listing().query.border_marked {
            self.edit(|q| q.border_marked = marked);
        }
    }

    pub(crate) fn toggle(self: Pin<&mut Self>, facet: i32, value: &QString) {
        let value = value.to_string();
        if let Some(choice) = kind_of(facet).and_then(list_facet).and_then(|facet| choice(facet, &value)) {
            self.edit(|q| q.toggle(choice));
        }
    }

    /// `field` is the bound's field's index in `input::Field::ALL` (not a list facet).
    pub(crate) fn bound(&self, field: i32, end: i32) -> QString {
        let query = controller::listing().query;
        let text = match (row_of(field).and_then(Field::from_index), row_of(end).and_then(Edge::from_index)) {
            (Some(Field::Range(f)), Some(e)) => query.number(f, e.number()).map(|v| v.to_string()),
            (Some(Field::Day(f)), Some(e)) => query.day(f, e.day()).map(|d| d.to_string()),
            _ => None,
        };
        QString::from(&text.unwrap_or_default())
    }

    /// A bound typed as text: an empty field is no bound; text that doesn't parse (mid-typing, "5x") keeps the bound it
    /// had, so the list is not widened by it (the field shows the text in red, `bound_valid`).
    pub(crate) fn set_bound(self: Pin<&mut Self>, field: i32, end: i32, text: &QString) {
        let text = text.to_string();
        let empty = text.trim().is_empty();
        match (row_of(field).and_then(Field::from_index), row_of(end).and_then(Edge::from_index)) {
            (Some(Field::Range(f)), Some(e)) => {
                let value = parse_number(&text);
                if empty || value.is_some() {
                    self.edit(|q| q.set_number(f, e.number(), value));
                }
            }
            (Some(Field::Day(f)), Some(e)) => {
                let value = Day::parse(&text);
                if empty || value.is_some() {
                    self.edit(|q| q.set_day(f, e.day(), value));
                }
            }
            _ => {}
        }
    }

    pub(crate) fn bound_valid(&self, field: i32, text: &QString) -> bool {
        row_of(field).and_then(Field::from_index).is_some_and(|f| bound_is_valid(f, &text.to_string()))
    }

    pub(crate) fn clear(mut self: Pin<&mut Self>) {
        // A count in flight is for the query being cleared: it is not taken for the cleared one.
        self.as_mut().rust_mut().generation += 1;
        controller::query_changed(Query::default());
        self.as_mut().sync();
        self.as_mut().loaded();
        self.edited();
    }

    /// The filter bar opened or closed (the window reports it). Opening it counts the choices.
    pub(crate) fn set_bar_open(mut self: Pin<&mut Self>, open: bool) {
        if self.bar_open != open {
            self.as_mut().rust_mut().bar_open = open;
            if open {
                self.count_facets();
            }
        }
    }

    /// The counts are asked for while the bar is open or a filter is set; otherwise they are not computed, since nothing
    /// shows them.
    pub(crate) fn count_if_needed(self: Pin<&mut Self>) {
        if self.bar_open || self.active > 0 {
            self.count_facets();
        }
    }

    pub(crate) fn count_facets(mut self: Pin<&mut Self>) {
        let generation = {
            let mut this = self.as_mut().rust_mut();
            this.generation += 1;
            this.generation
        };
        let listing = controller::listing();
        let filters = self.qt_thread();
        let shown = pg_platform::native::PATH_RULES.display_merged(&listing.folder);
        crate::app::session().facets(listing.folder, listing.recursive, listing.query, move |facets| {
            // A count that couldn't be read is not shown as one: the bar keeps the counts it has (the inbox says why).
            let Some(facets) = crate::app::observe(pg_model::notice::Op::Facets, facets, |e| {
                pg_model::notice::Notice::FacetsFailed { error: crate::app::error_text(&format!("{shown}: {e:#}")) }
            }) else {
                return;
            };
            // Fails only when the filter bar is gone (the window closing).
            let _ = filters.queue(move |filters| filters.apply_facets(generation, facets));
        });
    }

    /// The counts arrived: publish them and the totals. The query's own properties are unchanged, so they are not
    /// set again (each set would notify the bar's bindings for nothing).
    fn apply_facets(mut self: Pin<&mut Self>, generation: u64, facets: Facets) {
        if generation != self.generation {
            return;
        }
        let (total, matching) = (count(facets.total), count(facets.matching));
        publish_counts(&facets, &controller::listing().query);
        self.as_mut().rust_mut().facets = facets;
        self.as_mut().set_total(total);
        self.as_mut().set_matching(matching);
        self.as_mut().set_counts_pending(false);
        self.counted();
    }
}

/// Publish what the list facets show (`Counts::new`): the catalog's counts for the query.
fn publish_counts(facets: &Facets, q: &Query) {
    controller::set_counts(Some(Counts::new(facets.types.clone(), facets.borders.clone(), &facets.exts, q)));
}

/// The `ListFacetKind` a QML `Filters.ListFacetKind` value names: the int of the enum's value, in declaration order.
fn kind_of(value: i32) -> Option<ListFacetKind> {
    [
        ListFacetKind::Types,
        ListFacetKind::Shapes,
        ListFacetKind::Status,
        ListFacetKind::Borders,
        ListFacetKind::Ratings,
        ListFacetKind::Exts,
    ]
    .into_iter()
    .nth(usize::try_from(value).ok()?)
}

/// The list facet a QML `Filters.ListFacetKind` value names (the bridge's enum, compared by its value).
fn list_facet(kind: ListFacetKind) -> Option<ListFacet> {
    [
        (ListFacetKind::Types, ListFacet::Types),
        (ListFacetKind::Shapes, ListFacet::Shapes),
        (ListFacetKind::Status, ListFacet::Status),
        (ListFacetKind::Borders, ListFacet::Borders),
        (ListFacetKind::Ratings, ListFacet::Ratings),
        (ListFacetKind::Exts, ListFacet::Exts),
    ]
    .into_iter()
    .find(|(value, _)| *value == kind)
    .map(|(_, facet)| facet)
}
