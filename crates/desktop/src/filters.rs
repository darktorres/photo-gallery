//! The filter bar's query (PLAN §7): the files a `query::Query` keeps, edited through typed calls
//! (`pg_catalog::query`'s editing API), so the QML never builds or reads it, and the counts next to the choices
//! (`browse::read::facets`). FilterBar.qml makes one and is the only QML that calls into it: the window reads the query as
//! JSON (`query`, for the grid's model and the saved views) and sets it (a saved view, the session) through the
//! filter bar's properties.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// The query as JSON (`query::Query`). Setting it replaces the query (fields showing typed text reload).
        #[qproperty(QString, query, READ, WRITE = set_query_loaded, NOTIFY, FINAL)]
        /// How many facets filter.
        #[qproperty(i32, active, READ, WRITE, NOTIFY, FINAL)]
        /// Changes whenever the query or the counts do: for bindings that call `has()` or the counts.
        #[qproperty(i32, revision, READ, WRITE, NOTIFY, FINAL)]
        /// Part of the file name, any case; `""` doesn't filter.
        #[qproperty(QString, name, READ, WRITE = set_name_edited, NOTIFY, FINAL)]
        /// -1 either way; 1 files with a duplicate; 0 imported files without one.
        #[qproperty(i32, duplicates, READ, WRITE = set_duplicates_edited, NOTIFY, FINAL)]
        /// The borders review: -1 either way; 1 borders marked as not a border; 0 borders found and not marked.
        #[qproperty(
            i32, border_marked, cxx_name = "borderMarked", READ, WRITE = set_border_marked_edited, NOTIFY, FINAL
        )]
        /// Files in the folder, unfiltered, and those the query keeps (`countFacets()`).
        #[qproperty(i32, total, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, matching, READ, WRITE, NOTIFY, FINAL)]
        /// The folder's extensions, most files first, then any chosen that it doesn't have (so they can be
        /// unchosen).
        #[qproperty(QStringList, extensions, READ, WRITE, NOTIFY, FINAL)]
        /// One line saying what the query keeps, for the save-view dialog.
        #[qproperty(QString, description, READ, WRITE, NOTIFY, FINAL)]
        type Filters = super::FiltersRust;
    }

    impl cxx_qt::Threading for Filters {}

    unsafe extern "RustQt" {
        /// The counts `countFacets` asked for are in.
        #[qsignal]
        fn counted(self: Pin<&mut Filters>);

        fn set_query_loaded(self: Pin<&mut Filters>, value: &QString);
        fn set_name_edited(self: Pin<&mut Filters>, value: &QString);
        fn set_duplicates_edited(self: Pin<&mut Filters>, value: i32);
        fn set_border_marked_edited(self: Pin<&mut Filters>, value: i32);

        /// The user changed the query.
        #[qsignal]
        fn edited(self: Pin<&mut Filters>);

        /// The query was replaced (`query` set, Clear): fields showing typed text reload it.
        #[qsignal]
        fn loaded(self: Pin<&mut Filters>);

        /// Whether a value of a list facet is chosen (`types`, `exts`, `ratings`, `orientations`, `status`,
        /// `borders`).
        #[qinvokable]
        fn has(self: &Filters, facet: &QString, value: &QString) -> bool;

        /// Choose a value of a list facet, or unchoose it.
        #[qinvokable]
        fn toggle(self: Pin<&mut Filters>, facet: &QString, value: &QString);

        /// How the filter bar names a value of a list facet.
        #[qinvokable]
        fn label(self: &Filters, facet: &QString, value: &QString) -> QString;

        /// One end of a range facet (`size_mb`, `megapixels`, `duration_s`: `min`/`max`; `taken`, `modified`:
        /// `from`/`to`) as text, `""` when open.
        #[qinvokable]
        fn bound(self: &Filters, facet: &QString, end: &QString) -> QString;

        /// Set one end of a range facet from what was typed; text that doesn't parse leaves it open.
        #[qinvokable]
        #[cxx_name = "setBound"]
        fn set_bound(self: Pin<&mut Filters>, facet: &QString, end: &QString, text: &QString);

        /// Whether text typed for a range facet is empty or parses (`query::Query::bound_is_valid`).
        #[qinvokable]
        #[cxx_name = "boundValid"]
        fn bound_valid(self: &Filters, facet: &QString, text: &QString) -> bool;

        /// Filter nothing.
        #[qinvokable]
        fn clear(self: Pin<&mut Filters>);

        /// How many files choosing a type (`image`, `animated`, `video`) or an extension would show, given the
        /// other facets (`countFacets()`).
        #[qinvokable]
        #[cxx_name = "typeCount"]
        fn type_count(self: &Filters, media_type: &QString) -> i32;

        #[qinvokable]
        #[cxx_name = "extCount"]
        fn ext_count(self: &Filters, ext: &QString) -> i32;

        /// The same for a borders value (`query::BorderFacet`: `none`, `black`, `white`, `color`, `blur`).
        #[qinvokable]
        #[cxx_name = "borderCount"]
        fn border_count(self: &Filters, value: &QString) -> i32;

        /// Count a merged folder's files (`""` = all folders), with or without its subfolders: `total`, `matching`,
        /// the types', extensions' and borders' counts. Counted off the GUI thread; they change when the counts are in
        /// (`counted`).
        #[qinvokable]
        #[cxx_name = "countFacets"]
        fn count_facets(self: Pin<&mut Filters>, folder: &QString, recursive: bool);
    }
}

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use pg_catalog::browse::{self, read::Facets};
use pg_catalog::query::{from_json, to_json};
use pg_model::date::Day;
use pg_model::query::{BorderFacet, MediaType, Query, parse_number};

