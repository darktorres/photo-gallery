//! The saved views and the session, as they were saved.

use crate::Catalog;
use crate::Result;
use rusqlite::OptionalExtension;

use crate::query::from_json;
use pg_model::name::Name;
use pg_model::view::SavedView;
use rusqlite::types::Type;

/// The saved views, by name as the grid orders file names (the name's key).
pub fn views(catalog: &Catalog) -> Result<Vec<SavedView>> {
    let conn = &catalog.conn;
    let rules = catalog.rules();
    let mut stmt = conn.prepare("SELECT name, folder, recursive, query, sort FROM views")?;
    let rows = stmt.query_map([], |r| {
        let stored: Vec<u8> = r.get(1)?;
        let folder = rules
            .parse_merged(&stored)
            .ok_or_else(|| rusqlite::Error::InvalidColumnType(1, "folder".into(), Type::Blob))?;
        Ok(SavedView {
            name: r.get(0)?,
            folder,
            recursive: r.get(2)?,
            query: from_json(&r.get::<_, String>(3)?),
            sort: from_json(&r.get::<_, String>(4)?),
        })
    })?;
    let mut views: Vec<SavedView> = rows.collect::<rusqlite::Result<_>>()?;
    views.sort_by_cached_key(|v| Name::from(v.name.as_str()).name_key());
    Ok(views)
}

/// Where the user was (`session` table): read at start, written as they move.
pub fn session_get(catalog: &Catalog, key: &str) -> Result<Option<String>> {
    let conn = &catalog.conn;
    Ok(conn.query_row("SELECT value FROM session WHERE key = ?1", [key], |r| r.get(0)).optional()?)
}
