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

use pg_client::facets::Facet;
use pg_client::input::{Edge, Field, bound_is_valid, choice};
use pg_client::text;

use crate::controller;
use crate::facet_choices::{Counts, count};

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

/// The duplicates box's index for a query's choice: 0 either way, 1 with a duplicate, 2 without one.
fn duplicates_index(d: Option<bool>) -> i32 {
    match d {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    }
}

/// The review box's index for a query's choice: 0 either way, 1 borders not marked, 2 borders marked.
fn review_index(marked: Option<bool>) -> i32 {
    match marked {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }
}

impl qobject::Filters {
    /// After the query changed: the properties that show it, and the counts the choices show.
    fn sync(mut self: Pin<&mut Self>) {
        let q = controller::listing().query;
        let (name, duplicates_index, border_marked_index, description) = (
            QString::from(&q.name),
            duplicates_index(q.duplicates),
            review_index(q.border_marked),
            QString::from(&text::describe(&q)),
        );
        publish_counts(&self.facets, &q);
        if self.name != name {
            self.as_mut().rust_mut().name = name;
            self.as_mut().name_changed();
        }
        if self.duplicates_index != duplicates_index {
            self.as_mut().rust_mut().duplicates_index = duplicates_index;
            self.as_mut().duplicates_index_changed();
        }
        if self.border_marked_index != border_marked_index {
            self.as_mut().rust_mut().border_marked_index = border_marked_index;
            self.as_mut().border_marked_index_changed();
        }
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

    pub(crate) fn set_name_edited(self: Pin<&mut Self>, value: &QString) {
        let name = value.to_string().trim().to_string();
        if name != controller::listing().query.name {
            self.edit(|q| q.name = name);
        }
    }

    pub(crate) fn set_duplicates_index(self: Pin<&mut Self>, index: i32) {
        let duplicates = match index {
            1 => Some(true),
            2 => Some(false),
            _ => None,
        };
        if duplicates != controller::listing().query.duplicates {
            self.edit(|q| q.duplicates = duplicates);
        }
    }

    pub(crate) fn set_border_marked_index(self: Pin<&mut Self>, index: i32) {
        let marked = match index {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        };
        if marked != controller::listing().query.border_marked {
            self.edit(|q| q.border_marked = marked);
        }
    }

    pub(crate) fn toggle(self: Pin<&mut Self>, facet: i32, value: &QString) {
        let value = value.to_string();
        if let Some(choice) = Facet::from_index(facet).and_then(|facet| choice(facet, &value)) {
            self.edit(|q| q.toggle(choice));
        }
    }

    pub(crate) fn bound(&self, facet: i32, end: i32) -> QString {
        let query = controller::listing().query;
        let text = match (Field::from_index(facet), Edge::from_index(end)) {
            (Some(Field::Range(f)), Some(e)) => query.number(f, e.number()).map(|v| v.to_string()),
            (Some(Field::Day(f)), Some(e)) => query.day(f, e.day()).map(|d| d.to_string()),
            _ => None,
        };
        QString::from(&text.unwrap_or_default())
    }

    pub(crate) fn set_bound(self: Pin<&mut Self>, facet: i32, end: i32, text: &QString) {
        let text = text.to_string();
        match (Field::from_index(facet), Edge::from_index(end)) {
            (Some(Field::Range(f)), Some(e)) => self.edit(|q| q.set_number(f, e.number(), parse_number(&text))),
            (Some(Field::Day(f)), Some(e)) => self.edit(|q| q.set_day(f, e.day(), Day::parse(&text))),
            _ => {}
        }
    }

    pub(crate) fn bound_valid(&self, facet: i32, text: &QString) -> bool {
        Field::from_index(facet).is_some_and(|field| bound_is_valid(field, &text.to_string()))
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
            let facets = facets.unwrap_or_else(|e| {
                pg_log::warn("filters", &format!("facets of {shown}: {e:#}"));
                Facets::default()
            });
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

/// Publish what the list facets show: a media type's and a border's count, and the folder's extensions (most files
/// first, then the chosen ones it doesn't have, at 0, so they can be unchosen).
fn publish_counts(facets: &Facets, q: &Query) {
    let mut exts: Vec<(String, i64)> = facets.exts.clone();
    exts.extend(q.exts.iter().filter(|e| !facets.exts.iter().any(|(x, _)| x == *e)).map(|e| (e.clone(), 0)));
    crate::facet_choices::set_counts(Counts { types: facets.types.clone(), borders: facets.borders.clone(), exts });
}
