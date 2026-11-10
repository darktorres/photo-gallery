//! The failure ledger (docs/NOTIFICATIONS.md): the files a pass couldn't make its result of, for the state each file
//! was in. The thumbnails' and the borders' passes write it (`file_failures`, migration `TO_4`); the import's failures
//! are its failed analyses (`analysis.error`), read as they are. Each pass's ledger is one inbox entry, built when the
//! inbox is read ([`entries`]): shown while the pass has failures the user hasn't dismissed, unread while some came after
//! what the user last saw of it (`ledger_ack`). Uses `core` (the connection) and `schema` (the write transaction).
//!
//! A row describes the file as it failed: a change to the file (its size, time or turn), or the file going missing,
//! removes it (the trigger `files_failures_stale`), and so does a pass that makes the file's result after all
//! ([`clear`]). A failure that holds writes nothing when it is recorded again ([`record`]): it keeps its serial, so it is
//! not news again.

use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension, params};

use pg_model::ids::FileId;
use pg_model::notice::{FilePass, LedgerCounts, Notice};
use pg_model::rules::Path as RulePath;
use pg_model::turns::Turns;

use crate::Catalog;
use crate::error::Result;
use crate::schema::write_transaction;

/// The longest error text kept (bytes; cut on a character's boundary).
pub const ERROR_BYTES: usize = 1024;

/// A file a pass couldn't make its result of, as the pass read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub file_id: FileId,
    /// The file's state when the pass read it: the row is kept only while the catalog still has the file so.
    pub mtime: i64,
    pub size: i64,
    pub rotation: Turns,
    pub error: String,
    /// Whether the pass tries the file again by itself (a decode error, which a pass tries until the file changes), or
    /// not until it changes or the user retries it (a panic).
    pub retry: bool,
}

/// A pass's ledger as the inbox shows it: one entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    pub pass: FilePass,
    /// How many files fail, and how many of them came after what the user last saw.
    pub total: u64,
    pub new: u64,
    /// The newest failure's serial: what the user sees of the entry when it is listed ([`mark_read`], [`dismiss`]).
    pub newest: i64,
    /// When the first and the newest failures were recorded (Unix seconds).
    pub created: i64,
    pub updated: i64,
    pub read: bool,
}

impl Summary {
    /// The entry's notice: with its counts once some failures are new and some were seen.
    pub fn notice(&self) -> Notice {
        let earlier = self.total - self.new;
        let counts = (self.new > 0 && earlier > 0).then_some(LedgerCounts { new: self.new, earlier });
        match self.pass {
            FilePass::Thumbs => Notice::ThumbsFailed { counts },
            FilePass::Borders => Notice::BordersFailed { counts },
            FilePass::Import => Notice::ImportFailed { counts },
        }
    }
}

/// A failed file, as the inbox's expanded entry lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedFile {
    pub file_id: FileId,
    /// The file's path, exact.
    pub path: RulePath,
    pub error: String,
    /// The pass tries it again by itself; else not until it changes, or the user retries it.
    pub retry: bool,
    /// When it was first recorded (Unix seconds).
    pub first_seen: i64,
    /// Whether it came after what the user last saw of the entry.
    pub new: bool,
}

