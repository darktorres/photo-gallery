//! The catalog's rebuild, both halves: the file side (the replacement made beside the catalog, the old file moved aside,
//! the copy kept before a migration) and the row copy (`carry_rows`: every table's rows carried into the new layout, the
//! old shapes converted by the migrations' own `Rust` steps). `Catalog::prepare` decides when; this does it. Uses
//! `schema`, `convert`, `types` and `error`.

use std::path::{Path, PathBuf};

use pg_model::notice::Carried;
use pg_model::rules::PathRules;
use pg_platform::file::beside;
use rusqlite::Connection;

use crate::convert::{Source, table_columns};
use crate::error::{Error, Result};
use crate::schema::{Hooks, SchemaSet, Step, apply_settings, bring_to, version_in};
use crate::transaction::checkpoint_truncate;
use crate::types::{PREPARE_CACHE_KIB, Rebuilt};

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
    checkpoint_truncate(conn)?;
    let partial = beside(&backup, ".partial");
    std::fs::copy(db_file(path)?, &partial)
        .and_then(|_| std::fs::rename(&partial, &backup))
        .map_err(|e| Error::step(format!("copying the catalog to {} before it is migrated", backup.display()), e))?;
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
        checkpoint_truncate(&old)?;
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
            Ok((files, folders)) => Carried::Rows { files, folders },
            Err(e) => {
                // The copy is one transaction, so an error leaves no rows; the replacement is made again without them.
                remove_database(&replacement)?;
                make_replacement(&replacement, set, rules, None)?;
                Carried::Nothing { reason: pg_model::error::chain(&e) }
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
        .map_err(|e| Error::step(format!("catalog {}: moving it aside ({why})", path.display()), e))?;
    std::fs::rename(&replacement, path).map_err(|e| {
        Error::step(
            format!("catalog {}: its new file {} can't take the name", path.display(), replacement.display()),
            e,
        )
    })?;
    Ok(Rebuilt { kept, why, carried })
}

/// The file SQLite opens for the catalog at `path`: the path itself, which the caller gave in a folder SQLite can open
/// (`pg_platform::sqlite::openable_folder`).
pub(crate) fn db_file(path: &Path) -> Result<PathBuf> {
    Ok(path.to_path_buf())
}

