//! The browsing side's writes that aren't the file queue: a saved view, and the place the session was left at.
//! The ratings and turns the user makes are in `crate::writes`.

use anyhow::Result;
use rusqlite::{Connection, params};

use super::read::View;

/// Save a view, replacing one of the same name.
pub fn save_view(conn: &Connection, v: &View) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO views(name, folder, recursive, query, sort) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![v.name, v.folder, v.recursive, v.query.to_json(), v.sort.to_json()],
    )?;
    Ok(())
}

pub fn delete_view(conn: &Connection, name: &str) -> Result<()> {
    conn.execute("DELETE FROM views WHERE name = ?1", [name])?;
    Ok(())
}

pub fn session_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute("INSERT OR REPLACE INTO session(key, value) VALUES (?1, ?2)", [key, value])?;
    Ok(())
}

/// Several session keys, in one transaction.
pub fn session_set_many(conn: &Connection, values: &[(&str, &str)]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for (key, value) in values {
        session_set(&tx, key, value)?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::{by, library};
    use crate::browse::read::views;
    use crate::query::{MediaType, Query, SortKey};

    #[test]
    fn views_keep_their_query() {
        let c = library();
        let v = View {
            name: "Unrated videos".into(),
            folder: "p".into(),
            recursive: true,
            query: Query { types: vec![MediaType::Video], ratings: vec![0], ..Query::default() },
            sort: by(SortKey::Size, true),
        };
        save_view(&c.conn, &v).unwrap();
        assert_eq!(views(&c.conn).unwrap(), [v]);
    }
}
