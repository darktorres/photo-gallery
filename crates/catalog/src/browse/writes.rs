//! The browsing side's writes that aren't the file queue: a saved view, and the place the session was left at.
//! The ratings and turns the user makes are in `crate::writes`.

use crate::Result;
use rusqlite::{Connection, params};

use crate::Catalog;
use crate::query::to_json;

use pg_model::view::SavedView;

/// Save a view, replacing one of the same name.
pub fn save_view(catalog: &Catalog, v: &SavedView) -> Result<()> {
    let conn = &catalog.conn;
    conn.execute(
        "INSERT OR REPLACE INTO views(name, folder, recursive, query, sort) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![v.name, v.folder, v.recursive, to_json(&v.query), to_json(&v.sort)],
    )?;
    Ok(())
}

pub fn delete_view(catalog: &Catalog, name: &str) -> Result<()> {
    let conn = &catalog.conn;
    conn.execute("DELETE FROM views WHERE name = ?1", [name])?;
    Ok(())
}

pub fn session_set(catalog: &Catalog, key: &str, value: &str) -> Result<()> {
    put_session(&catalog.conn, key, value)
}

fn put_session(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute("INSERT OR REPLACE INTO session(key, value) VALUES (?1, ?2)", [key, value])?;
    Ok(())
}

/// Several session keys, in one transaction.
pub fn session_set_many(catalog: &Catalog, values: &[(&str, &str)]) -> Result<()> {
    let tx = catalog.conn.unchecked_transaction()?;
    for (key, value) in values {
        put_session(&tx, key, value)?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::{by, library};
    use crate::browse::read::views;
    use pg_model::query::{MediaType, Query, RatingFilter, SortKey};

    #[test]
    fn views_keep_their_query() {
        let c = library();
        let v = SavedView {
            name: "Unrated videos".into(),
            folder: "p".into(),
            recursive: true,
            query: Query { types: vec![MediaType::Video], ratings: vec![RatingFilter::Unrated], ..Query::default() },
            sort: by(SortKey::Size, true),
        };
        save_view(&c, &v).unwrap();
        assert_eq!(views(&c).unwrap(), [v]);
    }
}
