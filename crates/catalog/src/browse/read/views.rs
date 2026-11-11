//! The saved views and the session, as they were saved. Both are listings, stored in the same columns (`folder`,
//! `recursive`, `query`, `sort`): `listing_at` reads them, and `browse::writes::listing_values` writes them.

use crate::Catalog;
use crate::error::Result;
use rusqlite::{OptionalExtension, Row};

use pg_model::name::Name;
use pg_model::rules::PathRules;
use pg_model::view::{Listing, SavedView};
use pg_query::from_json;
use rusqlite::types::Type;

/// The listing in the four columns from `first` on (`folder`, `recursive`, `query`, `sort`), the folder stored under
/// `rules`. A query or a sort that doesn't read is the default.
pub(crate) fn listing_at(r: &Row<'_>, first: usize, rules: PathRules) -> rusqlite::Result<Listing> {
    let stored: Vec<u8> = r.get(first)?;
    let folder = rules
        .parse_merged(&stored)
        .ok_or_else(|| rusqlite::Error::InvalidColumnType(first, "folder".into(), Type::Blob))?;
    Ok(Listing {
        folder,
        recursive: r.get(first + 1)?,
        query: from_json(&r.get::<_, String>(first + 2)?),
        sort: from_json(&r.get::<_, String>(first + 3)?),
    })
}

/// The saved views, by name as the grid orders file names (the name's key).
pub fn views(catalog: &Catalog) -> Result<Vec<SavedView>> {
    let rules = catalog.rules();
    let mut stmt = catalog.conn.prepare("SELECT name, folder, recursive, query, sort FROM views")?;
    let rows = stmt.query_map([], |r| Ok(SavedView { name: r.get(0)?, listing: listing_at(r, 1, rules)? }))?;
    let mut views: Vec<SavedView> = rows.collect::<rusqlite::Result<_>>()?;
    views.sort_by_cached_key(|v| Name::from(v.name.as_str()).name_key());
    Ok(views)
}

/// Where the user was (the `session` table's row): read at start, written as they move. `None` before the first save.
pub fn session_listing(catalog: &Catalog) -> Result<Option<Listing>> {
    let rules = catalog.rules();
    let sql = "SELECT folder, recursive, query, sort FROM session WHERE id = 1";
    Ok(catalog.conn.query_row(sql, [], |r| listing_at(r, 0, rules)).optional()?)
}