use pg_app::Kind;
use pg_client::input::{bound_is_valid, choice, date_facet, day_end_of, end_of, range_facet};
use pg_client::text;

pub struct FiltersRust {
    query: QString,
    active: i32,
    revision: i32,
    name: QString,
    duplicates: i32,
    border_marked: i32,
    total: i32,
    matching: i32,
    extensions: QStringList,
    description: QString,
    parsed: Query,
    facets: Facets,
    /// The latest counts asked for; older ones arriving are dropped.
    generation: u64,
}

impl Default for FiltersRust {
    /// Filtering nothing.
    fn default() -> FiltersRust {
        let parsed = Query::default();
        FiltersRust {
            query: QString::from(&to_json(&parsed)),
            active: 0,
            revision: 0,
            name: QString::default(),
            duplicates: duplicates_number(parsed.duplicates),
            border_marked: duplicates_number(parsed.border_marked),
            total: 0,
            matching: 0,
            extensions: QStringList::default(),
            description: QString::from(&text::describe(&parsed)),
            parsed,
            facets: Facets::default(),
            generation: 0,
        }
    }
}

/// A three-way choice as the properties hold it: -1 either way, 1 yes, 0 no.
fn duplicates_number(d: Option<bool>) -> i32 {
    d.map_or(-1, i32::from)
}

fn from_number(value: i32) -> Option<bool> {
    match value {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    }
}

/// Counts saturate: the properties are ints, as QML's are.
fn count(n: i64) -> i32 {
    i32::try_from(n.max(0)).unwrap_or(i32::MAX)
}

