//! The ids of `files`, `folders` and `roots` are `AUTOINCREMENT` (migration 19): an id a removed folder had is never handed
//! out again, so a stale id names nothing, and it can't reach another file. SQLite can't make a `INTEGER PRIMARY KEY`
//! `AUTOINCREMENT` in place, so each table is rebuilt: the new table is made with the same columns, the rows are copied
//! with their ids, the old table is dropped and the new one renamed into its place. The table's indexes and triggers are
//! replayed from the text SQLite stores for them, so they are exactly the ones it had, whichever migration made them.
//!
//! The rebuild runs with foreign keys off and `legacy_alter_table` on (`schema::FOREIGN_KEYS_OFF_FROM`): the drop of a
//! parent would otherwise cascade into its children, and the rename would rewrite the views and triggers of the other
//! tables. The foreign-key violations are counted before and after, and a rebuild that adds any is refused.

use anyhow::anyhow;
use pg_model::rules::PathRules;
use rusqlite::Transaction;

use crate::convert::Source;
use crate::error::{Error, Result};

/// The tables rebuilt, parents first, with their definitions and their columns (the copy's column list).
const REBUILT: &[(&str, &str, &str)] = &[
    (
        "roots",
        "id INTEGER PRIMARY KEY AUTOINCREMENT, path BLOB NOT NULL UNIQUE, added INTEGER NOT NULL",
        "id, path, added",
    ),
    ("folders", "id INTEGER PRIMARY KEY AUTOINCREMENT, path BLOB NOT NULL UNIQUE, key BLOB NOT NULL", "id, path, key"),
    (
        "files",
        "id INTEGER PRIMARY KEY AUTOINCREMENT, volume INTEGER, file_ref INTEGER, created INTEGER, \
         folder_id INTEGER NOT NULL REFERENCES folders(id), name BLOB NOT NULL, ext TEXT NOT NULL, kind TEXT NOT NULL, \
         size INTEGER NOT NULL, mtime INTEGER NOT NULL, missing INTEGER NOT NULL DEFAULT 0, \
         rotation INTEGER NOT NULL DEFAULT 0, order_key BLOB NOT NULL, pending INTEGER NOT NULL DEFAULT 1, \
         seq INTEGER NOT NULL DEFAULT 0, changed INTEGER NOT NULL DEFAULT 0, unpaired INTEGER NOT NULL DEFAULT 0, \
         UNIQUE(volume, file_ref)",
        "id, volume, file_ref, created, folder_id, name, ext, kind, size, mtime, missing, rotation, order_key, pending, \
         seq, changed, unpaired",
    ),
];

/// Migration 19's conversion: `files`, `folders` and `roots` become `AUTOINCREMENT`. Runs in the migration's transaction
/// with foreign keys off (checked first). It is a no-op where `files` is already `AUTOINCREMENT`: the copy a rebuild makes
/// runs the migrations' conversions on its copied rows, and its tables were made by these steps already.
pub(crate) fn ids_autoincrement(tx: &Transaction<'_>, _source: Source, _rules: PathRules) -> Result<()> {
    if is_autoincrement(tx, "files")? {
        return Ok(());
    }
    let before = foreign_key_violations(tx)?;
    for (table, definition, columns) in REBUILT {
        rebuild(tx, table, definition, columns)?;
    }
    let after = foreign_key_violations(tx)?;
    if after > before {
        return Err(Error::Other(anyhow!(
            "the rebuild for AUTOINCREMENT would add foreign key violations ({before} before, {after} after): \
             it is refused, and the catalog is unchanged"
        )));
    }
    if before > 0 {
        pg_log::warn(
            "migration",
            &format!(
                "{before} foreign key violations were in the catalog before its migration; they are kept as they were"
            ),
        );
    }
    // A file whose pages the rebuild left behind is compacted at this start (`vacuum_if_owed`).
    tx.execute("UPDATE meta SET value = 1 WHERE key = 'vacuum_owed'", [])?;
    Ok(())
}

/// Whether `table` is an `AUTOINCREMENT` table already (its stored definition says so).
fn is_autoincrement(tx: &Transaction<'_>, table: &str) -> Result<bool> {
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1 AND instr(sql, 'AUTOINCREMENT') > 0",
        [table],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

/// Rebuilds `table` with `definition` and copies `columns` of its rows into the new table, keeping their ids. Its
/// indexes and triggers are read as SQLite stores them before the drop, and run again after the rename.
fn rebuild(tx: &Transaction<'_>, table: &str, definition: &str, columns: &str) -> Result<()> {
    let objects: Vec<String> = tx
        .prepare(
            "SELECT sql FROM sqlite_schema WHERE tbl_name = ?1 AND type IN ('index', 'trigger') AND sql IS NOT NULL \
             ORDER BY type, name",
        )?
        .query_map([table], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    tx.execute_batch(&format!("CREATE TABLE {table}_new({definition});"))?;
    tx.execute(&format!("INSERT INTO {table}_new({columns}) SELECT {columns} FROM {table}"), [])?;
    tx.execute_batch(&format!("DROP TABLE {table}; ALTER TABLE {table}_new RENAME TO {table};"))?;
    for sql in &objects {
        tx.execute_batch(sql)?;
    }
    Ok(())
}

/// The number of foreign key violations in the database (`PRAGMA foreign_key_check`: one row each).
fn foreign_key_violations(tx: &Transaction<'_>) -> Result<usize> {
    let mut statement = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while rows.next()?.is_some() {
        count += 1;
    }
    Ok(count)
}
