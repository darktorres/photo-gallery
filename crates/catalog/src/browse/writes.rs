//! The browsing side's writes that aren't the file queue: a saved view, and the place the session was left at. Both are
//! listings in the same columns (`browse::read::listing_at` reads them back). The ratings and turns the user makes are
//! in `crate::writes`.

use crate::error::Result;
use rusqlite::{Connection, params};

use crate::Catalog;
use pg_query::to_json;

use pg_model::rules::PathRules;
use pg_model::view::{Listing, SavedView};

/// A listing's column values, `folder` (stored under `rules`), `recursive`, `query` and `sort`, in that order.
pub(crate) fn listing_values(rules: PathRules, listing: &Listing) -> (Vec<u8>, bool, String, String) {
    (rules.join_merged(&listing.folder), listing.recursive, to_json(&listing.query), to_json(&listing.sort))
}

/// Save a view, replacing one of the same name.
pub fn save_view(catalog: &Catalog, v: &SavedView) -> Result<()> {
    let (folder, recursive, query, sort) = listing_values(catalog.rules(), &v.listing);
    catalog.conn.execute(
        "INSERT OR REPLACE INTO views(name, folder, recursive, query, sort) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![v.name, folder, recursive, query, sort],
    )?;
    Ok(())
}

pub fn delete_view(catalog: &Catalog, name: &str) -> Result<()> {
    let conn = &catalog.conn;
    conn.execute("DELETE FROM views WHERE name = ?1", [name])?;
    Ok(())
}

/// Remember where the user is: the session's one row.
pub fn save_session_listing(catalog: &Catalog, listing: &Listing) -> Result<()> {
    put_session(&catalog.conn, catalog.rules(), listing)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::{by, library, under_both_rules};
    use crate::browse::read::{session_listing, views};
    use pg_model::name::Name;
    use pg_model::query::{MediaType, Query, RatingFilter, SortKey};
    use pg_model::rules::{Merged, PathRules};

    fn views_keep_their_query(rules: PathRules) {
        let c = library(rules);
        let v = SavedView {
            name: "Unrated videos".into(),
            listing: Listing {
                folder: Merged::from(vec![Name::from("p")]),
                recursive: true,
                query: Query { types: vec![MediaType::Video], ratings: vec![RatingFilter::Unrated], ..Query::default() },
                sort: by(SortKey::Size, true),
            },
        };
        save_view(&c, &v).unwrap();
        assert_eq!(views(&c).unwrap(), [v]);
    }

    under_both_rules!(views_keep_their_query, views_keep_their_query_windows, views_keep_their_query_posix);

    // A folder holding a lone surrogate and a raw byte (no text holds either) is stored and read back exactly.
    #[test]
    fn a_saved_view_keeps_a_folder_with_odd_names() {
        let c = Catalog::prepare(
            std::path::Path::new(":memory:"),
            crate::Versions {
                import: pg_analysis::analyze::IMPORT_VERSION,
                borders: pg_analysis::borders::BORDERS_VERSION,
                thumbs: 1,
            },
            PathRules::Posix,
        )
        .unwrap();
        let folder = Merged::from(vec![Name::from_raw(b"x\xFF"), Name::from_wide(&[0x61, 0xD800])]);
        let v = SavedView {
            name: "odd".into(),
            listing: Listing { folder, recursive: true, query: Query::default(), sort: by(SortKey::Name, false) },
        };
        save_view(&c, &v).unwrap();
        assert_eq!(views(&c).unwrap(), [v]);
    }

    // The session is one row: none before the first save, then the listing last saved, read back as it was written.
    fn the_session_reads_back_its_last_listing(rules: PathRules) {
        let c = library(rules);
        assert_eq!(session_listing(&c).unwrap(), None);
        let first = Listing { folder: Merged::from(vec![Name::from("p"), Name::from("q")]), ..Listing::default() };
        save_session_listing(&c, &first).unwrap();
        let second = Listing { recursive: false, sort: by(SortKey::Size, true), ..first };
        save_session_listing(&c, &second).unwrap();
        assert_eq!(session_listing(&c).unwrap(), Some(second));
        let rows: i64 = c.conn.query_row("SELECT COUNT(*) FROM session", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1);
    }

    under_both_rules!(
        the_session_reads_back_its_last_listing,
        the_session_reads_back_its_last_listing_windows,
        the_session_reads_back_its_last_listing_posix
    );
}
