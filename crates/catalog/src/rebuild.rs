//! The catalog's rebuild, both halves: the file side (the replacement made beside the catalog, the old file moved aside,
//! the copy kept before a migration) and the row copy (`carry_rows`: every table's rows carried into the new layout, the
//! old shapes converted by the migrations' own `Rust` steps). `Catalog::prepare` decides when; this does it. Uses
//! `schema`, `convert`, `types` and `error`.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use pg_model::notice::Carried;
use pg_model::rules::PathRules;
use pg_platform::file::beside;
use rusqlite::Connection;

use crate::convert::{Source, column_type, table_columns};
use crate::error::{Error, Result};
use crate::schema::{BASELINE, MIGRATIONS, SETTINGS, Step, bring_to, current_version};
use crate::types::Rebuilt;

/// A count of rows the copy made (never negative), as the notice of the rebuild holds it.
fn count(rows: i64) -> u64 {
    u64::try_from(rows).unwrap_or(0)
}

/// Where the replacement for the catalog at `path` is made before it takes the name ([`rebuild`]).
fn building_path(path: &Path) -> PathBuf {
    beside(path, ".building")
}

/// Where the copy of the catalog at `path` made before it is migrated from `version` is kept.
pub(crate) fn backup_path(path: &Path, version: i64) -> PathBuf {
    beside(path, &format!(".pre-v{version}.bak"))
}

/// Copy the catalog at `path`, at `version`, aside before it is migrated forward (the migrations can't be undone, and an
/// older build refuses a catalog of a later version: going back to it is putting the copy back). Its `-wal` is merged
/// into the file first, so the copy is whole; the caller holds the catalog exclusively. A copy already there (from an
/// earlier try of the same migration) is kept: it is the older one.
pub(crate) fn back_up(conn: &Connection, path: &Path, version: i64) -> Result<()> {
    let backup = backup_path(path, version);
    if backup.exists() {
        return Ok(());
    }
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    let partial = beside(&backup, ".partial");
    std::fs::copy(db_file(path)?, &partial)
        .and_then(|_| std::fs::rename(&partial, &backup))
        .with_context(|| format!("copying the catalog to {} before it is migrated", backup.display()))?;
    Ok(())
}

/// Make the catalog `old` (at `path`) into a new one in this build's layout, carrying its rows ([`rebuild`]): the old file
/// is moved aside, and the new one takes its name. `old` is the connection the change was found on; it is closed here.
pub(crate) fn rebuild(old: Connection, path: &Path, rules: PathRules, why: String, readable: bool) -> Result<Rebuilt> {
    let source = if readable {
        // The changes still in the old file's `-wal` are checkpointed into it, so the copy reads all of them.
        old.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        // Paths stored under other rules don't read as paths under these, so none of them is carried; the reason
        // says why, for the note.
        match crate::lifecycle::rules_mismatch(&old, rules) {
            Ok(None) => Ok(()),
            Ok(Some(reason)) => Err(reason),
            Err(e) => Err(format!("its path rules can't be read: {e}")),
        }
    } else {
        Err("the old file holds no catalog".to_string())
    };
    drop(old);
    let replacement = building_path(path);
    remove_database(&replacement)?;
    let carried = match source {
        Ok(()) => match make_replacement(&replacement, rules, Some(path)) {
            Ok((files, folders)) => Carried::Rows { files: count(files), folders: count(folders) },
            Err(e) => {
                // The copy is one transaction, so an error leaves no rows; the replacement is made again without them.
                remove_database(&replacement)?;
                make_replacement(&replacement, rules, None)?;
                Carried::Nothing { reason: format!("{e:#}") }
            }
        },
        Err(reason) => {
            make_replacement(&replacement, rules, None)?;
            Carried::Nothing { reason }
        }
    };
    // To `catalog.sqlite.replaced`, then `.replaced.1`, and so on: kept whole, with its sidecars, for the user to
    // remove.
    let kept = pg_dbfile::move_aside(path, ".replaced")
        .with_context(|| format!("catalog {}: moving it aside ({why})", path.display()))?;
    std::fs::rename(&replacement, path).with_context(|| {
        format!("catalog {}: its new file {} can't take the name", path.display(), replacement.display())
    })?;
    Ok(Rebuilt { kept, why, carried })
}

