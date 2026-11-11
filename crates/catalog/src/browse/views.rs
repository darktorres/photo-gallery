//! The saved views and the session, read and written. Both are listings, stored in the same columns (`folder`,
//! `recursive`, `query`, `sort`): `listing_at` reads them and `listing_values` writes them. The ratings and turns the
//! user makes are the file queue's (`crate::queue`).

use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, params};

use pg_model::name::Name;
use pg_model::rules::PathRules;
use pg_model::view::{Listing, SavedView};
use pg_query::{from_json, to_json};

use crate::Catalog;
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

impl Catalog {
    /// The saved views, by name as the grid orders file names (the name's key).
    pub fn views(&self) -> Result<Vec<SavedView>> {
        let rules = self.rules();
        let mut stmt = self.conn.prepare("SELECT name, folder, recursive, query, sort FROM views")?;
        let rows = stmt.query_map([], |r| Ok(SavedView { name: r.get(0)?, listing: listing_at(r, 1, rules)? }))?;
        let mut views: Vec<SavedView> = rows.collect::<rusqlite::Result<_>>()?;
        views.sort_by_cached_key(|v| Name::from(v.name.as_str()).name_key());
        Ok(views)
    }

    /// Save a view, replacing one of the same name.
    pub fn save_view(&self, v: &SavedView) -> Result<()> {
        let (folder, recursive, query, sort) = listing_values(self.rules(), &v.listing);
        self.conn.execute(
            "INSERT OR REPLACE INTO views(name, folder, recursive, query, sort) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![v.name, folder, recursive, query, sort],
        )?;
        Ok(())
    }

    /// Delete the view named `name`, if there is one.
    pub fn delete_view(&self, name: &str) -> Result<()> {
        self.conn.execute("DELETE FROM views WHERE name = ?1", [name])?;
        Ok(())
    }

    /// Where the user was (the `session` table's row): read at start, written as they move. `None` before the first
    /// save.
    pub fn session_listing(&self) -> Result<Option<Listing>> {
        let rules = self.rules();
        let sql = "SELECT folder, recursive, query, sort FROM session WHERE id = 1";
        Ok(self.conn.query_row(sql, [], |r| listing_at(r, 0, rules)).optional()?)
    }

    /// Remember where the user is: the session's one row.
    pub fn save_session_listing(&self, listing: &Listing) -> Result<()> {
        put_session(&self.conn, self.rules(), listing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::{by, library, under_both_rules};
    use pg_model::query::{MediaType, Query, RatingFilter, SortKey};
    use pg_model::rules::Merged;

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
        c.save_view(&v).unwrap();
        assert_eq!(c.views().unwrap(), [v]);
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
        c.save_view(&v).unwrap();
        assert_eq!(c.views().unwrap(), [v]);
    }

    // The session is one row: none before the first save, then the listing last saved, read back as it was written.
    fn the_session_reads_back_its_last_listing(rules: PathRules) {
        let c = library(rules);
        assert_eq!(c.session_listing().unwrap(), None);
        let first = Listing { folder: Merged::from(vec![Name::from("p"), Name::from("q")]), ..Listing::default() };
        c.save_session_listing(&first).unwrap();
        let second = Listing { recursive: false, sort: by(SortKey::Size, true), ..first };
        c.save_session_listing(&second).unwrap();
        assert_eq!(c.session_listing().unwrap(), Some(second));
        let rows: i64 = c.conn.query_row("SELECT COUNT(*) FROM session", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1);
    }

    under_both_rules!(
        the_session_reads_back_its_last_listing,
        the_session_reads_back_its_last_listing_windows,
        the_session_reads_back_its_last_listing_posix
    );
}
