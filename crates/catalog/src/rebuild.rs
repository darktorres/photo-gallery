//! The catalog's rebuild, both halves: the file side (the replacement made beside the catalog, the old file moved aside,
//! the copy kept before a migration) and the row copy (`carry_rows`: every table's rows carried into the new layout, the
//! old shapes converted by the migrations' own `Rust` steps). `Catalog::prepare` decides when; this does it. Uses
//! `schema`, `convert`, `types` and `error`.

use std::path::{Path, PathBuf};

use anyhow::Context;
use pg_model::notice::Carried;
use pg_model::rules::PathRules;
use pg_platform::file::beside;
use rusqlite::Connection;

use crate::convert::{Source, table_columns};
use crate::error::{Error, Result};
use crate::schema::{Hooks, SETTINGS, SchemaSet, Step, bring_to, version_in};
use crate::types::{PREPARE_CACHE_KIB, Rebuilt};

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
pub(crate) fn rebuild(
    old: Connection,
    path: &Path,
    set: &SchemaSet<'_>,
    rules: PathRules,
    why: String,
    readable: bool,
) -> Result<Rebuilt> {
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
        Ok(()) => match make_replacement(&replacement, set, rules, Some(path)) {
            Ok((files, folders)) => Carried::Rows { files: count(files), folders: count(folders) },
            Err(e) => {
                // The copy is one transaction, so an error leaves no rows; the replacement is made again without them.
                remove_database(&replacement)?;
                make_replacement(&replacement, set, rules, None)?;
                Carried::Nothing { reason: format!("{e:#}") }
            }
        },
        Err(reason) => {
            make_replacement(&replacement, set, rules, None)?;
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
fn make_replacement(path: &Path, set: &SchemaSet<'_>, rules: PathRules, from: Option<&Path>) -> Result<(i64, i64)> {
    let mut conn = Connection::open(db_file(path)?).with_context(|| format!("catalog {}", path.display()))?;
    // An empty file takes incremental auto-vacuum at once; a catalog made here is then compacted in place, never as a whole.
    conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
    conn.execute_batch(SETTINGS)?;
    // The copy and the migrations are one-off work over whole tables: the small cache `Catalog::prepare` uses for them.
    conn.pragma_update(None, "cache_size", -PREPARE_CACHE_KIB)?;
    bring_to(&conn, set, rules, version_in(set)?, &Hooks::none())
        .with_context(|| format!("catalog {}", path.display()))?;
    let counts = match from {
        Some(old) => {
            carry_rows(&mut conn, old, set, rules)?;
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
    pg_dbfile::remove_database(path).map_err(Error::Io)
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
    // Keyed to `files` alone: its rows (each file's placeholder) are its own, and outlive a re-analysis.
    "placeholders",
    "analysis",
    // After `analysis`: its rows are keyed by the analyses'.
    "pairs_owed",
    "hashes_owed",
    "border_sides",
    "border_marks",
    "pdq_pairs",
    "ratings",
    "file_ratings",
    "file_writes",
    "roots",
    // A removal in progress is finished by the copy's catalog, as the old one would have finished it.
    "root_removals",
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

/// Copies the rows of the catalog at `old` into `conn`, a new catalog made from the schema: each table of [`CARRIED`],
/// by column name (a column the old file lacks takes its default), with its ids kept. The `meta` rows the new catalog
/// seeds are overwritten with the old values, except `path_rules`, which is this build's; a key the old file has and the
/// new one doesn't is not copied, so the keys stay the baseline's. The triggers are set aside for the copy, since they
/// would rewrite the copied rows' `pending` and `seq`, and are made again from their own text after it. All of it is one
/// transaction: a failure copies nothing, and the old file is not changed. `rules` are the rules the old file's paths
/// were stored under (the caller has checked they are this build's). Then every `Rust` step of the migrations runs, in
/// order, on the copy ([`Source::Carried`]): none before the first release, and each after it finds the old shape it
/// converts in the old file, so a conversion is idempotent.
pub(crate) fn carry_rows(conn: &mut Connection, old: &Path, set: &SchemaSet<'_>, rules: PathRules) -> Result<()> {
    // The old file's name as SQLite takes it, bound as a BLOB: its bytes on Unix, and on Windows the UTF-8 of the alias
    // for a folder with no UTF-8 name (`pg_platform::sqlite`).
    let bound =
        pg_platform::sqlite::attach_bytes(old).with_context(|| format!("{}: the old catalog", old.display()))?;
    conn.execute("ATTACH DATABASE ?1 AS carried", [bound]).context("attaching the old catalog")?;
    let copied = copy_tables(conn, set, rules);
    conn.execute_batch("DETACH DATABASE carried")?;
    copied
}

fn copy_tables(conn: &mut Connection, set: &SchemaSet<'_>, rules: PathRules) -> Result<()> {
    let triggers: Vec<(String, String)> = conn
        .prepare("SELECT name, sql FROM main.sqlite_schema WHERE type = 'trigger'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let tx = conn.transaction()?;
    for (name, _) in &triggers {
        tx.execute_batch(&format!("DROP TRIGGER main.\"{name}\""))?;
    }
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
            // The file's own `path_rules` is this build's, and its `vacuum_owed` is the one this file's creation set: the
            // copy's value for it is not the old file's.
            tx.execute(
                &format!(
                    "INSERT OR REPLACE INTO main.meta ({columns}) SELECT {columns} FROM carried.meta \
                     WHERE key IN (SELECT key FROM main.meta) AND key NOT IN ('path_rules', 'vacuum_owed')"
                ),
                [],
            )?;
        } else {
            // A plain insert: a row the layout refuses (a CHECK, a NOT NULL, a UNIQUE) is an error that rolls the copy back,
            // not a row dropped. The old file has this build's layout, so none is expected.
            tx.execute(&format!("INSERT INTO main.{table} ({columns}) SELECT {columns} FROM carried.{table}"), [])?;
        }
    }
    // The migrations' conversions, in order, before the triggers are back (they would fire on the converted rows).
    for step in set.migrations.iter().flat_map(|migration| migration.steps.iter()) {
        if let Step::Rust(convert) = *step {
            convert(&tx, Source::Carried, rules)?;
        }
    }
    carry_sequences(&tx)?;
    for (_, sql) in &triggers {
        tx.execute_batch(sql)?;
    }
    tx.commit()?;
    Ok(())
}

/// The ids' high-water marks of the old catalog (`sqlite_sequence`, of the `AUTOINCREMENT` tables) are carried over: the
/// copy's sequence is its largest copied id, so an id the old catalog had freed above that would be handed out again.
/// The sequence is raised to the old one's, and never lowered; a table without a row there (none freed yet) is given
/// the old one's. A file with no `sqlite_sequence` (none of its tables is AUTOINCREMENT) has nothing to carry.
fn carry_sequences(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    let old_has_sequences: i64 =
        tx.query_row("SELECT count(*) FROM carried.sqlite_schema WHERE name = 'sqlite_sequence'", [], |r| r.get(0))?;
    if old_has_sequences == 0 {
        return Ok(());
    }
    for table in ["files", "folders", "roots"] {
        tx.execute(
            "UPDATE main.sqlite_sequence SET seq = max(seq, (SELECT seq FROM carried.sqlite_sequence WHERE name = ?1)) \
             WHERE name = ?1 AND EXISTS (SELECT 1 FROM carried.sqlite_sequence WHERE name = ?1)",
            [table],
        )?;
        tx.execute(
            "INSERT INTO main.sqlite_sequence(name, seq) SELECT ?1, seq FROM carried.sqlite_sequence WHERE name = ?1 \
             AND NOT EXISTS (SELECT 1 FROM main.sqlite_sequence WHERE name = ?1)",
            [table],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Hooks, SCHEMA, bring_to, current_version};
    use pg_platform::native::PATH_RULES;
    use pg_testkit::run_folder;

    /// A fresh catalog file at `path`, made as `prepare` makes one: with incremental auto-vacuum when `incremental`, and
    /// without it otherwise (then the file owes a VACUUM).
    fn made(path: &Path, incremental: bool) -> Connection {
        let conn = Connection::open(path).unwrap();
        if incremental {
            conn.pragma_update(None, "auto_vacuum", "INCREMENTAL").unwrap();
        }
        bring_to(&conn, &SCHEMA, PATH_RULES, current_version().unwrap(), &Hooks::none()).unwrap();
        conn
    }

    fn vacuum_owed(conn: &Connection) -> i64 {
        conn.query_row("SELECT value FROM meta WHERE key = 'vacuum_owed'", [], |r| r.get(0)).unwrap()
    }

    /// The copy keeps the new file's own `vacuum_owed`: the old file's mark (set when it was made without incremental
    /// auto-vacuum) is not carried over, since the new file was made with it.
    #[test]
    fn the_copy_keeps_the_new_files_vacuum_mark() {
        let dir = run_folder("scratch_catalog", "vacuum-mark").unwrap();
        let old_path = dir.join("old.sqlite");
        drop(made(&old_path, false));
        assert_eq!(vacuum_owed(&Connection::open(&old_path).unwrap()), 1, "the old file owes a vacuum");
        let new_path = dir.join("new.sqlite");
        let mut new = made(&new_path, true);
        assert_eq!(vacuum_owed(&new), 0);
        carry_rows(&mut new, &old_path, &SCHEMA, PATH_RULES).unwrap();
        assert_eq!(vacuum_owed(&new), 0, "the copy did not carry the old file's mark");
        drop(new);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A queued write the layout refuses (its value out of its kind's range: the CHECK) is an error that rolls the copy
    /// back, not a row dropped: the old file has such a row only because its constraints were off.
    #[test]
    fn a_queued_write_the_layout_refuses_is_an_error_not_a_dropped_row() {
        let dir = run_folder("scratch_catalog", "refused-write").unwrap();
        let old_path = dir.join("old.sqlite");
        {
            let old = made(&old_path, true);
            old.execute_batch(
                "PRAGMA ignore_check_constraints = ON; PRAGMA foreign_keys = OFF;
                 INSERT INTO file_writes(file_id, kind, value, queued) VALUES (1, 'rating', 9, 0);",
            )
            .unwrap();
        }
        let new_path = dir.join("new.sqlite");
        let mut new = made(&new_path, true);
        let error = carry_rows(&mut new, &old_path, &SCHEMA, PATH_RULES).unwrap_err().to_string();
        assert!(error.contains("CHECK"), "{error}");
        let rows: i64 = new.query_row("SELECT count(*) FROM file_writes", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0, "the copy is rolled back");
        drop(new);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
