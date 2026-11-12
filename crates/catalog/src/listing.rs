//! A listing in its four columns (`folder`, `recursive`, `query`, `sort`), as the saved views and the session store
//! it: read, written, and the session's row put. The views and the migrations' conversion of the old session
//! (`convert::convert_session`) use it. A leaf, below `core`.

use rusqlite::types::Type;
use rusqlite::{Connection, Row, params};

use pg_model::rules::PathRules;
use pg_model::view::Listing;
use pg_query::{from_json, to_json};

use crate::error::{Error, Result};

/// The listing in the four columns from `first` on (`folder`, `recursive`, `query`, `sort`), the folder stored under
/// `rules`. A query or a sort that doesn't read is the column's conversion error: the reader reports it, and the
/// stored text is left as it is.
pub(crate) fn listing_at(r: &Row<'_>, first: usize, rules: PathRules) -> rusqlite::Result<Listing> {
    let stored: Vec<u8> = r.get(first)?;
    let folder = rules
        .parse_merged(&stored)
        .ok_or_else(|| rusqlite::Error::InvalidColumnType(first, "folder".into(), Type::Blob))?;
    Ok(Listing {
        folder,
        recursive: r.get(first + 1)?,
        query: json_column(r, first + 2, from_json)?,
        sort: json_column(r, first + 3, from_json)?,
    })
}

/// A JSON column, read by `read`: text that doesn't read is the column's conversion failure.
fn json_column<T, E>(r: &Row<'_>, column: usize, read: fn(&str) -> std::result::Result<T, E>) -> rusqlite::Result<T>
where
    E: std::error::Error + Send + Sync + 'static,
{
    let text: String = r.get(column)?;
    read(&text).map_err(|e| rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(e)))
}

/// A listing's column values, `folder` (stored under `rules`), `recursive`, `query` and `sort`, in that order.
pub(crate) fn listing_values(rules: PathRules, listing: &Listing) -> Result<(Vec<u8>, bool, String, String)> {
    let query = to_json(&listing.query).map_err(|e| Error::Other(e.into()))?;
    let sort = to_json(&listing.sort).map_err(|e| Error::Other(e.into()))?;
    Ok((rules.join_merged(&listing.folder), listing.recursive, query, sort))
}

/// The session's row on `conn`.
pub(crate) fn put_session(conn: &Connection, rules: PathRules, listing: &Listing) -> Result<()> {
    let (folder, recursive, query, sort) = listing_values(rules, listing)?;
    conn.execute(
        "INSERT OR REPLACE INTO session(id, folder, recursive, query, sort) VALUES (1, ?1, ?2, ?3, ?4)",
        params![folder, recursive, query, sort],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::name::Name;
    use pg_model::rules::Merged;

    /// A stored listing whose query doesn't read is a read's error, and its text is left as it is: nothing is replaced
    /// by the default.
    #[test]
    fn a_stored_query_that_does_not_read_is_an_error_and_its_text_stays() {
        let rules = PathRules::Windows;
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE session(folder BLOB, recursive INTEGER, query TEXT, sort TEXT);").unwrap();
        let folder = rules.join_merged(&Merged::from(vec![Name::from("p")]));
        conn.execute(
            "INSERT INTO session VALUES (?1, 1, 'not json', '{\"key\":\"name\",\"descending\":false}')",
            params![folder],
        )
        .unwrap();
        let read =
            conn.query_row("SELECT folder, recursive, query, sort FROM session", [], |r| listing_at(r, 0, rules));
        assert!(read.is_err(), "{read:?}");
        let stored: String = conn.query_row("SELECT query FROM session", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, "not json");
    }
}
