//! The filter bar's query (PLAN §7): the files a `query::Query` keeps, edited through typed calls
//! (`pg_model::query`'s editing API), so the QML never builds or reads it, and the counts next to the choices
//! (`browse::read::facets`). The query is the listing's (`controller.rs`): this object edits it and shows its properties,
//! and the list facets' rows are `FacetChoices` (facet_choices.rs), which read the query and the counts directly. FilterBar.qml
//! makes one and is the only QML that calls into it.

pub(crate) mod state;
use state::FiltersRust;

use crate::filters_bridge::qobject;

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use pg_app::reexport::Facets;
use pg_model::date::Day;
use pg_model::query::{Query, parse_number};

use pg_client::facets::{Counts, Facet};
use pg_client::input::{Edge, Field, bound_is_valid, choice, tri_index, tri_of_index};
use pg_client::text;

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
            description: QString::from(&text::describe(&q)),
            facets: Facets::default(),
            generation: 0,
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
        publish_counts(&self.facets, &q);
        // The generated setters notify only on a change.
        self.as_mut().set_name(QString::from(&q.name));
        self.as_mut().set_duplicates_index(duplicates_index(q.duplicates));
        self.as_mut().set_border_marked_index(review_index(q.border_marked));
        let description = QString::from(&text::describe(&q));
        self.as_mut().set_active(i32::try_from(q.active()).unwrap_or(i32::MAX));
        self.as_mut().set_description(description);
    }

    /// The user changed the query: the listing takes it (its policy, `listing::dispatch`), then the bar shows it and
    /// `edited` asks for the list after its pause. The session is saved when that list is shown, not per key.
    fn edit(mut self: Pin<&mut Self>, change: impl FnOnce(&mut Query)) {
        let mut query = controller::listing().query;
        change(&mut query);
        self.as_mut().set_query(query);
    }

    fn set_query(mut self: Pin<&mut Self>, query: Query) {
        controller::query_changed(query);
        self.as_mut().sync();
        self.edited();
    }

    pub(crate) fn reload(mut self: Pin<&mut Self>) {
        self.as_mut().sync();
        self.loaded();
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
        if let Some(choice) = row_of(facet).and_then(Facet::from_index).and_then(|facet| choice(facet, &value)) {
            self.edit(|q| q.toggle(choice));
        }
    }

    pub(crate) fn bound(&self, facet: i32, end: i32) -> QString {
        let query = controller::listing().query;
        let text = match (row_of(facet).and_then(Field::from_index), row_of(end).and_then(Edge::from_index)) {
            (Some(Field::Range(f)), Some(e)) => query.number(f, e.number()).map(|v| v.to_string()),
            (Some(Field::Day(f)), Some(e)) => query.day(f, e.day()).map(|d| d.to_string()),
            _ => None,
        };
        QString::from(&text.unwrap_or_default())
    }

    pub(crate) fn set_bound(self: Pin<&mut Self>, facet: i32, end: i32, text: &QString) {
        let text = text.to_string();
        match (row_of(facet).and_then(Field::from_index), row_of(end).and_then(Edge::from_index)) {
            (Some(Field::Range(f)), Some(e)) => self.edit(|q| q.set_number(f, e.number(), parse_number(&text))),
            (Some(Field::Day(f)), Some(e)) => self.edit(|q| q.set_day(f, e.day(), Day::parse(&text))),
            _ => {}
        }
    }

    pub(crate) fn bound_valid(&self, facet: i32, text: &QString) -> bool {
        row_of(facet).and_then(Field::from_index).is_some_and(|field| bound_is_valid(field, &text.to_string()))
    }

    pub(crate) fn clear(mut self: Pin<&mut Self>) {
        controller::query_changed(Query::default());
        self.as_mut().sync();
        self.as_mut().loaded();
        self.edited();
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
        crate::app::get().reading().session().facets(listing.folder, listing.recursive, listing.query, move |facets| {
            let facets = match facets {
                Ok(facets) => {
                    crate::app::resolved("facets-failed");
                    facets
                }
                Err(e) => {
                    let error = crate::app::error_text(&format!("{shown}: {e:#}"));
                    crate::app::post(pg_model::notice::Notice::FacetsFailed { error });
                    Facets::default()
                }
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
        self.counted();
    }
}

/// Publish what the list facets show (`Counts::new`): the catalog's counts for the query.
fn publish_counts(facets: &Facets, q: &Query) {
    controller::set_counts(Counts::new(facets.types.clone(), facets.borders.clone(), &facets.exts, q));
}