/// The file SQLite opens for the catalog at `path`: the path itself, which the caller gave in a folder SQLite can open
/// (`pg_platform::sqlite::openable_folder`).
pub(crate) fn db_file(path: &Path) -> anyhow::Result<PathBuf> {
    Ok(path.to_path_buf())
}

/// Make the catalog file `path` in this build's layout, with the rows of `from` copied into it when given
/// ([`carry_rows`]). Returns its file and library folder counts (zero without `from`). The file is closed, with its
/// changes in the main file.
fn make_replacement(path: &Path, rules: PathRules, from: Option<&Path>) -> Result<(i64, i64)> {
    let mut conn = Connection::open(db_file(path)?).with_context(|| format!("catalog {}", path.display()))?;
    conn.execute_batch(SETTINGS)?;
    bring_to(&conn, BASELINE, MIGRATIONS, rules, current_version()?)
        .with_context(|| format!("catalog {}", path.display()))?;
    let counts = match from {
        Some(old) => {
            carry_rows(&mut conn, old, rules)?;
            let count = |table: &str| -> rusqlite::Result<i64> {
                conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            };
            (count("files")?, count("roots")?)
        }
        None => (0, 0),
    };
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    conn.close().map_err(|(_, e)| e)?;
    Ok(counts)
}

/// Delete the database at `path` with its sidecars (`pg_dbfile::remove_database`). Only ever called on a file this
/// build made (a replacement left by an interrupted rebuild), never on the catalog itself.
fn remove_database(path: &Path) -> Result<()> {
    pg_dbfile::remove_database(path).map_err(|e| Error::Other(anyhow!("{e}")))
}

/// The tables a rebuild carries over from the old catalog, in the order their foreign keys need (a table comes after the
/// tables it references). Every table of the current schema (the baseline's and the migrations') is here or in
/// [`TRANSIENT`]: none is derived, since a recomputed table would need a version mark to say when it is stale, and one
/// is added only with the change that needs it.
pub(crate) const CARRIED: &[&str] = &[
    "meta",
    "scans",
    "folders",
    "files",
    "analysis",
    "border_sides",
    "border_marks",
    "pdq_pairs",
    "ratings",
    "file_ratings",
    "file_writes",
    "roots",
    "folder_failures",
    "views",
    "session",
    "notices",
    "file_failures",
    "ledger_ack",
];

/// The tables whose rows hold only while the processes that wrote them run: a rebuilt catalog starts without them. The
/// scan leases (no walk runs across a rebuild, which needs the catalog to itself).
#[cfg_attr(not(test), expect(dead_code, reason = "a rebuild copies none of them; the test of `CARRIED` reads it"))]
pub(crate) const TRANSIENT: &[&str] = &["scan_leases"];

/// Copies the rows of the catalog at `old` into `conn`, a new catalog made from [`BASELINE`]: each table of [`CARRIED`],
/// by column name (a column the old file lacks takes its default), with its ids kept. The `meta` rows the new catalog
/// seeds are overwritten with the old values, except `path_rules`, which is this build's; a key the old file has and the
/// new one doesn't is not copied, so the keys stay the baseline's. The triggers are set aside for the copy, since they
/// would rewrite the copied rows' `pending` and `seq`, and are made again from their own text after it. All of it is one
/// transaction: a failure copies nothing, and the old file is not changed. `rules` are the rules the old file's paths
/// were stored under (the caller has checked they are this build's): a saved view that kept its folder as text (before
/// the folders were typed) is converted to the typed form as it is copied. Then every `Rust` step of the migrations
/// runs, in order, on the copy ([`Source::Carried`]): each finds the old shape it converts in the old file, so a file
/// of any version is converted as its migrations would have, and one already converted is left as it is.
pub(crate) fn carry_rows(conn: &mut Connection, old: &Path, rules: PathRules) -> Result<()> {
    // The old file's name as SQLite takes it, bound as a BLOB: its bytes on Unix, and on Windows the UTF-8 of the alias
    // for a folder with no UTF-8 name (`pg_platform::sqlite`).
    let bound =
        pg_platform::sqlite::attach_bytes(old).with_context(|| format!("{}: the old catalog", old.display()))?;
    conn.execute("ATTACH DATABASE ?1 AS carried", [bound]).context("attaching the old catalog")?;
    let copied = copy_tables(conn, rules);
    conn.execute_batch("DETACH DATABASE carried")?;
    copied
}