/// Make the catalog file `path` in this build's layout, with the rows of `from` copied into it when given
/// ([`carry_rows`]). Returns its file and library folder counts (zero without `from`). The file is closed, with its
/// changes in the main file.
fn make_replacement(path: &Path, set: &SchemaSet<'_>, rules: PathRules, from: Option<&Path>) -> Result<(u64, u64)> {
    let mut conn = Connection::open(db_file(path)?)
        .map_err(|e| Error::step(format!("catalog {}: making its replacement", path.display()), e))?;
    // An empty file takes incremental auto-vacuum at once; a catalog made here is then compacted in place, never as a whole.
    conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
    apply_settings(&conn)?;
    // The copy and the migrations are one-off work over whole tables: the small cache `Catalog::prepare` uses for them.
    conn.pragma_update(None, "cache_size", -PREPARE_CACHE_KIB)?;
    bring_to(&conn, set, rules, version_in(set)?, &Hooks::none()).map_err(|e| {
        Error::step(format!("catalog {}: bringing its replacement to this build's schema", path.display()), e)
    })?;
    let counts = match from {
        Some(old) => {
            carry_rows(&mut conn, old, set, rules)?;
            // The copy fills the indexes by row (in id order, not key order), and the keys' recompute moves every file's
            // index entries: the pages are part full (21 MiB of 214 on the owner's catalog). The start compacts the file
            // once (`lifecycle::vacuum_if_owed`, under the exclusive hold the rebuild has, with its room check): a
            // `VACUUM` copies each index in key order (193 MiB then). A failure there keeps the mark for the next start,
            // where one here would fail the rebuild and carry nothing.
            conn.execute("UPDATE meta SET value = 1 WHERE key = 'vacuum_owed'", [])?;
            // A `COUNT(*)` is a `u64` read: a negative one is an error, not a 0.
            let count = |table: &str| -> rusqlite::Result<u64> {
                conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            };
            (count("files")?, count("roots")?)
        }
        None => (0, 0),
    };
    checkpoint_truncate(&conn)?;
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
/// `TRANSIENT` (a test constant): none is derived, since a recomputed table would need a version mark to say when it is stale, and one
/// is added only with the change that needs it.
pub(crate) const CARRIED: &[&str] = &[
    "meta",
    "scans",
    // Before the files, which hold its ids: the old file's, or made from its files' serials ([`converted`]).
    "volumes",
    // Made, not copied: one placeholder the folders hold until the keys are made ([`crate::lifecycle::recompute_keys`]).
    "mfolders",
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
    // Work the app has promised and not yet done: a scan asked for, a temp to remove.
    "scan_requests",
    "temp_leftovers",
];

/// The tables whose rows hold only while the processes that wrote them run: a rebuilt catalog starts without them. The
/// scan leases (no walk runs across a rebuild, which needs the catalog to itself).
/// (Only the test of `CARRIED` reads it: a rebuild copies none of these, so outside the tests the list has no reader.)
#[cfg(test)]
pub(crate) const TRANSIENT: &[&str] = &["scan_leases"];

/// Copies the rows of the catalog at `old` into `conn`, a new catalog made from the schema: each table of [`CARRIED`],
/// by column name (a column the old file lacks takes its default), with its ids kept. The `meta` rows the new catalog
/// seeds are overwritten with the old values, except `path_rules` (this build's), `vacuum_owed` (the new file's own, as
/// its creation set it) and `key_version` (made again below); a key the old file has and the new one doesn't is not
/// copied, so the keys stay the baseline's. The triggers are set aside for the copy, since they
/// would rewrite the copied rows' `pending` and `seq`, and are made again from their own text after it. All of it is one
/// transaction: a failure copies nothing, and the old file is not changed. `rules` are the rules the old file's paths
/// were stored under (the caller has checked they are this build's). Then every `Rust` step of the migrations runs, in
/// order, on the copy ([`Source::Carried`]): none before the first release, and each after it finds the old shape it
/// converts in the old file, so a conversion is idempotent.
pub(crate) fn carry_rows(conn: &mut Connection, old: &Path, set: &SchemaSet<'_>, rules: PathRules) -> Result<()> {
    // The old file's name as SQLite takes it, bound as a BLOB: its bytes on Unix, and on Windows the UTF-8 of the alias
    // for a folder with no UTF-8 name (`pg_platform::sqlite`).
    let bound = pg_platform::sqlite::attach_bytes(old)
        .map_err(|e| Error::step(format!("{}: the old catalog", old.display()), e))?;
    conn.execute("ATTACH DATABASE ?1 AS carried", [bound]).map_err(|e| Error::step("attaching the old catalog", e))?;
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
    let old_has_volumes = !table_columns(&tx, "carried", "volumes")?.is_empty();
    let mut placeholder_mid = None;
    for table in CARRIED {
        let new_columns = table_columns(&tx, "main", table)?;
        let old_columns = table_columns(&tx, "carried", table)?;
        if *table == "mfolders" {
            // The merged folders are made from the folders' paths, whatever the old file holds: one row with a key no
            // folder has (a key is empty or ends in 0x01, and no name holds a NUL) until then.
            placeholder_mid =
                Some(tx.query_row("INSERT INTO main.mfolders(key) VALUES (x'00') RETURNING id", [], |r| {
                    r.get::<_, i64>(0)
                })?);
            continue;
        }
        if *table == "volumes" && !old_has_volumes {
            // An older layout held each file's serial in `files.volume`: its volumes are those serials.
            if table_columns(&tx, "carried", "files")?.iter().any(|c| c == "volume") {
                tx.execute(
                    "INSERT INTO main.volumes(serial) SELECT DISTINCT volume FROM carried.files WHERE volume IS NOT NULL",
                    [],
                )?;
            }
            continue;
        }
        if matches!(*table, "folders" | "folder_failures") && !old_columns.is_empty() {
            copy_folder_rows(&tx, table, &old_columns, placeholder_mid)?;
            continue;
        }
        // Each column of the new layout the old file has, or one a conversion makes from the old file's columns.
        let mut columns: Vec<&str> = Vec::new();
        let mut values: Vec<String> = Vec::new();
        for column in &new_columns {
            if let Some(value) = converted(table, column, &old_columns, old_has_volumes) {
                columns.push(column);
                values.push(value);
            } else if old_columns.iter().any(|o| o == column) {
                columns.push(column);
                values.push(column.clone());
            }
        }
        if columns.is_empty() {
            continue;
        }
        let (columns, values) = (columns.join(", "), values.join(", "));
        if *table == "meta" {
            // The file's own `path_rules` is this build's, and its `vacuum_owed` is the one this file's creation set: the
            // copy's value for it is not the old file's. Its `key_version` is not the old file's either: the keys are made
            // below, by this build's algorithm, and the mark is set for them.
            tx.execute(
                &format!(
                    "INSERT OR REPLACE INTO main.meta ({columns}) SELECT {columns} FROM carried.meta \
                     WHERE key IN (SELECT key FROM main.meta) AND key NOT IN ('path_rules', 'vacuum_owed', 'key_version')"
                ),
                [],
            )?;
        } else {
            // A plain insert: a row the layout refuses (a CHECK, a NOT NULL, a UNIQUE) is an error that rolls the copy back,
            // not a row dropped. The old file has this build's layout, so none is expected.
            tx.execute(&format!("INSERT INTO main.{table} ({columns}) SELECT {values} FROM carried.{table}"), [])?;
        }
    }
    // An extension the old file named and this build doesn't hold has no code ([`converted`]'s -1): its files are
    // missing, as `lifecycle::mark_unheld_missing` makes a later build's.
    tx.execute("UPDATE main.files SET missing = 1 WHERE ext < 0 AND missing = 0", [])?;
    // The folders' and the merged folders' keys are made by this build's algorithm from the paths, merging the folders
    // the rules make one (the placeholders [`copy_folder_rows`] gave them are replaced here, and the files follow).
    crate::lifecycle::recompute_keys(&tx, rules)?;
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

/// The value a carried row's `column` of `table` takes when the old layout stored it in another shape, as an expression
/// over the old row: `None` when the column is copied as it is (or the old file has nothing it is made from). Each
/// expression reads both shapes (a value already of this layout's shape is kept), so a catalog of this layout carries
/// through them unchanged.
///
/// - `files.volume`: the id of the volume of the old file's serial, when the old file had no `volumes` table.
/// - `files.mid`: the merged folder of the row's folder (the placeholder until the keys are made).
/// - `files.ext`: the extension's code ([`pg_model::media::Ext::code`]) for its old name; -1 for a name this build
///   doesn't hold.
/// - `files.kind`, `analysis.kind`: the kind's code for `image` and `video`.
/// - `analysis.format`, `analysis.codec`: an image's format code for its old name; a video's old `format` (its codec's
///   name) is its `codec`.
/// - `border_sides.edge`, `.kind`, `.class`, `.rejected`: the codes for their old names.
fn converted(table: &str, column: &str, old: &[String], old_has_volumes: bool) -> Option<String> {
    use pg_model::detection::{Class, Edge, Kind as SideKind, Reason};
    use pg_model::format::Format;
    use pg_model::media::Ext;
    let has = |c: &str| old.iter().any(|o| o == c);
    // `CASE column WHEN 'name' THEN code … ELSE other END` for every code a type has, `other` the column itself (already a
    // code) or what a name with no code becomes.
    fn names<T>(column: &str, from_code: fn(i64) -> Option<T>, key: fn(T) -> &'static str, other: &str) -> String {
        let arms: String =
            (0..256).filter_map(|code| from_code(code).map(|v| format!(" WHEN '{}' THEN {code}", key(v)))).collect();
        format!("CASE {column}{arms} ELSE {other} END")
    }
    let kind = "CASE kind WHEN 'image' THEN 0 WHEN 'video' THEN 1 ELSE kind END";
    match (table, column) {
        ("files", "volume") if has("volume") && !old_has_volumes => {
            Some("(SELECT id FROM main.volumes WHERE serial = volume)".to_string())
        }
        ("files", "mid") => Some("(SELECT mid FROM main.folders WHERE id = folder_id)".to_string()),
        ("files", "ext") if has("ext") => Some(format!(
            "CASE WHEN typeof(ext) = 'text' THEN {} ELSE ext END",
            names("lower(ext)", Ext::from_code, Ext::key, "-1")
        )),
        ("files" | "analysis", "kind") if has("kind") => Some(kind.to_string()),
        ("analysis", "format") if has("format") && has("kind") => Some(format!(
            "CASE WHEN typeof(format) <> 'text' THEN format WHEN kind IN ('video', 1) THEN NULL ELSE {} END",
            names("format", Format::from_code, Format::key, "NULL")
        )),
        ("analysis", "codec") if !has("codec") && has("format") && has("kind") => {
            Some("CASE WHEN typeof(format) = 'text' AND kind IN ('video', 1) THEN format END".to_string())
        }
        ("border_sides", "edge") if has("edge") => Some(names("edge", Edge::from_code, Edge::key, "edge")),
        ("border_sides", "kind") if has("kind") => Some(names("kind", SideKind::from_code, SideKind::key, "kind")),
        ("border_sides", "class") if has("class") => Some(names("class", Class::from_code, Class::key, "class")),
        ("border_sides", "rejected") if has("rejected") => {
            Some(names("rejected", Reason::from_code, Reason::key, "rejected"))
        }
        _ => None,
    }
}

/// The folder tables are copied with their ids kept (the files and the ledger point at them), each row's path as bytes (the
/// old layout stored it as text), and its keys as placeholders: a `path_key` unique to the row (its id's digits), and, for
/// the folders, the placeholder merged folder `mid`. The keys are made from the paths, the folders the new rules make one
/// are merged, and each folder is moved to its merged folder, by [`crate::lifecycle::recompute_keys`], which the copy runs
/// next.
fn copy_folder_rows(tx: &rusqlite::Transaction<'_>, table: &str, old: &[String], mid: Option<i64>) -> Result<()> {
    let new = table_columns(tx, "main", table)?;
    let mut columns: Vec<String> = Vec::new();
    let mut values: Vec<String> = Vec::new();
    for column in &new {
        if column == "path_key" || column == "mid" || !old.iter().any(|o| o == column) {
            continue;
        }
        columns.push(column.clone());
        values.push(if column == "path" { "CAST(path AS BLOB)".to_string() } else { column.clone() });
    }
    // The id's decimal digits: unique to the row, and never a key a folder gets (a key ends in a terminator byte).
    columns.push("path_key".to_string());
    values.push("CAST(id AS BLOB)".to_string());
    if new.iter().any(|c| c == "mid") {
        let mid = mid.ok_or(Error::Stored { what: "the merged folders are made before the folders" })?;
        columns.push("mid".to_string());
        values.push(mid.to_string());
    }
    tx.execute(
        &format!("INSERT INTO main.{table} ({}) SELECT {} FROM carried.{table}", columns.join(", "), values.join(", ")),
        [],
    )?;
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

    /// The old layout keyed a folder by its exact path, so one folder spelled in two cases was two rows. Carried, the two
    /// are one folder: the lower id is kept, both folders' files are that folder's, and its key is the rules' key for
    /// its path. (Windows: the rules fold case.)
    #[cfg(windows)]
    #[test]
    fn folders_spelled_in_two_cases_carry_over_as_one_folder() {
        let dir = run_folder("scratch_catalog", "carry-cases").unwrap();
        let old_path = dir.join("old.sqlite");
        {
            let old = Connection::open(&old_path).unwrap();
            old.execute_batch(
                "CREATE TABLE folders(id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, key BLOB NOT NULL);
                 CREATE TABLE files(id INTEGER PRIMARY KEY, volume INTEGER, file_ref INTEGER, folder_id INTEGER NOT NULL,
                     name BLOB NOT NULL, ext TEXT NOT NULL, kind TEXT NOT NULL, size INTEGER NOT NULL,
                     mtime INTEGER NOT NULL, order_key BLOB NOT NULL);
                 INSERT INTO folders VALUES (1, 'D:\\p', x'00'), (2, 'd:\\P', x'00');
                 INSERT INTO files VALUES (1, 1, 1, 1, x'61', 'jpg', 'image', 1, 1, x'61'),
                     (2, 1, 2, 2, x'62', 'jpg', 'image', 1, 1, x'62');",
            )
            .unwrap();
        }
        let new_path = dir.join("new.sqlite");
        let mut new = made(&new_path, true);
        carry_rows(&mut new, &old_path, &SCHEMA, PATH_RULES).unwrap();
        let folders: Vec<(i64, String)> = {
            let mut stmt = new.prepare("SELECT id, CAST(path AS TEXT) FROM folders ORDER BY id").unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
        };
        assert_eq!(folders, [(1, "D:\\p".to_string())], "one folder, the lower id kept");
        let files: Vec<(i64, i64)> = {
            let mut stmt = new.prepare("SELECT id, folder_id FROM files ORDER BY id").unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
        };
        assert_eq!(files, [(1, 1), (2, 1)], "both files are the folder's");
        let key: Vec<u8> = new.query_row("SELECT path_key FROM folders WHERE id = 1", [], |r| r.get(0)).unwrap();
        let folder = PATH_RULES.parse(r"D:\p").unwrap();
        assert_eq!(key, PATH_RULES.path_key(&folder));
        drop(new);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The layout before the size plan (`docs/plans/CATALOG_SIZE_PLAN.md`) held names where this one holds codes, each
    /// file's volume serial, and stored keys (`folders.match_key`, `files.order_key`); a video's codec was its
    /// `analysis.format`. Carried, each name is its code, the serials are interned, the keys are made from the paths
    /// (two drives' copies of a folder are one merged folder), and the codec is in its own column. An extension this
    /// build doesn't hold leaves its file missing. (Windows: the paths are drive paths.)
    #[cfg(windows)]
    #[test]
    fn the_layout_before_the_codes_carries_over_converted() {
        use pg_model::detection::{Class, Edge, Kind as SideKind, Reason};
        use pg_model::format::Format;
        use pg_model::media::Ext;
        let dir = run_folder("scratch_catalog", "carry-codes").unwrap();
        let old_path = dir.join("old.sqlite");
        {
            let old = Connection::open(&old_path).unwrap();
            old.execute_batch(
                r"CREATE TABLE folders(id INTEGER PRIMARY KEY, path BLOB NOT NULL, path_key BLOB NOT NULL UNIQUE,
                     match_key BLOB NOT NULL);
                 CREATE TABLE files(id INTEGER PRIMARY KEY, volume INTEGER, file_ref INTEGER, folder_id INTEGER NOT NULL,
                     name BLOB NOT NULL, ext TEXT NOT NULL, kind TEXT NOT NULL, size INTEGER NOT NULL,
                     mtime INTEGER NOT NULL, missing INTEGER NOT NULL DEFAULT 0, order_key BLOB NOT NULL);
                 CREATE TABLE analysis(file_id INTEGER PRIMARY KEY, version INTEGER NOT NULL, size INTEGER NOT NULL,
                     mtime INTEGER NOT NULL, rotation INTEGER NOT NULL, kind TEXT NOT NULL, analyzed INTEGER NOT NULL,
                     format TEXT);
                 CREATE TABLE border_sides(file_id INTEGER NOT NULL, pass INTEGER NOT NULL, edge TEXT NOT NULL,
                     kind TEXT NOT NULL, class TEXT NOT NULL, depth INTEGER NOT NULL, straight REAL NOT NULL,
                     rejected TEXT, PRIMARY KEY(file_id, pass, edge)) WITHOUT ROWID;
                 INSERT INTO folders VALUES (1, CAST('D:\p' AS BLOB), x'01', x'00'), (2, CAST('E:\P' AS BLOB), x'02', x'00');
                 INSERT INTO files VALUES (1, 9000000000, 1, 1, x'612e6a7067', 'jpg', 'image', 1, 1, 0, x'00'),
                     (2, -5, 2, 2, x'622e6d7034', 'mp4', 'video', 1, 1, 0, x'00'),
                     (3, 9000000000, 3, 1, x'632e787878', 'xxx', 'image', 1, 1, 0, x'00');
                 INSERT INTO analysis VALUES (1, 1, 1, 1, 0, 'image', 1, 'jpeg'), (2, 1, 1, 1, 0, 'video', 1, 'h264');
                 INSERT INTO border_sides VALUES (1, 0, 'left', 'blur', 'blur', 3, 1.0, 'no_seam');",
            )
            .unwrap();
        }
        let new_path = dir.join("new.sqlite");
        let mut new = made(&new_path, true);
        carry_rows(&mut new, &old_path, &SCHEMA, PATH_RULES).unwrap();
        let files: Vec<(i64, i64, i64, i64, i64)> = {
            let mut stmt = new
                .prepare("SELECT f.id, v.serial, f.ext, f.kind, f.missing FROM files f JOIN volumes v ON v.id = f.volume ORDER BY f.id")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let code = |e: &str| Ext::of(e).unwrap().code();
        assert_eq!(files, [(1, 9000000000, code("jpg"), 0, 0), (2, -5, code("mp4"), 1, 0), (3, 9000000000, -1, 0, 1)]);
        let volumes: i64 = new.query_row("SELECT count(*) FROM volumes", [], |r| r.get(0)).unwrap();
        assert_eq!(volumes, 2, "one row a serial");
        // Both drives' `p` are one merged folder, of the rules' key, and the files are in it.
        let merged: Vec<(i64, Vec<u8>)> = {
            let mut stmt = new.prepare("SELECT id, key FROM mfolders").unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
        };
        assert_eq!(merged.len(), 1, "{merged:?}");
        assert_eq!(merged[0].1, PATH_RULES.folder_key(&PATH_RULES.parse(r"D:\p").unwrap().parts));
        let elsewhere: i64 =
            new.query_row("SELECT count(*) FROM files WHERE mid <> ?1", [merged[0].0], |r| r.get(0)).unwrap();
        assert_eq!(elsewhere, 0);
        let analysis: Vec<(i64, Option<i64>, Option<String>)> = {
            let mut stmt = new.prepare("SELECT kind, format, codec FROM analysis ORDER BY file_id").unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect()
        };
        assert_eq!(analysis, [(0, Some(Format::Jpeg.code()), None), (1, None, Some("h264".to_string()))]);
        let side: (i64, i64, i64, Option<i64>) = new
            .query_row("SELECT edge, kind, class, rejected FROM border_sides", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .unwrap();
        assert_eq!(side, (Edge::Left.code(), SideKind::Blur.code(), Class::Blur.code(), Some(Reason::NoSeam.code())));
        let check: String = new.query_row("PRAGMA foreign_key_check", [], |r| r.get(0)).unwrap_or_else(|_| "ok".into());
        assert_eq!(check, "ok", "no row breaks a foreign key");
        drop(new);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
