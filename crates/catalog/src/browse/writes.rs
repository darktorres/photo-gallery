//! The write side of browsing: the commands the UI makes on the catalog (a rating, a turn, a saved view, the place the
//! session was left at). A rating or a turn that the file itself should carry is queued for the writeback, in the
//! same transaction (`file_writes`).

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use super::read::View;
use pg_model::format::Format;
use pg_model::ids::FileId;
use pg_model::rating::Rating;
use pg_model::turns::Turns;

/// Rate a file: 1–5, or 0 to clear. The catalog changes at once. With `write_back`, the rating is also queued
/// for the file itself (`ingest::writeback`), when its format can hold one and it differs from what the file
/// holds; clearing writes 0 only into a file that holds a rating.
pub fn set_rating(conn: &Connection, file_id: FileId, rating: Option<Rating>, write_back: bool) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let file_id = file_id.get();
    match rating {
        Some(rating) => tx.execute(
            "INSERT INTO ratings(file_id, rating, updated) VALUES (?1, ?2, unixepoch())
             ON CONFLICT(file_id) DO UPDATE SET rating = excluded.rating, updated = excluded.updated",
            params![file_id, rating.get()],
        )?,
        None => tx.execute("DELETE FROM ratings WHERE file_id = ?1", [file_id])?,
    };
    let format: Option<String> = tx
        .query_row("SELECT format FROM analysis WHERE file_id = ?1 AND error IS NULL", [file_id], |r| r.get(0))
        .optional()?
        .flatten();
    let can_hold = format.as_deref().and_then(Format::from_name).is_some_and(pg_metadata::can_hold_rating);
    let in_file: Option<u8> =
        tx.query_row("SELECT value FROM file_ratings WHERE file_id = ?1", [file_id], |r| r.get(0)).optional()?;
    // What the file should hold: the rating, or none (0) if it holds one and the rating is cleared.
    let wanted = match rating {
        Some(rating) => Some(rating.get()),
        None if in_file.is_some_and(|v| v > 0) => Some(0),
        None => in_file,
    };
    if write_back && can_hold && wanted != in_file {
        tx.execute(
            "INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, 'rating', ?2, unixepoch())
             ON CONFLICT(file_id, kind) DO UPDATE SET value = excluded.value, queued = excluded.queued,
                attempts = 0, error = NULL",
            params![file_id, wanted],
        )?;
    } else {
        // Back to what the file holds (or not written back): nothing left to write. A write already being
        // copied in finishes, and then this value follows it.
        tx.execute("DELETE FROM file_writes WHERE file_id = ?1 AND kind = 'rating' AND temp IS NULL", [file_id])?;
        tx.execute(
            "UPDATE file_writes SET value = ?2 WHERE file_id = ?1 AND kind = 'rating'",
            params![file_id, wanted.unwrap_or(0)],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Turn a file `turns` quarter turns clockwise: queued for `ingest::writeback`, which turns the file itself
/// losslessly where it can (`rotate::rotate`) and keeps the turn in the catalog otherwise. Turns add up until
/// written.
pub fn queue_rotation(conn: &Connection, file_id: FileId, turns: Turns) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let (file_id, turns) = (file_id.get(), turns.get());
    tx.execute(
        "INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, 'rotation', ?2 % 4, unixepoch())
         ON CONFLICT(file_id, kind) DO UPDATE SET value = (value + excluded.value) % 4, queued = excluded.queued,
            attempts = 0, error = NULL",
        params![file_id, turns],
    )?;
    tx.execute(
        "DELETE FROM file_writes WHERE file_id = ?1 AND kind = 'rotation' AND value = 0 AND temp IS NULL",
        [file_id],
    )?;
    tx.commit()?;
    Ok(())
}

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
    use crate::browse::read::{View, entry, list_grid, views};
    use crate::query::{MediaType, Query, Sort, SortKey};
    use pg_model::ids::FileId;
    use pg_model::rating::Rating;
    use pg_model::turns::Turns;

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

    fn queued(c: &crate::Catalog, id: i64, kind: &str) -> Option<i64> {
        c.conn
            .query_row("SELECT value FROM file_writes WHERE file_id = ?1 AND kind = ?2", params![id, kind], |r| {
                r.get(0)
            })
            .optional()
            .unwrap()
    }

    #[test]
    fn ratings_and_turns_queue_for_the_file() {
        let c = library();
        set_rating(&c.conn, FileId::new(2), Rating::new(3), true).unwrap();
        assert_eq!(queued(&c, 2, "rating"), Some(3));
        // Back to what the file holds (nothing): nothing to write.
        set_rating(&c.conn, FileId::new(2), None, true).unwrap();
        assert_eq!(queued(&c, 2, "rating"), None);
        // A file that holds 4: clearing writes 0.
        crate::Catalog::set_file_rating(&c.conn, FileId::new(1), Rating::new(4)).unwrap();
        set_rating(&c.conn, FileId::new(1), None, true).unwrap();
        assert_eq!(queued(&c, 1, "rating"), Some(0));
        // BMP can't hold one; write-back off writes nothing.
        set_rating(&c.conn, FileId::new(4), Rating::new(5), true).unwrap();
        set_rating(&c.conn, FileId::new(3), Rating::new(5), false).unwrap();
        assert_eq!((queued(&c, 4, "rating"), queued(&c, 3, "rating")), (None, None));
        // Turns add up modulo 4; a full turn leaves nothing to do.
        queue_rotation(&c.conn, FileId::new(1), Turns::new(1)).unwrap();
        queue_rotation(&c.conn, FileId::new(1), Turns::new(1)).unwrap();
        assert_eq!(queued(&c, 1, "rotation"), Some(2));
        queue_rotation(&c.conn, FileId::new(1), Turns::new(2)).unwrap();
        assert_eq!(queued(&c, 1, "rotation"), None);
        // The grid shows a pending turn, aspect included.
        queue_rotation(&c.conn, FileId::new(1), Turns::new(3)).unwrap();
        let entries = list_grid(&c.conn, "", true, &Query::default(), Sort::default()).unwrap();
        let e = entries.iter().find(|e| e.id == FileId::new(1)).unwrap();
        assert_eq!(e.pending_turns, Turns::new(3));
        assert_eq!((e.shown_size(), e.aspect()), ((3, 4), 3.0 / 4.0));
        assert_eq!(entry(&c.conn, FileId::new(1)).unwrap().map(|e| e.pending_turns), Some(Turns::new(3)));
    }
}
