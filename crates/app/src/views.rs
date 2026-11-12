//! Where the grid looks and in what order: the session's listing (the folder shown, with or without its subfolders, the
//! sort, and the filter bar's query), and the saved views (ImageRater's bookmarks). The listing is remembered for the
//! next start; a view is a listing with a name.

use std::sync::Arc;

use pg_model::query::{Query, Sort};
use pg_model::view::{Listing, SavedView};
use pg_query::from_json;

use crate::catalogs::Catalogs;
use crate::error::Result;

/// A filter query from its JSON (the command line's `--query=`). Text that doesn't read is an error, for the caller to report.
pub fn query_from_json(json: &str) -> serde_json::Result<Query> {
    from_json(json)
}

/// A sort from its JSON (the command line's `--sort=`). Text that doesn't read is an error, for the caller to report.
pub fn sort_from_json(json: &str) -> serde_json::Result<Sort> {
    from_json(json)
}

/// The listing and the saved views, in the catalog.
pub struct Views {
    catalogs: Arc<Catalogs>,
}

impl Views {
    pub(crate) fn new(catalogs: Arc<Catalogs>) -> Views {
        Views { catalogs }
    }

    /// Where the user was at the last save: the default listing where nothing was saved. A saved listing that can't be
    /// read is an error (the caller says so, and uses the default for that start only; the stored text stays).
    pub fn listing(&self) -> Result<Listing> {
        Ok(self.catalogs.with_catalog(pg_catalog::Catalog::session_listing)?.unwrap_or_default())
    }

    /// Remember the listing for the next start.
    pub fn save_listing(&self, listing: &Listing) -> Result<()> {
        self.catalogs.with_catalog(|c| c.save_session_listing(listing))
    }

    /// The saved views, sorted by name.
    pub fn all(&self) -> Result<Vec<SavedView>> {
        self.catalogs.with_catalog(pg_catalog::Catalog::views)
    }

    /// Save (or replace) a view under its name.
    pub fn save(&self, view: &SavedView) -> Result<()> {
        self.catalogs.with_catalog(|c| c.save_view(view))
    }

    /// Delete the view of this name.
    pub fn delete(&self, name: &str) -> Result<()> {
        self.catalogs.with_catalog(|c| c.delete_view(name))
    }

    /// The saved view by this name, if there is one.
    pub fn view(&self, name: &str) -> Result<Option<SavedView>> {
        Ok(self.all()?.into_iter().find(|v| v.name == name))
    }
}
