//! The saved views and the session, as they were saved.

use crate::Catalog;
use crate::Result;
use rusqlite::OptionalExtension;

use crate::query::{Query, Sort};
use pg_model::path::name_key;

/// A saved view (ImageRater's bookmarks): where, which files, in what order.
#[derive(Clone, PartialEq, Debug)]
pub struct View {
    pub name: String,
    /// Merged path; `""` for all folders.
    pub folder: String,
    pub recursive: bool,
    pub query: Query,
    pub sort: Sort,
}

/// The saved views, by name as the grid orders file names (`name_key`).
pub fn views(catalog: &Catalog) -> Result<Vec<View>> {
    let conn = &catalog.conn;
    let mut stmt = conn.prepare("SELECT name, folder, recursive, query, sort FROM views")?;
    let rows = stmt.query_map([], |r| {
        Ok(View {
            name: r.get(0)?,
            folder: r.get(1)?,
            recursive: r.get(2)?,
            query: Query::from_json(&r.get::<_, String>(3)?),
            sort: Sort::from_json(&r.get::<_, String>(4)?),
        })
    })?;
    let mut views: Vec<View> = rows.collect::<rusqlite::Result<_>>()?;
    views.sort_by_cached_key(|v| name_key(&v.name));
    Ok(views)
}

/// Where the user was (`session` table): read at start, written as they move.
pub fn session_get(catalog: &Catalog, key: &str) -> Result<Option<String>> {
    let conn = &catalog.conn;
    Ok(conn.query_row("SELECT value FROM session WHERE key = ?1", [key], |r| r.get(0)).optional()?)
}