/// Record `failures` of `pass`, found at `now`: one row per file, kept only while the catalog has the file as the pass
/// read it (one changed or gone since is not recorded). A failure recorded already is updated only if its error or its
/// retry changed, and keeps its serial. One statement for them all.
pub fn record(catalog: &Catalog, pass: FilePass, failures: &[Failure], now: i64) -> Result<()> {
    if failures.is_empty() {
        return Ok(());
    }
    let rows: Vec<serde_json::Value> = failures
        .iter()
        .map(|f| {
            serde_json::json!([f.file_id.get(), f.mtime, f.size, f.rotation.get(), tidy(&f.error), i64::from(f.retry)])
        })
        .collect();
    let rows = serde_json::to_string(&rows).map_err(anyhow::Error::from)?;
    let tx = write_transaction(&catalog.conn)?;
    tx.execute(
        "INSERT INTO file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen)
         SELECT f.id, ?1, f.mtime, f.size, f.rotation, j.value ->> 4, j.value ->> 5, ?3
         FROM json_each(?2) j CROSS JOIN files f ON f.id = j.value ->> 0
         WHERE f.mtime = j.value ->> 1 AND f.size = j.value ->> 2 AND f.rotation = j.value ->> 3 AND f.missing = 0
         ON CONFLICT(file_id, pass) DO UPDATE SET error = excluded.error, retry = excluded.retry
         WHERE file_failures.error IS NOT excluded.error OR file_failures.retry IS NOT excluded.retry",
        params![pass.code(), rows, now],
    )?;
    tx.commit()?;
    Ok(())
}

/// The files `pass`'s ledger holds: a pass reads them when it starts, to [`clear`] only those it then makes.
pub fn recorded(catalog: &Catalog, pass: FilePass) -> Result<BTreeSet<FileId>> {
    let mut stmt = catalog.conn.prepare_cached("SELECT file_id FROM file_failures WHERE pass = ?1")?;
    let ids = stmt.query_map([pass.code()], |r| r.get::<_, i64>(0))?;
    Ok(ids.map(|id| id.map(FileId::new)).collect::<rusqlite::Result<_>>()?)
}

/// Whether `pass`'s ledger holds any file (dismissed or not).
pub fn any(catalog: &Catalog, pass: FilePass) -> Result<bool> {
    Ok(catalog
        .conn
        .query_row("SELECT EXISTS(SELECT 1 FROM file_failures WHERE pass = ?1)", [pass.code()], |r| r.get(0))?)
}

/// The results of `files` were made: their failures are gone. Nothing is written when the ledger holds none of them (it
/// is looked at first, so a clear of what isn't there doesn't take the write lock).
pub fn clear(catalog: &Catalog, pass: FilePass, files: &[FileId]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let ids = serde_json::to_string(&files.iter().map(|f| f.get()).collect::<Vec<_>>()).map_err(anyhow::Error::from)?;
    let held: bool = catalog.conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM file_failures WHERE pass = ?1 AND file_id IN (SELECT value FROM json_each(?2)))",
        params![pass.code(), ids],
        |r| r.get(0),
    )?;
    if !held {
        return Ok(());
    }
    let tx = write_transaction(&catalog.conn)?;
    tx.execute(
        "DELETE FROM file_failures WHERE pass = ?1 AND file_id IN (SELECT value FROM json_each(?2))",
        params![pass.code(), ids],
    )?;
    tx.commit()?;
    Ok(())
}

/// Try every failed file of `pass` again: the pass's next run lists them (their `seq` moves on), and the rows go, so
/// what still fails is recorded anew. The user's Retry. The import's failures are retried by `gallery import --retry`.
pub fn retry(catalog: &Catalog, pass: FilePass) -> Result<()> {
    if pass == FilePass::Import {
        return Ok(());
    }
    let tx = write_transaction(&catalog.conn)?;
    tx.execute_batch(next_seq!())?;
    tx.execute(failures_relisted!(""), [pass.code()])?;
    tx.execute("DELETE FROM file_failures WHERE pass = ?1", [pass.code()])?;
    tx.commit()?;
    Ok(())
}

/// The ledgers' entries the inbox shows: each pass with failures the user hasn't dismissed (or with new ones since).
pub fn entries(catalog: &Catalog) -> Result<Vec<Summary>> {
    let mut out = Vec::new();
    for pass in FilePass::ALL {
        if let Some(summary) = summary(&catalog.conn, pass)? {
            out.push(summary);
        }
    }
    Ok(out)
}

/// What the user saw of a pass's entry: the serial seen up to, and when they read and dismissed it.
struct Ack {
    seen_to: i64,
    read: bool,
    dismissed: bool,
}

