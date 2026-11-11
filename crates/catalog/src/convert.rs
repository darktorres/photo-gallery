//! The data conversions of the migrations (`schema::Step::Rust`): what a migration does to rows rather than to the
//! layout. One implementation serves both paths a catalog takes forward: the migration (`schema::bring_to`, reading
//! the file being migrated, [`Source::Main`]) and the rebuild's copy (`rebuild::copy_tables`, reading the old file
//! attached as `carried`, [`Source::Carried`]), which runs every step of `schema::MIGRATIONS` in order after copying
//! the shared columns. A step finds what it converts by its shape (a column's presence or type), never by the version,
//! which a rebuilt file's schema can't be trusted for; each is idempotent, so a step whose old shape isn't there does
//! nothing. The rows are always written to `main`, in the current layout of the step's migration.
//!
//! The rebuild path runs an old step against the current schema: a later migration that drops or renames a column a
//! step here touches updates the step in the same change (the every-old-version rebuild tests are what holds it).
//!
//! A leaf: it uses only `listing` and `error`.

use anyhow::anyhow;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use pg_model::rules::PathRules;

use crate::error::{Error, Result};
use crate::listing::listing_values;

/// Where a step reads the rows it converts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// The file being migrated: its old shape is still there beside the new one, until the step after this one.
    Main,
    /// The old file a rebuild copies from, attached as `carried`; the shared columns are already in `main`.
    Carried,
}

impl Source {
    /// The schema name the old shape is read from.
    fn schema(self) -> &'static str {
        match self {
            Source::Main => "main",
            Source::Carried => "carried",
        }
    }
}

/// The column names of `table` in `schema` (`main` or an attached database); none when the table isn't there.
pub(crate) fn table_columns(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<Vec<String>> {
    conn.prepare("SELECT name FROM pragma_table_info(?1, ?2)")?.query_map([table, schema], |r| r.get(0))?.collect()
}

/// The declared type of `table.column` in `schema`; `None` when the column isn't there.
pub(crate) fn column_type(
    conn: &Connection,
    schema: &str,
    table: &str,
    column: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT type FROM pragma_table_info(?1, ?2) WHERE name = ?3", [table, schema, column], |r| r.get(0))
        .optional()
}

/// Version 5's rows: the session's text keys (`folder` as the JSON of the folder's parts, or, before S-17.8, the
/// folder's text; `recursive` as `true` or `false`; `query`; `sort`) become its one row, each key the old reader's
/// default where it was never saved. Nothing is written for a session never saved. The old shape is a `session` table
/// with a `key` column; the folders were text when `views.folder` was. On the migration the row goes to `session_row`,
/// the table version 5 makes beside the old one and renames after this step; on the rebuild, to `session`.
pub(crate) fn convert_session(tx: &Transaction<'_>, source: Source, rules: PathRules) -> Result<()> {
    let from = source.schema();
    if !table_columns(tx, from, "session")?.iter().any(|c| c == "key") {
        return Ok(());
    }
    let text_folders = column_type(tx, from, "views", "folder")?.is_some_and(|t| t.eq_ignore_ascii_case("TEXT"));
    let keys: Vec<(String, Vec<u8>)> = tx
        .prepare(&format!("SELECT key, CAST(value AS BLOB) FROM {from}.session"))?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if keys.is_empty() {
        return Ok(());
    }
    let value = |key: &str| keys.iter().find(|(k, _)| k == key).map(|(_, v)| String::from_utf8_lossy(v).into_owned());
    let folder = match keys.iter().find(|(k, _)| k == "folder") {
        Some((_, text)) if text_folders => {
            rules.parse_merged(text).ok_or_else(|| Error::Other(anyhow!("the session's folder has a NUL")))?
        }
        _ => pg_query::from_json(&value("folder").unwrap_or_default()),
    };
    let listing = pg_model::view::Listing {
        folder,
        recursive: value("recursive").is_none_or(|r| r == "true"),
        sort: pg_query::from_json(&value("sort").unwrap_or_default()),
        query: pg_query::from_json(&value("query").unwrap_or_default()),
    };
    let target = match source {
        Source::Main => "main.session_row",
        Source::Carried => "main.session",
    };
    let (folder, recursive, query, sort) = listing_values(rules, &listing);
    tx.execute(
        &format!("INSERT OR REPLACE INTO {target}(id, folder, recursive, query, sort) VALUES (1, ?1, ?2, ?3, ?4)"),
        params![folder, recursive, query, sort],
    )?;
    Ok(())
}

/// `TO_8_NOTIFY`'s rows (docs/NOTIFICATIONS.md):
/// - every time, idempotently: the retired scan notices go (they held counts, not paths, so they can't become rows of
///   the folder ledger, which the next walk fills); a `write-dropped` notice becomes an Event (no key, no part); every
///   failed analysis gets its import ledger row, if it has none (with serials in `(files.seq, file_id)` order, after
///   every serial taken); and each ledger row's `hidden` is computed from its file, which no trigger does on either path;
/// - once, from the shape before (a `ledger_ack.dismissed` time): a dismissal becomes the serial it was made at
///   (`dismissed_to`; none for an entry read after it was dismissed, which came back and must stay back), and the import
///   entry's marks, which were `files.seq` values, become the serials of the backfilled rows they covered.
pub(crate) fn convert_notify(tx: &Transaction<'_>, source: Source, _rules: PathRules) -> Result<()> {
    let from = source.schema();
    tx.execute("DELETE FROM main.notices WHERE kind IN ('scan-unreadable', 'mount-refused')", [])?;
    tx.execute("UPDATE main.notices SET key = NULL, part = NULL WHERE kind = 'write-dropped' AND key IS NOT NULL", [])?;
    let old_acks = table_columns(tx, from, "ledger_ack")?.iter().any(|c| c == "dismissed");
    if old_acks {
        tx.execute(
            &format!(
                "UPDATE main.ledger_ack SET dismissed_to = (
                     SELECT CASE WHEN o.dismissed IS NULL OR o.read > o.dismissed THEN NULL ELSE o.seen_to END
                     FROM {from}.ledger_ack o WHERE o.kind = ledger_ack.kind)"
            ),
            [],
        )?;
    }
    let backfill = backfill_import(tx)?;
    if old_acks {
        // A mark covered the failed files whose `seq` was at or below it; the serials follow `(seq, file_id)`, so the
        // newest of those is what was seen. None at or below it (relists and turns move `seq` on): everything before
        // the upgrade was seen, `base`. Never NULL: the column is NOT NULL.
        tx.execute(
            "UPDATE main.ledger_ack SET
                 seen_to = coalesce((SELECT max(l.serial) FROM main.file_failures l JOIN main.files f ON f.id = l.file_id
                                     WHERE l.id > ?1 AND f.seq <= ledger_ack.seen_to), ?2),
                 dismissed_to = CASE WHEN dismissed_to IS NULL THEN NULL ELSE
                     coalesce((SELECT max(l.serial) FROM main.file_failures l JOIN main.files f ON f.id = l.file_id
                               WHERE l.id > ?1 AND f.seq <= ledger_ack.dismissed_to), ?2) END
             WHERE kind = 'import'",
            params![backfill.max_id_before, backfill.base],
        )?;
    }
    tx.execute(
        "UPDATE main.file_failures SET hidden = h.v
         FROM (SELECT l.id AS id, (f.missing OR (l.pass = 'import' AND f.pending)) AS v
               FROM main.file_failures l JOIN main.files f ON f.id = l.file_id) AS h
         WHERE file_failures.id = h.id AND file_failures.hidden IS NOT h.v",
        [],
    )?;
    Ok(())
}

