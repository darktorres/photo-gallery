//! A listing in its four columns (`folder`, `recursive`, `query`, `sort`), as the saved views and the session store
//! it: read, written, and the session's row put, which the views and the rebuild's copy both do. A leaf, below `core`.

use rusqlite::types::Type;
use rusqlite::{Connection, Row, params};

use pg_model::rules::PathRules;
use pg_model::view::Listing;
use pg_query::{from_json, to_json};

use crate::error::Result;

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

/// A listing's column values, `folder` (stored under `rules`), `recursive`, `query` and `sort`, in that order.
pub(crate) fn listing_values(rules: PathRules, listing: &Listing) -> (Vec<u8>, bool, String, String) {
    (rules.join_merged(&listing.folder), listing.recursive, to_json(&listing.query), to_json(&listing.sort))
}

/// The session's row on `conn` (the rebuild's copy writes it too).
pub(crate) fn put_session(conn: &Connection, rules: PathRules, listing: &Listing) -> Result<()> {
    let (folder, recursive, query, sort) = listing_values(rules, listing);
    conn.execute(
        "INSERT OR REPLACE INTO session(id, folder, recursive, query, sort) VALUES (1, ?1, ?2, ?3, ?4)",
        params![folder, recursive, query, sort],
    )?;
    Ok(())
}