fn ack(conn: &Connection, pass: FilePass) -> Result<Option<Ack>> {
    Ok(conn
        .prepare_cached("SELECT seen_to, read IS NOT NULL, dismissed IS NOT NULL FROM ledger_ack WHERE kind = ?1")?
        .query_row([pass.code()], |r| Ok(Ack { seen_to: r.get(0)?, read: r.get(1)?, dismissed: r.get(2)? }))
        .optional()?)
}

/// The import's failed files: the files the grid lists as failed (an analysis that failed and is current, of a file that
/// is there). From the failed analyses' index: the files are looked up by id (`CROSS JOIN`: the planner would otherwise
/// walk every present file).
macro_rules! import_failures {
    ($columns:literal, $more:literal) => {
        concat!(
            "SELECT ",
            $columns,
            " FROM analysis a CROSS JOIN files f ON f.id = a.file_id
             WHERE a.error IS NOT NULL AND f.pending = 0 AND f.missing = 0",
            $more
        )
    };
}

fn summary(conn: &Connection, pass: FilePass) -> Result<Option<Summary>> {
    let ack = ack(conn, pass)?;
    let seen_to = ack.as_ref().map_or(0, |a| a.seen_to);
    type Row = (i64, i64, Option<i64>, Option<i64>, Option<i64>);
    let row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Row> {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
    };
    let (total, new, newest, created, updated) = match pass {
        FilePass::Import => conn
            .prepare_cached(import_failures!(
                "count(*), count(*) FILTER (WHERE f.seq > ?1), max(f.seq), min(a.analyzed), max(a.analyzed)",
                ""
            ))?
            .query_row([seen_to], row)?,
        FilePass::Thumbs | FilePass::Borders => conn
            .prepare_cached(
                "SELECT count(*), count(*) FILTER (WHERE serial > ?1), max(serial), min(first_seen), max(first_seen)
                 FROM file_failures WHERE pass = ?2",
            )?
            .query_row(params![seen_to, pass.code()], row)?,
    };
    let (Some(newest), Some(created), Some(updated)) = (newest, created, updated) else { return Ok(None) };
    let (total, new) = (count(total), count(new));
    if ack.as_ref().is_some_and(|a| a.dismissed) && new == 0 {
        return Ok(None);
    }
    let read = ack.as_ref().is_some_and(|a| a.read) && new == 0;
    Ok(Some(Summary { pass, total, new, newest, created, updated, read }))
}

/// A failed file as [`files`] reads it: id, folder, name, error, retry, first seen, new.
type FailedRow = (i64, Vec<u8>, Vec<u8>, String, bool, i64, bool);

/// The failed files of `pass`'s entry, newest first, at most `limit`.
pub fn files(catalog: &Catalog, pass: FilePass, limit: usize) -> Result<Vec<FailedFile>> {
    let conn = &catalog.conn;
    let seen_to = ack(conn, pass)?.map_or(0, |a| a.seen_to);
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let mut stmt = match pass {
        FilePass::Import => conn.prepare_cached(import_failures!(
            "f.id, (SELECT path FROM folders WHERE id = f.folder_id), f.name, a.error, 1, a.analyzed, f.seq > ?1",
            " ORDER BY f.seq DESC LIMIT ?2"
        ))?,
        FilePass::Thumbs | FilePass::Borders => conn.prepare_cached(
            "SELECT f.id, (SELECT path FROM folders WHERE id = f.folder_id), f.name, l.error, l.retry, l.first_seen,
                    l.serial > ?1
             FROM file_failures l JOIN files f ON f.id = l.file_id WHERE l.pass = ?3
             ORDER BY l.serial DESC LIMIT ?2",
        )?,
    };
    let rules = catalog.rules();
    let map = |r: &rusqlite::Row<'_>| -> rusqlite::Result<FailedRow> {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
    };
    let rows: Vec<_> = if pass == FilePass::Import {
        stmt.query_map(params![seen_to, limit], map)?.collect::<rusqlite::Result<_>>()?
    } else {
        stmt.query_map(params![seen_to, limit, pass.code()], map)?.collect::<rusqlite::Result<_>>()?
    };
    let mut out = Vec::with_capacity(rows.len());
    for (id, folder, name, error, retry, first_seen, new) in rows {
        let mut path =
            rules.parse_bytes(&folder).ok_or_else(|| anyhow::anyhow!("a failed file's folder holds a NUL"))?;
        path.parts.push(pg_model::name::Name::from_raw(&name));
        out.push(FailedFile { file_id: FileId::new(id), path, error, retry, first_seen, new });
    }
    Ok(out)
}

