//! The browsing side's writes that aren't the file queue: a saved view, and the place the session was left at.
//! The ratings and turns the user makes are in `crate::writes`.

use crate::error::Result;
use rusqlite::{Connection, params};

use crate::Catalog;
use pg_query::to_json;

use pg_model::view::SavedView;

/// Save a view, replacing one of the same name.
pub fn save_view(catalog: &Catalog, v: &SavedView) -> Result<()> {
    let conn = &catalog.conn;
    let folder = catalog.rules().join_merged(&v.folder);
    conn.execute(
        "INSERT OR REPLACE INTO views(name, folder, recursive, query, sort) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![v.name, folder, v.recursive, to_json(&v.query), to_json(&v.sort)],
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
    let tx = crate::schema::write_transaction(&catalog.conn)?;
    for (key, value) in values {
        put_session(&tx, key, value)?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::{by, library, under_both_rules};
    use crate::browse::read::{session_get, views};
    use pg_model::name::Name;
    use pg_model::query::{MediaType, Query, RatingFilter, SortKey};
    use pg_model::rules::{Merged, PathRules};

    fn views_keep_their_query(rules: PathRules) {
        let c = library(rules);
        let v = SavedView {
            name: "Unrated videos".into(),
            folder: Merged::from(vec![Name::from("p")]),
            recursive: true,
            query: Query { types: vec![MediaType::Video], ratings: vec![RatingFilter::Unrated], ..Query::default() },
            sort: by(SortKey::Size, true),
        };
        save_view(&c, &v).unwrap();
        assert_eq!(views(&c).unwrap(), [v]);
    }

    under_both_rules!(views_keep_their_query, views_keep_their_query_windows, views_keep_their_query_posix);

    // A folder holding a lone surrogate and a raw byte (no text holds either) is stored and read back exactly.
    #[test]
    fn a_saved_view_keeps_a_folder_with_odd_names() {
        let c =
            Catalog::prepare(std::path::Path::new(":memory:"), crate::fixture::versions(), PathRules::Posix).unwrap();
        let folder = Merged::from(vec![Name::from_raw(b"x\xFF"), Name::from_wide(&[0x61, 0xD800])]);
        let v = SavedView {
            name: "odd".into(),
            folder,
            recursive: true,
            query: Query::default(),
            sort: by(SortKey::Name, false),
        };
        save_view(&c, &v).unwrap();
        assert_eq!(views(&c).unwrap(), [v]);
    }

    // The place's keys are stored as text and read back as they were written, several in one transaction, and a
    // replaced key keeps its last value.
    fn session_keys_read_back_as_written(rules: PathRules) {
        let c = library(rules);
        session_set_many(&c, &[("folder", "p\\q"), ("recursive", "false"), ("sort", "{}")]).unwrap();
        session_set(&c, "sort", "{\"key\":\"size\"}").unwrap();
        assert_eq!(session_get(&c, "folder").unwrap().as_deref(), Some("p\\q"));
        assert_eq!(session_get(&c, "recursive").unwrap().as_deref(), Some("false"));
        assert_eq!(session_get(&c, "sort").unwrap().as_deref(), Some("{\"key\":\"size\"}"));
        assert_eq!(session_get(&c, "query").unwrap(), None);
    }

    under_both_rules!(
        session_keys_read_back_as_written,
        session_keys_read_back_as_written_windows,
        session_keys_read_back_as_written_posix
    );
}
