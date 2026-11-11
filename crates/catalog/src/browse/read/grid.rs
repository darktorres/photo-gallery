//! The grid's entries: what a list of files shows of each (`Entry`, 56 bytes), a single file's entry, and a
//! folder's file list, filtered and sorted.

use std::cmp::Ordering;

use crate::core::Catalog;
use crate::error::Result;
use rusqlite::types::{Type, Value};
use rusqlite::{OptionalExtension, params_from_iter};

use super::scope::scope;
use pg_model::grid::{Borders, Entry};
use pg_model::ids::FileId;
use pg_model::media::Ext;
use pg_model::orientation::Orientation;
use pg_model::query::{Query, Sort};
use pg_model::rating::Rating;
use pg_model::rules::Merged;
use pg_model::turns::Turns;
use pg_query::{condition, key_sql};

/// What an `Entry` is read from (`entry_from_row`), with `f` the file. Macros for the statements that are constants,
/// constants for those made at run time.
macro_rules! entry_columns {
    () => {
        "f.id, f.mtime, coalesce(a.width, 0), coalesce(a.height, 0), a.duration_ms, f.ext,
    f.rotation, coalesce(w.value, 0), r.rating, coalesce(a.frames, 1), coalesce(a.orientation, 1),
    a.cut_left, a.cut_top, a.cut_right, a.cut_bottom, m.file_id IS NOT NULL"
    };
}

const ENTRY_COLUMNS: &str = entry_columns!();

/// The column after `ENTRY_COLUMNS`: a sort key (`list_grid`).
const SORT_COLUMN: usize = 16;

/// What the entries and the filters read besides the file.
macro_rules! file_joins {
    () => {
        concat!(
            "LEFT JOIN analysis a ON a.file_id = f.id AND a.error IS NULL
    LEFT JOIN ratings r ON r.file_id = f.id
    LEFT JOIN file_writes w ON w.file_id = f.id AND w.kind = '",
            pg_model::write_rotation!(),
            "'
    LEFT JOIN border_marks m ON m.file_id = f.id"
        )
    };
}

pub(crate) const FILE_JOINS: &str = file_joins!();

fn entry_from_row(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    let ext = r.get_ref(5)?.as_str()?;
    let sides: [Option<u32>; 4] = [r.get(11)?, r.get(12)?, r.get(13)?, r.get(14)?];
    let cut = sides.map(Option::unwrap_or_default);
    let cut_found = sides[0].map(|_| cut.iter().any(|&c| c > 0));
    Ok(Entry {
        id: FileId::new(r.get(0)?),
        mtime: r.get(1)?,
        width: r.get(2)?,
        height: r.get(3)?,
        duration_ms: r
            .get::<_, Option<i64>>(4)?
            .map_or(Entry::NO_DURATION, |ms| u32::try_from(ms.clamp(0, i64::from(u32::MAX) - 1)).unwrap_or(u32::MAX)),
        // The scan records only files with these extensions.
        ext: Ext::of(ext).ok_or_else(|| rusqlite::Error::InvalidColumnType(5, format!("ext {ext}"), Type::Text))?,
        rotation: Turns::new(r.get(6)?),
        pending_turns: Turns::new(r.get(7)?),
        rating: r.get::<_, Option<u8>>(8)?.and_then(Rating::new),
        animated: r.get::<_, i64>(9)? > 1,
        orientation: Orientation::from_tag(r.get(10)?),
        cut,
        borders: match cut_found {
            None => Borders::Unknown,
            Some(false) => Borders::None,
            Some(true) if r.get(15)? => Borders::Marked,
            Some(true) => Borders::Found,
        },
    })
}