/// The user read `pass`'s entry as it was listed up to `seen` (its [`Summary::newest`]): later failures stay new. A
/// mark older than the one kept moves nothing back.
pub fn mark_read(catalog: &Catalog, pass: FilePass, seen: i64, now: i64) -> Result<()> {
    catalog.conn.execute(
        "INSERT INTO ledger_ack(kind, seen_to, read) VALUES (?1, ?2, ?3)
         ON CONFLICT(kind) DO UPDATE SET seen_to = max(seen_to, excluded.seen_to), read = excluded.read",
        params![pass.code(), seen, now],
    )?;
    Ok(())
}

/// The user dismissed `pass`'s entry as it was listed up to `seen`: it comes back with the next new failure.
pub fn dismiss(catalog: &Catalog, pass: FilePass, seen: i64, now: i64) -> Result<()> {
    catalog.conn.execute(
        "INSERT INTO ledger_ack(kind, seen_to, read, dismissed) VALUES (?1, ?2, ?3, ?3)
         ON CONFLICT(kind) DO UPDATE SET seen_to = max(seen_to, excluded.seen_to), read = coalesce(read, excluded.read),
             dismissed = excluded.dismissed",
        params![pass.code(), seen, now],
    )?;
    Ok(())
}

/// `error` as kept: on one line (a control character is a space), at most [`ERROR_BYTES`].
fn tidy(error: &str) -> String {
    let mut out: String = error.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if out.len() > ERROR_BYTES {
        let cut = out.char_indices().map(|(i, _)| i).take_while(|&i| i <= ERROR_BYTES - 3).last().unwrap_or(0);
        out.truncate(cut);
        out.push('…');
    }
    out
}

