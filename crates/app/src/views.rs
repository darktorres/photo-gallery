//! Where the grid looks and in what order: the session's place (the folder shown, with or without its subfolders, the
//! sort, and the filter bar's query), and the saved views (ImageRater's bookmarks). The place is remembered for the next
//! start; a view is a place with a name.

use pg_catalog::browse;
use pg_catalog::query::{from_json, to_json};
use pg_model::query::{Query, Sort};
use pg_model::tree::FolderTree;
use pg_model::view::{Place, SavedView};

use crate::app::App;
use crate::error::Result;

impl App {
    /// A session value (the front end's keys: `folder`, `recursive`, `query`, `sort`, `file`), if one was saved.
    pub fn session_value(&self, key: &str) -> Option<String> {
        self.with_catalog(|c| browse::read::session_get(c, key)).ok().flatten()
    }

    /// Where the user was at the last save: the defaults where nothing was saved.
    pub fn place(&self) -> Place {
        Place {
            folder: self.session_value("folder").unwrap_or_default(),
            recursive: self.session_value("recursive").is_none_or(|r| r == "true"),
            sort: from_json::<Sort>(&self.session_value("sort").unwrap_or_default()),
            query: from_json::<Query>(&self.session_value("query").unwrap_or_default()),
        }
    }

    /// Remember the place for the next start.
    pub fn save_place(&self, place: &Place) -> Result<()> {
        let folder = place.folder.as_str();
        let recursive = place.recursive.to_string();
        let query = to_json(&place.query);
        let sort = to_json(&place.sort);
        let values = [("folder", folder), ("recursive", &*recursive), ("query", &*query), ("sort", &*sort)];
        self.with_catalog(|c| browse::writes::session_set_many(c, &values))
    }

    /// The saved views, sorted by name.
    pub fn views(&self) -> Result<Vec<SavedView>> {
        self.with_catalog(browse::read::views)
    }

    /// Save (or replace) a view under its name.
    pub fn save_view(&self, view: &SavedView) -> Result<()> {
        self.with_catalog(|c| browse::writes::save_view(c, view))
    }

    pub fn delete_view(&self, name: &str) -> Result<()> {
        self.with_catalog(|c| browse::writes::delete_view(c, name))
    }

    /// The saved view by this name, if there is one.
    pub fn view(&self, name: &str) -> Result<Option<SavedView>> {
        Ok(self.views()?.into_iter().find(|v| v.name == name))
    }

    /// The folder to show at start: the saved one when the tree still has it (as its merged path), else `None` (all
    /// folders). The tree is the folder tree the grid reads.
    pub fn restore(&self, tree: &FolderTree) -> Option<String> {
        let folder = self.place().folder;
        if folder.is_empty() {
            return None;
        }
        tree.find(&folder, &mut Vec::new()).map(|_| folder)
    }
}