impl qobject::Filters {
    /// After the query changed: the properties that show it, and a new revision.
    fn sync(mut self: Pin<&mut Self>) {
        let q = &self.parsed;
        let (json, active, name, duplicates, border_marked, description) = (
            QString::from(&to_json(q)),
            q.active() as i32,
            QString::from(&q.name),
            duplicates_number(q.duplicates),
            duplicates_number(q.border_marked),
            QString::from(&text::describe(q)),
        );
        let mut exts: Vec<String> = self.facets.exts.iter().map(|(e, _)| e.clone()).collect();
        exts.extend(q.exts.iter().filter(|e| !self.facets.exts.iter().any(|(x, _)| x == *e)).cloned());
        if self.query != json {
            self.as_mut().rust_mut().query = json;
            self.as_mut().query_changed();
        }
        if self.name != name {
            self.as_mut().rust_mut().name = name;
            self.as_mut().name_changed();
        }
        if self.duplicates != duplicates {
            self.as_mut().rust_mut().duplicates = duplicates;
            self.as_mut().duplicates_changed();
        }
        if self.border_marked != border_marked {
            self.as_mut().rust_mut().border_marked = border_marked;
            self.as_mut().border_marked_changed();
        }
        self.as_mut().set_active(active);
        self.as_mut().set_description(description);
        self.as_mut().set_extensions(exts.iter().map(QString::from).collect());
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    /// The user changed the query.
    fn edit(mut self: Pin<&mut Self>, change: impl FnOnce(&mut Query)) {
        change(&mut self.as_mut().rust_mut().parsed);
        self.as_mut().sync();
        self.edited();
    }

    /// The query was replaced.
    fn replace(mut self: Pin<&mut Self>, query: Query) {
        self.as_mut().rust_mut().parsed = query;
        self.as_mut().sync();
        self.loaded();
    }

    fn set_query_loaded(self: Pin<&mut Self>, value: &QString) {
        self.replace(from_json(&value.to_string()));
    }

    fn set_name_edited(self: Pin<&mut Self>, value: &QString) {
        let name = value.to_string().trim().to_string();
        if name != self.parsed.name {
            self.edit(|q| q.name = name);
        }
    }

    fn set_duplicates_edited(self: Pin<&mut Self>, value: i32) {
        let duplicates = from_number(value);
        if duplicates != self.parsed.duplicates {
            self.edit(|q| q.duplicates = duplicates);
        }
    }

    fn set_border_marked_edited(self: Pin<&mut Self>, value: i32) {
        let marked = from_number(value);
        if marked != self.parsed.border_marked {
            self.edit(|q| q.border_marked = marked);
        }
    }

    fn has(&self, facet: &QString, value: &QString) -> bool {
        choice(&facet.to_string(), &value.to_string()).is_some_and(|c| self.parsed.has(&c))
    }

    fn toggle(self: Pin<&mut Self>, facet: &QString, value: &QString) {
        let (facet, value) = (facet.to_string(), value.to_string());
        if let Some(choice) = choice(&facet, &value) {
            self.edit(|q| q.toggle(choice));
        }
    }

    fn label(&self, facet: &QString, value: &QString) -> QString {
        QString::from(&text::facet_label(&facet.to_string(), &value.to_string()))
    }

    fn bound(&self, facet: &QString, end: &QString) -> QString {
        let (facet, end) = (facet.to_string(), end.to_string());
        let text = if let (Some(f), Some(e)) = (range_facet(&facet), end_of(&end)) {
            self.parsed.number(f, e).map(|v| v.to_string())
        } else if let (Some(f), Some(e)) = (date_facet(&facet), day_end_of(&end)) {
            self.parsed.day(f, e).map(|d| d.to_string())
        } else {
            None
        };
        QString::from(&text.unwrap_or_default())
    }

    fn set_bound(self: Pin<&mut Self>, facet: &QString, end: &QString, text: &QString) {
        let (facet, end, text) = (facet.to_string(), end.to_string(), text.to_string());
        if let (Some(f), Some(e)) = (range_facet(&facet), end_of(&end)) {
            self.edit(|q| q.set_number(f, e, parse_number(&text)));
        } else if let (Some(f), Some(e)) = (date_facet(&facet), day_end_of(&end)) {
            self.edit(|q| q.set_day(f, e, Day::parse(&text)));
        }
    }

    fn bound_valid(&self, facet: &QString, text: &QString) -> bool {
        bound_is_valid(&facet.to_string(), &text.to_string())
    }

    fn clear(mut self: Pin<&mut Self>) {
        self.as_mut().replace(Query::default());
        self.edited();
    }

    fn type_count(&self, media_type: &QString) -> i32 {
        let t = MediaType::from_key(&media_type.to_string());
        count(self.facets.types.iter().find(|(x, _)| Some(*x) == t).map_or(0, |(_, n)| *n))
    }

    fn ext_count(&self, ext: &QString) -> i32 {
        let ext = ext.to_string();
        count(self.facets.exts.iter().find(|(x, _)| *x == ext).map_or(0, |(_, n)| *n))
    }

    fn border_count(&self, value: &QString) -> i32 {
        let b = BorderFacet::from_key(&value.to_string());
        count(self.facets.borders.iter().find(|(x, _)| Some(*x) == b).map_or(0, |(_, n)| *n))
    }

    fn count_facets(mut self: Pin<&mut Self>, folder: &QString, recursive: bool) {
        let generation = {
            let mut this = self.as_mut().rust_mut();
            this.generation += 1;
            this.generation
        };
        let (path, query) = (folder.to_string(), self.parsed.clone());
        let filters = self.qt_thread();
        let shown = path.clone();
        crate::app::get().session().submit(
            Kind::Facets,
            move |c| browse::read::facets(c, &path, recursive, &query),
            move |facets| {
                let facets = facets.unwrap_or_else(|e| {
                    eprintln!("facets of {shown}: {e:#}");
                    Facets::default()
                });
                // Fails only when the filter bar is gone (the window closing).
                let _ = filters.queue(move |filters| filters.apply_facets(generation, facets));
            },
        );
    }

    fn apply_facets(mut self: Pin<&mut Self>, generation: u64, facets: Facets) {
        if generation != self.generation {
            return;
        }
        let (total, matching) = (count(facets.total), count(facets.matching));
        self.as_mut().rust_mut().facets = facets;
        self.as_mut().set_total(total);
        self.as_mut().set_matching(matching);
        self.as_mut().sync();
        self.counted();
    }
}