/// A stored count (never negative).
fn count(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::under_both_rules;
    use crate::fixture::{self, File};
    use crate::test_support::versions;
    use pg_model::ids::FolderId;
    use pg_model::rules::PathRules;

    fn catalog(rules: PathRules) -> Catalog {
        let c = Catalog::prepare(std::path::Path::new(":memory:"), versions(), rules).unwrap();
        let folder = if rules == PathRules::Windows { r"D:\p" } else { "/p" };
        fixture::state::add_scan(&c, 1, folder).unwrap();
        fixture::state::add_folder(&c, 1, folder).unwrap();
        let files: Vec<File> = (1..=4)
            .zip(["1.jpg", "2.jpg", "3.jpg", "4.jpg"])
            .map(|(i, name)| File::new(FileId::new(i), FolderId::new(1), name, 10, 100))
            .collect();
        fixture::insert(&c, &files).unwrap();
        c
    }

    /// The failure of file `id` as the fixture made it (size 10, mtime 100).
    fn failure(id: i64, error: &str) -> Failure {
        Failure {
            file_id: FileId::new(id),
            mtime: 100,
            size: 10,
            rotation: Turns::NONE,
            error: error.into(),
            retry: false,
        }
    }

    fn serials(c: &Catalog) -> Vec<(i64, i64)> {
        c.conn
            .prepare("SELECT file_id, serial FROM file_failures ORDER BY file_id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    // A failure is recorded once per file, for the file as it was read: one recorded again keeps its serial, one of a
    // file changed since is not recorded, and a change to the file removes its row.
    fn a_failure_is_one_row_for_the_file_as_it_failed(rules: PathRules) {
        let c = catalog(rules);
        record(&c, FilePass::Thumbs, &[failure(1, "a"), failure(2, "b")], 10).unwrap();
        let first = serials(&c);
        assert_eq!(first.len(), 2);
        let version = c.data_version().unwrap();
        let changes = c.conn.total_changes();
        record(&c, FilePass::Thumbs, &[failure(1, "a"), failure(2, "b")], 20).unwrap();
        assert_eq!((serials(&c), c.conn.total_changes()), (first.clone(), changes), "an unchanged failure writes");
        assert_eq!(c.data_version().unwrap(), version);
        // A new error is kept, the serial too.
        record(&c, FilePass::Thumbs, &[failure(1, "a again")], 30).unwrap();
        assert_eq!(serials(&c), first);
        // Read at another state: not recorded.
        record(&c, FilePass::Thumbs, &[Failure { mtime: 99, ..failure(3, "c") }], 30).unwrap();
        assert_eq!(serials(&c).len(), 2);
        // The file changes: its row goes; the other pass's and file's stay.
        record(&c, FilePass::Borders, &[failure(2, "b")], 30).unwrap();
        crate::fixture::raw::set_size(&c, FileId::new(2), 11);
        assert_eq!(serials(&c).iter().map(|s| s.0).collect::<Vec<_>>(), [1]);
        assert_eq!(recorded(&c, FilePass::Thumbs).unwrap(), BTreeSet::from([FileId::new(1)]));
        clear(&c, FilePass::Thumbs, &[FileId::new(1), FileId::new(3)]).unwrap();
        assert!(serials(&c).is_empty());
        // A missing file is not recorded, and a deleted one takes its rows.
        crate::fixture::raw::set_missing(&c, FileId::new(4));
        record(&c, FilePass::Thumbs, &[failure(4, "d")], 40).unwrap();
        record(&c, FilePass::Thumbs, &[failure(3, "c")], 40).unwrap();
        c.conn.execute("DELETE FROM files WHERE id = 3", []).unwrap();
        assert!(serials(&c).is_empty());
    }

    under_both_rules!(
        a_failure_is_one_row_for_the_file_as_it_failed,
        a_failure_is_one_row_for_the_file_as_it_failed_windows,
        a_failure_is_one_row_for_the_file_as_it_failed_posix
    );

    // A pass's ledger is one entry: unread until read up to its newest failure, gone once dismissed, and back, unread,
    // with the next new failure, which it counts apart from the earlier ones.
    fn a_ledger_is_one_entry_new_again_with_a_new_failure(rules: PathRules) {
        let c = catalog(rules);
        assert!(entries(&c).unwrap().is_empty());
        record(&c, FilePass::Thumbs, &[failure(1, "a"), failure(2, "b")], 10).unwrap();
        let [entry] = entries(&c).unwrap()[..] else { panic!("one entry") };
        assert_eq!((entry.total, entry.new, entry.read), (2, 2, false));
        assert_eq!(entry.notice(), Notice::ThumbsFailed { counts: None });
        // Read as listed: a later failure is new.
        mark_read(&c, FilePass::Thumbs, entry.newest, 11).unwrap();
        assert!(entries(&c).unwrap()[0].read);
        dismiss(&c, FilePass::Thumbs, entry.newest, 12).unwrap();
        assert!(entries(&c).unwrap().is_empty());
        // The same failures again: still dismissed.
        record(&c, FilePass::Thumbs, &[failure(1, "a"), failure(2, "b")], 13).unwrap();
        assert!(entries(&c).unwrap().is_empty());
        record(&c, FilePass::Thumbs, &[failure(3, "c")], 14).unwrap();
        let [again] = entries(&c).unwrap()[..] else { panic!("one entry") };
        assert_eq!((again.total, again.new, again.read), (3, 1, false));
        assert_eq!(again.notice(), Notice::ThumbsFailed { counts: Some(LedgerCounts { new: 1, earlier: 2 }) });
        // A stale mark moves nothing back.
        mark_read(&c, FilePass::Thumbs, entry.newest, 15).unwrap();
        assert_eq!(entries(&c).unwrap()[0].new, 1);
        let listed = files(&c, FilePass::Thumbs, 10).unwrap();
        assert_eq!(
            listed.iter().map(|f| (f.file_id.get(), f.new)).collect::<Vec<_>>(),
            [(3, true), (2, false), (1, false)]
        );
        assert_eq!(listed[0].path.parts.last().unwrap().display(), "3.jpg");
        // A retry lists the files again for the pass and empties the ledger.
        let seq =
            |id: i64| -> i64 { c.conn.query_row("SELECT seq FROM files WHERE id = ?1", [id], |r| r.get(0)).unwrap() };
        let before = seq(1);
        retry(&c, FilePass::Thumbs).unwrap();
        assert!(seq(1) > before && entries(&c).unwrap().is_empty());
    }

    under_both_rules!(
        a_ledger_is_one_entry_new_again_with_a_new_failure,
        a_ledger_is_one_entry_new_again_with_a_new_failure_windows,
        a_ledger_is_one_entry_new_again_with_a_new_failure_posix
    );

    // The import's entry is its failed analyses: those of present files that are current.
    #[test]
    fn the_imports_entry_is_its_failed_analyses() {
        let c = catalog(PathRules::Posix);
        let failed = crate::fixture::raw::AnalysisRow {
            error: Some("it doesn't decode"),
            ..crate::fixture::raw::AnalysisRow::image(1, 10, 100)
        };
        crate::fixture::raw::insert_analysis(&c, &failed);
        let [entry] = entries(&c).unwrap()[..] else { panic!("one entry") };
        assert_eq!((entry.pass, entry.total), (FilePass::Import, 1));
        assert_eq!(files(&c, FilePass::Import, 10).unwrap()[0].error, "it doesn't decode");
        dismiss(&c, FilePass::Import, entry.newest, 1).unwrap();
        assert!(entries(&c).unwrap().is_empty());
        crate::fixture::raw::set_missing(&c, FileId::new(1));
        assert!(files(&c, FilePass::Import, 10).unwrap().is_empty());
    }

    // A new thumbnailer tries the panicked files again (their seq moves on) and starts the ledger anew; a new borders
    // detector starts its ledger anew.
    #[test]
    fn a_new_version_starts_its_ledger_anew() {
        let c = catalog(PathRules::Posix);
        record(&c, FilePass::Thumbs, &[failure(1, "panicked"), Failure { retry: true, ..failure(2, "b") }], 1).unwrap();
        record(&c, FilePass::Borders, &[failure(3, "c")], 1).unwrap();
        let seq =
            |id: i64| -> i64 { c.conn.query_row("SELECT seq FROM files WHERE id = ?1", [id], |r| r.get(0)).unwrap() };
        let (one, two) = (seq(1), seq(2));
        crate::lifecycle::set_thumbs_version(&c.conn, versions().thumbs).unwrap();
        assert_eq!(serials(&c).len(), 3, "the same version changes nothing");
        crate::lifecycle::set_thumbs_version(&c.conn, versions().thumbs + 1).unwrap();
        assert!(seq(1) > one && seq(2) == two, "only the panicked file is listed again");
        assert!(recorded(&c, FilePass::Thumbs).unwrap().is_empty());
        crate::lifecycle::set_borders_version(&c.conn, 0).unwrap();
        assert!(recorded(&c, FilePass::Borders).unwrap().is_empty());
    }

    #[test]
    fn an_error_is_kept_on_one_line_and_bounded() {
        assert_eq!(tidy("a\nb\tc"), "a b c");
        let long = "é".repeat(ERROR_BYTES);
        let kept = tidy(&long);
        assert!(kept.len() <= ERROR_BYTES && kept.ends_with('…'), "{}", kept.len());
    }
}