/// What [`backfill_import`] made: the rows after `max_id_before` are its, with serials from `base + 1` on.
struct Backfill {
    max_id_before: i64,
    base: i64,
}

/// A row of the import ledger for every failed analysis that has none, from the analysis's own record of the file
/// (`ON CONFLICT DO NOTHING`: an old file may hold some already, and one conflict would fail the whole copy). The
/// serials are stamped by one statement, whatever trigger is live (the migration's `file_failures_stamp` has stamped
/// them already; the rebuild's copy has none): `meta.seq` and then one each, in `(files.seq, file_id)` order, and
/// `meta.seq` moves past them once.
fn backfill_import(tx: &Transaction<'_>) -> Result<Backfill> {
    let max_id_before: i64 = tx.query_row("SELECT coalesce(max(id), 0) FROM main.file_failures", [], |r| r.get(0))?;
    tx.execute(
        "INSERT INTO main.file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen, hidden)
         SELECT a.file_id, 'import', a.mtime, a.size, a.rotation, a.error, 0, a.analyzed, (f.missing OR f.pending)
         FROM main.analysis a JOIN main.files f ON f.id = a.file_id WHERE a.error IS NOT NULL
         ON CONFLICT(file_id, pass) DO NOTHING",
        [],
    )?;
    let base: i64 = tx.query_row("SELECT value FROM main.meta WHERE key = 'seq'", [], |r| r.get(0))?;
    let stamped = tx.execute(
        "UPDATE main.file_failures SET serial = ?2 + r.n
         FROM (SELECT l.id AS id, row_number() OVER (ORDER BY f.seq, f.id) AS n
               FROM main.file_failures l JOIN main.files f ON f.id = l.file_id WHERE l.id > ?1) AS r
         WHERE file_failures.id = r.id",
        params![max_id_before, base],
    )?;
    let stamped = i64::try_from(stamped).map_err(|e| Error::Other(anyhow!("{e}: too many ledger rows")))?;
    tx.execute("UPDATE main.meta SET value = ?1 WHERE key = 'seq'", [base + stamped])?;
    Ok(Backfill { max_id_before, base })
}