/// The order of two sort keys (`key_sql`'s values): numbers, or text by its bytes (SQLite's order), reversed when
/// `descending`. No key (NULL) comes last either way.
fn compare_keys(a: &Value, b: &Value, descending: bool) -> Ordering {
    let by_value = match (a, b) {
        (Value::Null, Value::Null) => return Ordering::Equal,
        (Value::Null, _) => return Ordering::Greater,
        (_, Value::Null) => return Ordering::Less,
        (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
        (Value::Text(a), Value::Text(b)) => a.cmp(b),
        // No sort key is a real number or a blob.
        _ => Ordering::Equal,
    };
    if descending { by_value.reverse() } else { by_value }
}

impl Catalog {
    /// Read an entry again (after its file was rated or turned), keeping its place in the list. `None` if it is gone.
    pub fn grid_entry(&self, file_id: FileId) -> Result<Option<Entry>> {
        let conn = &self.conn;
        Ok(conn
            .prepare_cached(concat!("SELECT ", entry_columns!(), " FROM files f ", file_joins!(), " WHERE f.id = ?1"))?
            .query_row([file_id.get()], entry_from_row)
            .optional()?)
    }

    /// The present files of a view: a merged folder (optionally with everything below it), filtered and ordered.
    ///
    /// They are read in name order from the `files_order` index, so SQLite never sorts. Another order is a stable
    /// sort here on that key alone, files without it last: files with the same key stay in name order.
    ///
    /// The list is sized by a count of the scope's files, read from the `files_order` index alone in the same read
    /// transaction (the background job writes meanwhile): exact without filters, an upper bound with them.
    pub fn list_grid(&self, folder: &Merged, recursive: bool, query: &Query, sort: Sort) -> Result<Vec<Entry>> {
        let conn = &self.conn;
        let (scope, mut values) = scope(self.rules(), folder, recursive);
        let tx = conn.unchecked_transaction()?;
        let bound: usize = tx.query_row(
            &format!("SELECT count(*) FROM files f WHERE f.missing = 0 AND {scope}"),
            params_from_iter(&values),
            |r| r.get(0),
        )?;
        let cond = condition(query);
        values.extend(cond.params);
        let mut stmt = tx.prepare(&grid_sql(&scope, &cond.sql, sort))?;
        let mut rows = stmt.query(params_from_iter(values))?;
        let entries = if key_sql(sort).is_none() {
            let mut entries = Vec::with_capacity(bound);
            while let Some(r) = rows.next()? {
                entries.push(entry_from_row(r)?);
            }
            // What the filters left out.
            if entries.len() < entries.capacity() / 4 * 3 {
                entries.shrink_to_fit();
            }
            entries
        } else {
            let mut keyed = Vec::with_capacity(bound);
            while let Some(r) = rows.next()? {
                keyed.push((r.get::<_, Value>(SORT_COLUMN)?, entry_from_row(r)?));
            }
            keyed.sort_by(|(a, _), (b, _)| compare_keys(a, b, sort.descending));
            // Collected in place, into the keyed list's allocation, which is a fifth larger than the entries need.
            let mut entries: Vec<Entry> = keyed.into_iter().map(|(_, e)| e).collect();
            entries.shrink_to_fit();
            entries
        };
        drop(rows);
        drop(stmt);
        tx.commit()?;
        Ok(entries)
    }
}

/// `list_grid`'s query: the entries in name order, then the sort key if it isn't the name.
pub(crate) fn grid_sql(scope: &str, cond: &str, sort: Sort) -> String {
    let key = key_sql(sort);
    // Name order backwards for "by name, descending"; the other keys take their ties in name order.
    let dir = if key.is_none() && sort.descending { "DESC" } else { "ASC" };
    // `missing = 0` is written out, not bound: it's the `files_order` index's condition.
    let (comma, key) = key.map_or(("", ""), |k| (", ", k));
    format!(
        "SELECT {ENTRY_COLUMNS}{comma}{key} FROM files f {FILE_JOINS}
         WHERE f.missing = 0 AND {scope} AND {cond}
         ORDER BY f.order_key {dir}, f.id {dir}"
    )
}