fn copy_tables(conn: &mut Connection, rules: PathRules) -> Result<()> {
    let triggers: Vec<(String, String)> = conn
        .prepare("SELECT name, sql FROM main.sqlite_schema WHERE type = 'trigger'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let tx = conn.transaction()?;
    for (name, _) in &triggers {
        tx.execute_batch(&format!("DROP TRIGGER main.\"{name}\""))?;
    }
    // The saved views' folders were text before they were typed (the stored form is the rules' bytes now).
    let text_folders = column_type(&tx, "carried", "views", "folder")?.is_some_and(|t| t.eq_ignore_ascii_case("TEXT"));
    // The queued writes' temp files were text too: a TEXT value stays TEXT in the BLOB column, so it is cast.
    let text_temps =
        column_type(&tx, "carried", "file_writes", "temp")?.is_some_and(|t| t.eq_ignore_ascii_case("TEXT"));
    for table in CARRIED {
        let new_columns = table_columns(&tx, "main", table)?;
        let old_columns = table_columns(&tx, "carried", table)?;
        let common: Vec<&str> =
            new_columns.iter().map(String::as_str).filter(|c| old_columns.iter().any(|o| o.as_str() == *c)).collect();
        if common.is_empty() {
            continue;
        }
        let columns = common.join(", ");
        if *table == "meta" {
            tx.execute(
                &format!(
                    "INSERT OR REPLACE INTO main.meta ({columns}) SELECT {columns} FROM carried.meta \
                     WHERE key IN (SELECT key FROM main.meta) AND key <> 'path_rules'"
                ),
                [],
            )?;
        } else if *table == "file_writes" {
            // A queued write whose value breaks its kind's rule (version 7's CHECK) was never valid: it is left out, as
            // the migration drops it. This is the copy obeying the current layout's constraint, not a conversion of
            // data: the insert can't take a row the CHECK refuses, so it is dropped here, before any `Rust` step runs.
            tx.execute(
                &format!("INSERT OR IGNORE INTO main.{table} ({columns}) SELECT {columns} FROM carried.{table}"),
                [],
            )?;
        } else {
            tx.execute(&format!("INSERT INTO main.{table} ({columns}) SELECT {columns} FROM carried.{table}"), [])?;
        }
    }
    if text_folders {
        convert_text_folders(&tx, rules)?;
    }
    // The migrations' conversions, in order, before the triggers are back (they would fire on the converted rows).
    for step in MIGRATIONS.iter().flat_map(|steps| steps.iter()) {
        if let Step::Rust(convert) = *step {
            convert(&tx, Source::Carried, rules)?;
        }
    }
    if text_temps {
        tx.execute("UPDATE main.file_writes SET temp = CAST(temp AS BLOB) WHERE temp IS NOT NULL", [])?;
    }
    for (_, sql) in &triggers {
        tx.execute_batch(sql)?;
    }
    tx.commit()?;
    Ok(())
}

/// The saved views' folders, copied as text from a catalog before S-17.8: each becomes the rules' stored bytes. The
/// text is split by the rules, so a folder that the old text named is the same folder; a NUL in it is refused, and the
/// copy is rolled back with the rest.
fn convert_text_folders(tx: &rusqlite::Transaction<'_>, rules: PathRules) -> Result<()> {
    let views: Vec<(String, Vec<u8>)> = tx
        .prepare("SELECT name, CAST(folder AS BLOB) FROM main.views")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (name, text) in views {
        let folder = rules
            .parse_merged(&text)
            .ok_or_else(|| Error::Other(anyhow!("saved view {name:?}: a NUL in its folder")))?;
        tx.execute(
            "UPDATE main.views SET folder = ?1 WHERE name = ?2",
            rusqlite::params![rules.join_merged(&folder), name],
        )?;
    }
    Ok(())
}
