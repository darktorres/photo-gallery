//! Where the grid looks and in what order (PLAN §7): a merged folder, with or without its subfolders, in a
//! `query::Sort` order. With the filter bar's query (`filters::Filters`, as JSON), that is a view: where the user
//! was (the session's `folder`, `recursive`, `query` and `sort`) and the saved views (ImageRater's bookmarks) are
//! kept here. The window hands `sortJson()` and the query to the grid model, so the QML never builds or reads JSON.

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
        /// The merged folder shown (`""` = all folders).
        #[qproperty(QString, folder, READ, WRITE, NOTIFY, FINAL)]
        /// With the files of its subfolders.
        #[qproperty(bool, recursive, READ, WRITE, NOTIFY, FINAL)]
        /// What the files are ordered by (`query::SortKey` by name: `name`, `size`, `modified`, `taken`,
        /// `megapixels` or `duration`), and which way.
        #[qproperty(QString, sort_key, cxx_name = "sortKey", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(bool, descending, READ, WRITE, NOTIFY, FINAL)]
        /// Names of the saved views, sorted.
        #[qproperty(QStringList, saved_views, cxx_name = "savedViews", READ, WRITE, NOTIFY, FINAL)]
        type View = super::ViewRust;
    }

    unsafe extern "RustQt" {
        /// The sort, for `GalleryModel.openFolder`.
        #[qinvokable]
        #[cxx_name = "sortJson"]
        fn sort_json(self: &View) -> QString;

        /// Replace the sort (`query::Sort` as JSON: the window's test hook).
        #[qinvokable]
        #[cxx_name = "setSortJson"]
        fn set_sort_json(self: Pin<&mut View>, json: &QString);

        /// The query the last session left (`query::Query` as JSON).
        #[qinvokable]
        #[cxx_name = "sessionQuery"]
        fn session_query(self: &View) -> QString;

        /// Remember the folder, subfolders, sort and `query` for the next start.
        #[qinvokable]
        #[cxx_name = "saveSession"]
        fn save_session(self: &View, query: &QString);

        /// Save (or replace) the current view, filtered by `query`, under a name.
        #[qinvokable]
        #[cxx_name = "saveView"]
        fn save_view(self: Pin<&mut View>, name: &QString, query: &QString);

        #[qinvokable]
        #[cxx_name = "deleteView"]
        fn delete_view(self: Pin<&mut View>, name: &QString);

        /// Take a saved view's folder, subfolders and sort, and return its query (as JSON) for the filter bar; `""`
        /// if there is no view by that name.
        #[qinvokable]
        #[cxx_name = "applyView"]
        fn apply_view(self: Pin<&mut View>, name: &QString) -> QString;
    }
}

use core::pin::Pin;

use cxx_qt_lib::{QString, QStringList};
use pg_core::browse;
use pg_core::query::{Query, Sort, SortKey};

use crate::state;

pub struct ViewRust {
    folder: QString,
    recursive: bool,
    sort_key: QString,
    descending: bool,
    saved_views: QStringList,
}

fn session(key: &str) -> Option<String> {
    state::with_catalog(|c| browse::session_get(&c.conn, key)).ok().flatten()
}

fn view_names() -> QStringList {
    state::with_catalog(|c| browse::views(&c.conn))
        .unwrap_or_else(|e| {
            eprintln!("views: {e:#}");
            Vec::new()
        })
        .iter()
        .map(|v| QString::from(&v.name))
        .collect()
}

impl Default for ViewRust {
    /// Where the user was.
    fn default() -> ViewRust {
        let sort = Sort::from_json(&session("sort").unwrap_or_default());
        ViewRust {
            folder: QString::from(&session("folder").unwrap_or_default()),
            recursive: session("recursive").is_none_or(|r| r == "true"),
            sort_key: QString::from(sort.key.key()),
            descending: sort.descending,
            saved_views: view_names(),
        }
    }
}

impl ViewRust {
    fn sort(&self) -> Sort {
        Sort {
            key: SortKey::from_key(&self.sort_key.to_string()).unwrap_or(SortKey::Name),
            descending: self.descending,
        }
    }
}

impl qobject::View {
    fn sort_json(&self) -> QString {
        QString::from(&self.sort().to_json())
    }

    fn set_sort_json(mut self: Pin<&mut Self>, json: &QString) {
        let sort = Sort::from_json(&json.to_string());
        self.as_mut().set_sort_key(QString::from(sort.key.key()));
        self.set_descending(sort.descending);
    }

    fn session_query(&self) -> QString {
        QString::from(&session("query").unwrap_or_default())
    }

    fn save_session(&self, query: &QString) {
        let (folder, recursive, query, sort) =
            (self.folder.to_string(), self.recursive.to_string(), query.to_string(), self.sort().to_json());
        let values = [("folder", &*folder), ("recursive", &*recursive), ("query", &*query), ("sort", &*sort)];
        if let Err(e) = state::with_catalog(|c| browse::session_set_many(&c.conn, &values)) {
            eprintln!("session: {e:#}");
        }
    }

    fn save_view(self: Pin<&mut Self>, name: &QString, query: &QString) {
        let name = name.to_string().trim().to_string();
        if name.is_empty() {
            return;
        }
        let view = browse::View {
            name,
            folder: self.folder.to_string(),
            recursive: self.recursive,
            query: Query::from_json(&query.to_string()),
            sort: self.sort(),
        };
        if let Err(e) = state::with_catalog(|c| browse::save_view(&c.conn, &view)) {
            eprintln!("saving view: {e:#}");
        }
        self.set_saved_views(view_names());
    }

    fn delete_view(self: Pin<&mut Self>, name: &QString) {
        if let Err(e) = state::with_catalog(|c| browse::delete_view(&c.conn, &name.to_string())) {
            eprintln!("deleting view: {e:#}");
        }
        self.set_saved_views(view_names());
    }

    fn apply_view(mut self: Pin<&mut Self>, name: &QString) -> QString {
        let name = name.to_string();
        let found =
            state::with_catalog(|c| browse::views(&c.conn)).ok().and_then(|vs| vs.into_iter().find(|v| v.name == name));
        let Some(v) = found else { return QString::default() };
        self.as_mut().set_folder(QString::from(&v.folder));
        self.as_mut().set_recursive(v.recursive);
        self.as_mut().set_sort_key(QString::from(v.sort.key.key()));
        self.set_descending(v.sort.descending);
        QString::from(&v.query.to_json())
    }
}
