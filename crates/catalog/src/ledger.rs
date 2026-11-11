//! The failure ledgers (docs/NOTIFICATIONS.md). The files a pass couldn't make its result of, for the state each file was
//! in (`file_failures`): the thumbnails' and the borders' passes write theirs; the import's rows are kept by triggers
//! on `analysis` (`TO_8_NOTIFY`), mirroring the failed analyses. And the folders the walks couldn't take
//! (`folder_failures`), which the scan's writer records and a complete walk sweeps (`Catalog::mark_missing`).
//!
//! Each ledger is one inbox entry ([`LedgerKind`]), built when the inbox is read ([`Catalog::ledger_entries`]) from its
//! cheap state ([`Catalog::ledger_state`]: its newest and oldest rows, each one index seek), and its counts only while
//! the inbox is viewed. What the user saw of it is its acknowledgement (`ledger_ack`, by serial: [`LedgerAck`]): it is
//! shown while it has live rows and wasn't dismissed or has one since, and unread while never read or with a row since
//! the user last saw it. Uses `core` (the connection), `writers` (the ledger's writes) and the leaves `failure` and
//! `state`.
//!
//! A file's row describes the file as it failed: a change to the file (its size, time or turn) removes the thumbnails'
//! and the borders' rows (`files_failures_stale`), and so does a pass that makes the file's result after all
//! ([`Catalog::clear_failures`]); a file gone missing keeps them, hidden (`hidden`), so they come back with their serials.
//! A failure that holds writes nothing when it is recorded again ([`Catalog::record_failures`]): it keeps its serial, so
//! it is not news again.

use std::collections::BTreeSet;

use anyhow::anyhow;
use rusqlite::{Connection, OptionalExtension, params};

use pg_model::ids::{FileId, RootId};
use pg_model::notice::{FilePass, LedgerCounts, LedgerKind, Notice};
use pg_model::rules::Path as RulePath;

use crate::core::Catalog;
use crate::error::Result;
use crate::transaction::write_transaction;
use crate::types::FolderFailure;
use crate::writers::{clear_failures_on, failures_held, record_failures_on};

pub(crate) mod failure;
mod state;
pub use failure::{ERROR_BYTES, Failure};
pub use state::{Counts, FailedFolder, LedgerAck, LedgerRow, LedgerState};

/// A ledger as the inbox shows it: one entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    pub kind: LedgerKind,
    /// The newest row's serial: what the user sees of the entry when it is listed ([`Catalog::mark_ledger_read`],
    /// [`Catalog::dismiss_ledger`]).
    pub newest: i64,
    /// When the oldest and the newest rows were first recorded (Unix seconds).
    pub created: i64,
    pub updated: i64,
    pub read: bool,
    /// Read only while the inbox is viewed (`viewing`): `None` otherwise, and nothing renders a count then.
    pub counts: Option<Counts>,
}

impl Summary {
    /// The entry's notice: with its counts once some rows are new and some were seen (and the counts were read).
    pub fn notice(&self) -> Notice {
        let counts = self
            .counts
            .filter(|c| c.new > 0 && c.total > c.new)
            .map(|c| LedgerCounts { new: c.new, earlier: c.total - c.new });
        self.kind.notice(counts)
    }
}

/// A failed file, as the inbox's expanded entry lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedFile {
    pub file_id: FileId,
    /// The file's path, exact.
    pub path: RulePath,
    pub error: String,
    /// The pass tries it again by itself; else not until it changes, or the user retries it (the import's: by
    /// `gallery import --retry`).
    pub retry: bool,
    /// When it was first recorded (Unix seconds).
    pub first_seen: i64,
    /// Whether it came after what the user last saw of the entry.
    pub new: bool,
}

impl Catalog {
    /// Record `failures` of `pass` (the thumbnails' or the borders'), found at `now`, in a write transaction of its own
    /// (a pass that stores its results in a transaction records them there instead: `Tx::ledger_writer`). Whether a row
    /// changed. The import's ledger is its analyses' (an error).
    pub fn record_failures(&self, pass: FilePass, failures: &[Failure], now: i64) -> Result<bool> {
        if failures.is_empty() {
            return Ok(false);
        }
        let tx = write_transaction(&self.conn)?;
        let changed = record_failures_on(&tx, pass, failures, now)?;
        tx.commit()?;
        Ok(changed)
    }

    /// The files `pass`'s ledger holds: a pass reads them when it starts, to [`Catalog::clear_failures`] only those it
    /// then makes.
    pub fn recorded_failures(&self, pass: FilePass) -> Result<BTreeSet<FileId>> {
        let mut stmt = self.conn.prepare_cached("SELECT file_id FROM file_failures WHERE pass = ?1")?;
        let ids = stmt.query_map([pass.code()], |r| r.get::<_, i64>(0))?;
        Ok(ids.map(|id| id.map(FileId::new)).collect::<rusqlite::Result<_>>()?)
    }

    /// Whether `pass`'s ledger holds any file (dismissed, hidden or not).
    pub fn any_failures(&self, pass: FilePass) -> Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT EXISTS(SELECT 1 FROM file_failures WHERE pass = ?1)", [pass.code()], |r| r.get(0))?)
    }

    /// The results of `files` were made by `pass` (the thumbnails' or the borders'): their failures are gone. Nothing is
    /// written when the ledger holds none of them (it is looked at first, so a clear of what isn't there doesn't take
    /// the write lock). Whether a row went.
    pub fn clear_failures(&self, pass: FilePass, files: &[FileId]) -> Result<bool> {
        if !failures_held(&self.conn, pass, files)? {
            return Ok(false);
        }
        let tx = write_transaction(&self.conn)?;
        let gone = clear_failures_on(&tx, pass, files)?;
        tx.commit()?;
        Ok(gone)
    }

    /// Try every failed file of `pass` again: the pass's next run lists them (their `seq` moves on), and the rows go, so
    /// what still fails is recorded anew. The user's Retry, for the thumbnails and the borders. The import's rows mirror
    /// its analyses, so they are never deleted here: its failures are retried by `gallery import --retry`, and this
    /// does nothing for it.
    pub fn retry_failures(&self, pass: FilePass) -> Result<()> {
        if pass == FilePass::Import {
            return Ok(());
        }
        let tx = write_transaction(&self.conn)?;
        tx.execute_batch(next_seq!())?;
        tx.execute(failures_relisted!(""), [pass.code()])?;
        tx.execute("DELETE FROM file_failures WHERE pass = ?1", [pass.code()])?;
        tx.commit()?;
        Ok(())
    }

    /// What the user saw of `kind`'s entry (the default, nothing seen, when never acknowledged).
    pub fn ledger_ack(&self, kind: LedgerKind) -> Result<LedgerAck> {
        ack_on(&self.conn, kind)
    }

    /// `kind`'s cheap state as `ack` acknowledges it: from its newest and oldest live rows, each one seek of the
    /// ledger's serial index (`file_failures_live`, `folder_failures_serial`), whatever the ledger holds.
    pub fn ledger_state(&self, kind: LedgerKind, ack: &LedgerAck) -> Result<LedgerState> {
        state_on(&self.conn, kind, ack)
    }

    /// `kind`'s counts as `ack` acknowledges it: a count over its live rows, for the inbox while it is viewed.
    pub fn ledger_counts(&self, kind: LedgerKind, ack: &LedgerAck) -> Result<Counts> {
        counts_on(&self.conn, kind, ack)
    }

    /// The ledgers' entries the inbox shows ([`LedgerState::shown`]), in one snapshot; with their counts when
    /// `viewing`.
    pub fn ledger_entries(&self, viewing: bool) -> Result<Vec<Summary>> {
        let snapshot = self.conn.unchecked_transaction()?;
        let entries = ledger_entries_on(&snapshot, viewing)?;
        snapshot.commit()?;
        Ok(entries)
    }

    /// The failed files of `pass`'s entry, newest first, at most `limit`: the live rows (of files present, and for the
    /// import not pending, as the grid's failed filter has them).
    pub fn failed_files(&self, pass: FilePass, limit: usize) -> Result<Vec<FailedFile>> {
        let conn = &self.conn;
        let seen_to = ack_on(conn, LedgerKind::File(pass))?.seen_to;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut stmt = conn.prepare_cached(concat!(
            "SELECT f.id, (SELECT path FROM folders WHERE id = f.folder_id), f.name, l.error, l.retry, l.first_seen,
                    l.serial > ?1
             FROM file_failures l JOIN files f ON f.id = l.file_id WHERE l.pass = ?3 AND l.",
            live_failures!(),
            " ORDER BY l.serial DESC LIMIT ?2"
        ))?;
        let rows = stmt
            .query_map(params![seen_to, limit, pass.code()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
            })?
            .collect::<rusqlite::Result<Vec<FailedRow>>>()?;
        let rules = self.rules();
        let mut out = Vec::with_capacity(rows.len());
        for (id, folder, name, error, retry, first_seen, new) in rows {
            let mut path = rules.parse_bytes(&folder).ok_or_else(|| anyhow!("a failed file's folder holds a NUL"))?;
            path.parts.push(pg_model::name::Name::from_raw(&name));
            out.push(FailedFile { file_id: FileId::new(id), path, error, retry, first_seen, new });
        }
        Ok(out)
    }

    /// The folders of the scans' ledger, grouped by the library folder that holds them (by its path, then theirs), at
    /// most `limit`: its expanded entry's detail.
    pub fn failed_folders(&self, limit: usize) -> Result<Vec<FailedFolder>> {
        let conn = &self.conn;
        let seen_to = ack_on(conn, LedgerKind::ScanFolders)?.seen_to;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows: Vec<FolderRow> = conn
            .prepare_cached(
                "SELECT l.root, r.path, l.path, l.reason, l.error, l.first_seen, l.serial > ?1
                 FROM folder_failures l JOIN roots r ON r.id = l.root ORDER BY r.path, l.path LIMIT ?2",
            )?
            .query_map(params![seen_to, limit], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let rules = self.rules();
        let parse = |bytes: &[u8]| -> Result<RulePath> {
            Ok(rules.parse_bytes(bytes).ok_or_else(|| anyhow!("a failed folder's path holds a NUL"))?)
        };
        let mut out = Vec::with_capacity(rows.len());
        for (root, root_path, path, reason, error, first_seen, new) in rows {
            let failure =
                FolderFailure::from_code(&reason).ok_or_else(|| anyhow!("a failed folder's reason {reason:?}"))?;
            out.push(FailedFolder {
                root: RootId::new(root),
                root_path: parse(&root_path)?,
                path: parse(&path)?,
                failure,
                error,
                first_seen,
                new,
            });
        }
        Ok(out)
    }

    /// The user read `kind`'s entry as it was listed up to `seen` (its [`Summary::newest`]): later rows stay new. A
    /// mark older than the one kept moves nothing back. Whether the acknowledgement changed.
    pub fn mark_ledger_read(&self, kind: LedgerKind, seen: i64, now: i64) -> Result<bool> {
        mark_ledger_read_on(&self.conn, kind, seen, now)
    }

    /// The user dismissed `kind`'s entry as it was listed up to `seen`: it comes back with the next row after it, and
    /// says how many came since. Whether the acknowledgement changed.
    pub fn dismiss_ledger(&self, kind: LedgerKind, seen: i64, now: i64) -> Result<bool> {
        dismiss_ledger_on(&self.conn, kind, seen, now)
    }
}

/// A failed file as [`Catalog::failed_files`] reads it: id, folder, name, error, retry, first seen, new.
type FailedRow = (i64, Vec<u8>, Vec<u8>, String, bool, i64, bool);

/// A failed folder as [`Catalog::failed_folders`] reads it: root, root's path, path, reason, error, first seen, new.
type FolderRow = (i64, Vec<u8>, Vec<u8>, String, String, i64, bool);

/// [`Catalog::ledger_ack`] on `conn`.
pub(crate) fn ack_on(conn: &Connection, kind: LedgerKind) -> Result<LedgerAck> {
    Ok(conn
        .prepare_cached("SELECT seen_to, dismissed_to, read IS NOT NULL FROM ledger_ack WHERE kind = ?1")?
        .query_row([kind.code()], |r| Ok(LedgerAck { seen_to: r.get(0)?, dismissed_to: r.get(1)?, read: r.get(2)? }))
        .optional()?
        .unwrap_or_default())
}

/// The newest (`DESC`) and oldest (`ASC`) live rows of a file pass's ledger: one seek each on `file_failures_live`.
macro_rules! file_ledger_end {
    ($order:literal) => {
        concat!(
            "SELECT serial, first_seen FROM file_failures WHERE pass = ?1 AND ",
            live_failures!(),
            " ORDER BY serial ",
            $order,
            " LIMIT 1"
        )
    };
}

/// The newest and oldest rows of the scans' ledger: one seek each on `folder_failures_serial`.
macro_rules! folder_ledger_end {
    ($order:literal) => {
        concat!("SELECT serial, first_seen FROM folder_failures ORDER BY serial ", $order, " LIMIT 1")
    };
}

pub(crate) const FILE_LEDGER_NEWEST: &str = file_ledger_end!("DESC");
const FILE_LEDGER_OLDEST: &str = file_ledger_end!("ASC");
const FOLDER_LEDGER_NEWEST: &str = folder_ledger_end!("DESC");
const FOLDER_LEDGER_OLDEST: &str = folder_ledger_end!("ASC");

/// A file pass's counts over its live rows (`?1` the pass, `?2` seen to, `?3` dismissed to, or NULL).
pub(crate) const FILE_LEDGER_COUNTS: &str = concat!(
    "SELECT count(*), count(*) FILTER (WHERE serial > ?2), count(*) FILTER (WHERE ?3 IS NOT NULL AND serial > ?3)
     FROM file_failures WHERE pass = ?1 AND ",
    live_failures!()
);

/// The scans' ledger's counts (`?2` seen to, `?3` dismissed to, or NULL).
const FOLDER_LEDGER_COUNTS: &str =
    "SELECT count(*), count(*) FILTER (WHERE serial > ?2), count(*) FILTER (WHERE ?3 IS NOT NULL AND serial > ?3)
     FROM folder_failures WHERE ?1 IS NOT NULL";

/// The end row `sql` reads of `kind`'s ledger (`?1` its pass, for a file pass's).
fn ledger_end(conn: &Connection, kind: LedgerKind, sql: &str) -> Result<Option<LedgerRow>> {
    let row = |r: &rusqlite::Row<'_>| Ok(LedgerRow { serial: r.get(0)?, first_seen: r.get(1)? });
    let mut stmt = conn.prepare_cached(sql)?;
    Ok(match kind {
        LedgerKind::File(pass) => stmt.query_row([pass.code()], row),
        LedgerKind::ScanFolders => stmt.query_row([], row),
    }
    .optional()?)
}

/// [`Catalog::ledger_state`] on `conn`.
pub(crate) fn state_on(conn: &Connection, kind: LedgerKind, ack: &LedgerAck) -> Result<LedgerState> {
    let (newest, oldest) = match kind {
        LedgerKind::File(_) => (FILE_LEDGER_NEWEST, FILE_LEDGER_OLDEST),
        LedgerKind::ScanFolders => (FOLDER_LEDGER_NEWEST, FOLDER_LEDGER_OLDEST),
    };
    let newest = ledger_end(conn, kind, newest)?;
    // The oldest only matters for an entry that has rows.
    let oldest = if newest.is_some() { ledger_end(conn, kind, oldest)? } else { None };
    Ok(LedgerState::of(newest, oldest, ack))
}

/// [`Catalog::ledger_counts`] on `conn`.
pub(crate) fn counts_on(conn: &Connection, kind: LedgerKind, ack: &LedgerAck) -> Result<Counts> {
    let (sql, code) = match kind {
        LedgerKind::File(pass) => (FILE_LEDGER_COUNTS, pass.code()),
        LedgerKind::ScanFolders => (FOLDER_LEDGER_COUNTS, kind.code()),
    };
    let (total, new, since_dismissed): (i64, i64, i64) = conn
        .prepare_cached(sql)?
        .query_row(params![code, ack.seen_to, ack.dismissed_to], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    Ok(Counts { total: count(total), new: count(new), since_dismissed: count(since_dismissed) })
}

/// [`Catalog::ledger_entries`] on `conn`, which the caller holds a snapshot on.
pub(crate) fn ledger_entries_on(conn: &Connection, viewing: bool) -> Result<Vec<Summary>> {
    let mut out = Vec::new();
    for kind in LedgerKind::ALL {
        let ack = ack_on(conn, kind)?;
        let state = state_on(conn, kind, &ack)?;
        let (true, Some(newest), Some(oldest)) = (state.shown, state.newest, state.oldest) else { continue };
        let counts = if viewing { Some(counts_on(conn, kind, &ack)?) } else { None };
        out.push(Summary {
            kind,
            newest: newest.serial,
            created: oldest.first_seen,
            updated: newest.first_seen,
            read: !state.unread,
            counts,
        });
    }
    Ok(out)
}

/// [`Catalog::mark_ledger_read`] on `conn`: `seen_to` only moves on, and `read` is the first time it was read.
pub(crate) fn mark_ledger_read_on(conn: &Connection, kind: LedgerKind, seen: i64, now: i64) -> Result<bool> {
    let changed = conn
        .prepare_cached(
            "INSERT INTO ledger_ack(kind, seen_to, read) VALUES (?1, ?2, ?3)
             ON CONFLICT(kind) DO UPDATE SET seen_to = max(seen_to, excluded.seen_to), read = coalesce(read, excluded.read)
             WHERE excluded.seen_to > seen_to OR read IS NULL",
        )?
        .execute(params![kind.code(), seen, now])?;
    Ok(changed > 0)
}

/// [`Catalog::dismiss_ledger`] on `conn`: `dismissed_to` and `seen_to` only move on.
pub(crate) fn dismiss_ledger_on(conn: &Connection, kind: LedgerKind, seen: i64, now: i64) -> Result<bool> {
    let changed = conn
        .prepare_cached(
            "INSERT INTO ledger_ack(kind, seen_to, read, dismissed_to) VALUES (?1, ?2, ?3, ?2)
             ON CONFLICT(kind) DO UPDATE SET seen_to = max(seen_to, excluded.seen_to),
                 read = coalesce(read, excluded.read),
                 dismissed_to = max(coalesce(dismissed_to, excluded.dismissed_to), excluded.dismissed_to)
             WHERE excluded.seen_to > seen_to OR read IS NULL OR dismissed_to IS NULL
                 OR excluded.dismissed_to > dismissed_to",
        )?
        .execute(params![kind.code(), seen, now])?;
    Ok(changed > 0)
}

/// A stored count (never negative).
fn count(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::under_both_rules;
    use crate::fixture::{self, File, raw};
    use crate::queue;
    use crate::test_support::versions;
    use pg_model::ids::FolderId;
    use pg_model::rules::PathRules;
    use pg_model::turns::Turns;

    const THUMBS: LedgerKind = LedgerKind::File(FilePass::Thumbs);
    const IMPORT: LedgerKind = LedgerKind::File(FilePass::Import);

    /// A catalog at `path` with files 1 to `n` (size 10, mtime 100) in one folder, none analyzed.
    fn catalog_at(path: &std::path::Path, rules: PathRules, n: i64) -> Catalog {
        let c = Catalog::prepare(path, versions(), rules).unwrap();
        let folder = if rules == PathRules::Windows { r"D:\p" } else { "/p" };
        fixture::state::add_scan(&c, 1, folder).unwrap();
        fixture::state::add_folder(&c, 1, folder).unwrap();
        let files: Vec<File> =
            (1..=n).map(|i| File::new(FileId::new(i), FolderId::new(1), format!("{i}.jpg").leak(), 10, 100)).collect();
        fixture::insert(&c, &files).unwrap();
        c
    }

    fn catalog_of(rules: PathRules, n: i64) -> Catalog {
        catalog_at(std::path::Path::new(":memory:"), rules, n)
    }

    fn catalog(rules: PathRules) -> Catalog {
        catalog_of(rules, 4)
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

    /// The rows of `pass`'s ledger: file and serial.
    fn serials(c: &Catalog, pass: &str) -> Vec<(i64, i64)> {
        raw::rows_of(c, "SELECT file_id, serial FROM file_failures WHERE pass = ?1 ORDER BY file_id", [pass], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
    }

    /// The import storing an analysis of file `id` made from the bytes it read (`size`, `mtime`), failed with `error`
    /// or not: `INSERT OR REPLACE` at this build's import version, as the import writes it.
    fn analysis(c: &Catalog, id: i64, size: i64, mtime: i64, error: Option<&str>) {
        c.conn
            .execute(
                "INSERT OR REPLACE INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, error)
                 VALUES (?1, ?2, ?3, ?4, 0, 'image', 5, ?5)",
                params![id, versions().import, size, mtime, error],
            )
            .unwrap();
    }

    fn state(c: &Catalog, kind: LedgerKind) -> LedgerState {
        c.ledger_state(kind, &c.ledger_ack(kind).unwrap()).unwrap()
    }

    fn counts(c: &Catalog, kind: LedgerKind) -> Counts {
        c.ledger_counts(kind, &c.ledger_ack(kind).unwrap()).unwrap()
    }

    fn newest(c: &Catalog, kind: LedgerKind) -> i64 {
        state(c, kind).newest.unwrap().serial
    }

    // A failure is recorded once per file, for the file as it was read: one recorded again keeps its serial and writes
    // nothing (another connection sees no change), one of a file changed since is not recorded, and a change to the
    // file removes its row; a file gone missing keeps it, hidden, and a deleted one takes it.
    #[test]
    fn a_failure_is_one_row_for_the_file_as_it_failed() {
        let dir = pg_testkit::test_folder("scratch_catalog", "ledger-one-row").unwrap();
        let path = dir.join("catalog.sqlite");
        let rules = pg_platform::native::PATH_RULES;
        let c = catalog_at(&path, rules, 4);
        let other = Catalog::open(&path, rules).unwrap();

        assert!(c.record_failures(FilePass::Thumbs, &[failure(1, "a"), failure(2, "b")], 10).unwrap());
        let first = serials(&c, "thumbs");
        assert_eq!(first.len(), 2);
        let version = other.data_version().unwrap();
        assert!(!c.record_failures(FilePass::Thumbs, &[failure(1, "a"), failure(2, "b")], 20).unwrap());
        assert_eq!(serials(&c, "thumbs"), first);
        assert_eq!(other.data_version().unwrap(), version, "an unchanged failure writes nothing");
        // A new error is kept, the serial too.
        assert!(c.record_failures(FilePass::Thumbs, &[failure(1, "a again")], 30).unwrap());
        assert_eq!(serials(&c, "thumbs"), first);
        assert_ne!(other.data_version().unwrap(), version);
        // Read at another state: not recorded.
        assert!(!c.record_failures(FilePass::Thumbs, &[Failure { mtime: 99, ..failure(3, "c") }], 30).unwrap());
        // The file changes: its rows go; the other file's stay.
        c.record_failures(FilePass::Borders, &[failure(2, "b")], 30).unwrap();
        raw::set_size(&c, FileId::new(2), 11);
        assert_eq!(serials(&c, "thumbs").iter().map(|s| s.0).collect::<Vec<_>>(), [1]);
        assert!(serials(&c, "borders").is_empty());
        assert_eq!(c.recorded_failures(FilePass::Thumbs).unwrap(), BTreeSet::from([FileId::new(1)]));
        assert!(c.clear_failures(FilePass::Thumbs, &[FileId::new(1), FileId::new(3)]).unwrap());
        assert!(!c.clear_failures(FilePass::Thumbs, &[FileId::new(1)]).unwrap(), "nothing held");
        // A missing file is not recorded; one gone missing after keeps its row, hidden; a deleted one takes its rows.
        raw::set_missing(&c, FileId::new(4));
        assert!(!c.record_failures(FilePass::Thumbs, &[failure(4, "d")], 40).unwrap());
        c.record_failures(FilePass::Thumbs, &[failure(3, "c")], 40).unwrap();
        raw::set_missing(&c, FileId::new(3));
        assert_eq!(serials(&c, "thumbs").len(), 1);
        assert!(!state(&c, THUMBS).live);
        c.conn.execute("DELETE FROM files WHERE id = 3", []).unwrap();
        assert!(serials(&c, "thumbs").is_empty());
        // The import's ledger is its analyses'.
        assert!(c.record_failures(FilePass::Import, &[failure(1, "x")], 50).is_err());
        assert!(c.clear_failures(FilePass::Import, &[FileId::new(1)]).is_ok(), "nothing held: no write to refuse");
        drop((c, other));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // A pass's ledger is one entry: unread until read up to its newest failure, gone once dismissed, and back, unread,
    // with the next new failure, which it counts apart from the earlier ones (while the inbox is viewed).
    fn a_ledger_is_one_entry_new_again_with_a_new_failure(rules: PathRules) {
        let c = catalog(rules);
        assert!(c.ledger_entries(true).unwrap().is_empty());
        c.record_failures(FilePass::Thumbs, &[failure(1, "a"), failure(2, "b")], 10).unwrap();
        let [entry] = c.ledger_entries(true).unwrap()[..] else { panic!("one entry") };
        let two_new = Counts { total: 2, new: 2, since_dismissed: 0 };
        assert_eq!((entry.kind, entry.counts, entry.read), (THUMBS, Some(two_new), false));
        assert_eq!(entry.notice(), Notice::ThumbsFailed { counts: None });
        assert_eq!(c.ledger_entries(false).unwrap()[0].counts, None, "no count unless viewed");
        // Read as listed: a later failure is new.
        assert!(c.mark_ledger_read(THUMBS, entry.newest, 11).unwrap());
        assert!(!c.mark_ledger_read(THUMBS, entry.newest, 11).unwrap(), "read already");
        assert!(c.ledger_entries(false).unwrap()[0].read);
        assert!(c.dismiss_ledger(THUMBS, entry.newest, 12).unwrap());
        assert!(c.ledger_entries(false).unwrap().is_empty());
        // The same failures again: still dismissed.
        c.record_failures(FilePass::Thumbs, &[failure(1, "a"), failure(2, "b")], 13).unwrap();
        assert!(c.ledger_entries(false).unwrap().is_empty());
        c.record_failures(FilePass::Thumbs, &[failure(3, "c")], 14).unwrap();
        let [again] = c.ledger_entries(true).unwrap()[..] else { panic!("one entry") };
        assert_eq!((again.counts, again.read), (Some(Counts { total: 3, new: 1, since_dismissed: 1 }), false));
        assert_eq!(again.notice(), Notice::ThumbsFailed { counts: Some(LedgerCounts { new: 1, earlier: 2 }) });
        // A stale mark moves nothing back.
        assert!(!c.mark_ledger_read(THUMBS, entry.newest, 15).unwrap());
        assert_eq!(counts(&c, THUMBS).new, 1);
        let listed = c.failed_files(FilePass::Thumbs, 10).unwrap();
        assert_eq!(
            listed.iter().map(|f| (f.file_id.get(), f.new)).collect::<Vec<_>>(),
            [(3, true), (2, false), (1, false)]
        );
        assert_eq!(listed[0].path.parts.last().unwrap().display(), "3.jpg");
        // A retry lists the files again for the pass and empties the ledger.
        let before = raw::file_seq(&c, FileId::new(1));
        c.retry_failures(FilePass::Thumbs).unwrap();
        assert!(raw::file_seq(&c, FileId::new(1)) > before && c.ledger_entries(false).unwrap().is_empty());
    }

    under_both_rules!(
        a_ledger_is_one_entry_new_again_with_a_new_failure,
        a_ledger_is_one_entry_new_again_with_a_new_failure_windows,
        a_ledger_is_one_entry_new_again_with_a_new_failure_posix
    );

    // A dismissal is the serial it was made at: reading the entry once it is back never hides it again (it used to
    // vanish on open), and it says how many came since.
    #[test]
    fn a_dismissed_entry_that_comes_back_stays_when_read() {
        let c = catalog(PathRules::Posix);
        c.record_failures(FilePass::Thumbs, &[failure(1, "a")], 10).unwrap();
        c.dismiss_ledger(THUMBS, newest(&c, THUMBS), 11).unwrap();
        assert!(!state(&c, THUMBS).shown);
        c.record_failures(FilePass::Thumbs, &[failure(2, "b")], 12).unwrap();
        let back = state(&c, THUMBS);
        assert!(back.shown && back.unread && back.since_dismissed, "{back:?}");
        // Opened: read up to what was listed. It stays listed, read.
        c.mark_ledger_read(THUMBS, back.newest.unwrap().serial, 13).unwrap();
        let read = state(&c, THUMBS);
        assert!(read.shown && !read.unread && read.since_dismissed, "{read:?}");
        assert_eq!(counts(&c, THUMBS), Counts { total: 2, new: 0, since_dismissed: 1 });
        let [entry] = c.ledger_entries(false).unwrap()[..] else { panic!("still listed") };
        assert!(entry.read);
    }

    // The import's ledger mirrors its failed analyses, by the bytes each read: a re-analysis that fails the same keeps
    // its serial, a changed file that fails again is new, and a fixed or deleted analysis takes its row.
    fn the_imports_ledger_mirrors_its_failed_analyses(rules: PathRules) {
        let c = catalog(rules);
        analysis(&c, 1, 10, 100, Some("it doesn't decode"));
        let [(1, serial)] = serials(&c, "import")[..] else { panic!("one row") };
        let [entry] = c.ledger_entries(false).unwrap()[..] else { panic!("one entry") };
        assert_eq!((entry.kind, entry.newest, entry.created), (IMPORT, serial, 5));
        assert_eq!(c.failed_files(FilePass::Import, 10).unwrap()[0].error, "it doesn't decode");
        // Again, from the same bytes (`--retry`): the same serial, the new error.
        analysis(&c, 1, 10, 100, Some("still no"));
        assert_eq!(serials(&c, "import"), [(1, serial)]);
        assert_eq!(c.failed_files(FilePass::Import, 10).unwrap()[0].error, "still no");
        // The file changed and fails again: new.
        raw::set_size(&c, FileId::new(1), 11);
        analysis(&c, 1, 11, 100, Some("no"));
        let [(1, changed)] = serials(&c, "import")[..] else { panic!("one row") };
        assert!(changed > serial);
        // Fixed: gone. A deleted analysis takes its row too.
        analysis(&c, 1, 11, 100, None);
        assert!(serials(&c, "import").is_empty());
        analysis(&c, 2, 10, 100, Some("no"));
        assert_eq!(serials(&c, "import").len(), 1);
        c.conn.execute("DELETE FROM analysis WHERE file_id = 2", []).unwrap();
        assert!(serials(&c, "import").is_empty());
        // Its Retry is the import's own (`gallery import --retry`): the catalog's leaves the rows.
        analysis(&c, 3, 10, 100, Some("no"));
        c.retry_failures(FilePass::Import).unwrap();
        assert_eq!(serials(&c, "import").len(), 1);
    }

    under_both_rules!(
        the_imports_ledger_mirrors_its_failed_analyses,
        the_imports_ledger_mirrors_its_failed_analyses_windows,
        the_imports_ledger_mirrors_its_failed_analyses_posix
    );

    // The import's rows are hidden while their file is missing or pending, and come back with their serials, so none
    // is news again: missing and back, a new IMPORT_VERSION failing the same bytes again, a thumbnail Retry, and a
    // metadata-only write-back (and a `--retry` after it).
    #[test]
    fn the_imports_rows_come_back_with_their_serials() {
        let c = catalog(PathRules::Posix);
        analysis(&c, 1, 10, 100, Some("no"));
        c.mark_ledger_read(IMPORT, newest(&c, IMPORT), 1).unwrap();
        let seen = serials(&c, "import");
        let quiet = |c: &Catalog, what: &str| {
            let s = state(c, IMPORT);
            assert!(s.shown && !s.new && !s.unread, "{what}: {s:?}");
            assert_eq!(serials(c, "import"), seen, "{what}");
        };
        // Missing, then back.
        raw::set_missing(&c, FileId::new(1));
        assert!(!state(&c, IMPORT).live);
        assert!(c.failed_files(FilePass::Import, 10).unwrap().is_empty());
        c.conn.execute("UPDATE files SET missing = 0 WHERE id = 1", []).unwrap();
        quiet(&c, "back from missing");
        // A new IMPORT_VERSION: pending, so not listed; failing again on the same bytes, listed as it was.
        raw::older_import_version(&c);
        raw::start_import_version(&c);
        assert!(!state(&c, IMPORT).live, "a pending file is not listed");
        assert!(c.failed_files(FilePass::Import, 10).unwrap().is_empty());
        analysis(&c, 1, 10, 100, Some("no"));
        quiet(&c, "a new import version");
        // A thumbnail Retry moves `files.seq`, which the import's ledger doesn't read.
        c.record_failures(FilePass::Thumbs, &[failure(1, "t")], 2).unwrap();
        c.retry_failures(FilePass::Thumbs).unwrap();
        quiet(&c, "a thumbnail retry");
        // A metadata-only write-back: the analysis follows the file, and so does its row; `--retry` failing the same
        // bytes again is the same failure.
        let rating = pg_model::rating::Rating::new(3).unwrap();
        let write = pg_model::write::Write::Rating(rating);
        let job = queue::Job { file_id: FileId::new(1), write, temp: None, attempts: 0 };
        let rewrite = queue::Rewrite {
            before: Some((10, 100)),
            after: (12, 102),
            metadata_only: Some(pg_model::analysis::FileHash([0; 32])),
        };
        c.write_done(&job, queue::WriteEffect::FileRating { rewrite, rating }).unwrap();
        let moved = raw::rows_of(&c, "SELECT size, mtime FROM file_failures WHERE pass = 'import'", [], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        });
        assert_eq!(moved, [(12, 102)]);
        quiet(&c, "a metadata write-back");
        analysis(&c, 1, 12, 102, Some("no"));
        quiet(&c, "a retry after the write-back");
    }

    // 10 failures seen, then 5 of them fixed and 5 new ones: 5 new, 5 earlier.
    #[test]
    fn ten_seen_five_fixed_and_five_new_is_five_new() {
        let c = catalog_of(PathRules::Posix, 15);
        for id in 1..=10 {
            analysis(&c, id, 10, 100, Some("no"));
        }
        c.mark_ledger_read(IMPORT, newest(&c, IMPORT), 1).unwrap();
        for id in 1..=5 {
            analysis(&c, id, 10, 100, None);
        }
        for id in 11..=15 {
            analysis(&c, id, 10, 100, Some("no"));
        }
        assert_eq!(counts(&c, IMPORT), Counts { total: 10, new: 5, since_dismissed: 0 });
        let [entry] = c.ledger_entries(true).unwrap()[..] else { panic!("one entry") };
        assert_eq!(entry.notice(), Notice::ImportFailed { counts: Some(LedgerCounts { new: 5, earlier: 5 }) });
    }

    // A thumbnail row survives an offline blip with its serial, so it is not news when the drive comes back.
    #[test]
    fn a_thumbs_row_survives_an_offline_blip() {
        let c = catalog(PathRules::Posix);
        c.record_failures(FilePass::Thumbs, &[failure(1, "a")], 10).unwrap();
        let before = serials(&c, "thumbs");
        c.mark_ledger_read(THUMBS, newest(&c, THUMBS), 11).unwrap();
        raw::set_missing(&c, FileId::new(1));
        assert!(!state(&c, THUMBS).shown);
        c.conn.execute("UPDATE files SET missing = 0 WHERE id = 1", []).unwrap();
        assert_eq!(serials(&c, "thumbs"), before);
        let back = state(&c, THUMBS);
        assert!(back.shown && !back.new && !back.unread, "{back:?}");
    }

    // A new thumbnailer tries the panicked files again (their seq moves on) and starts the ledger anew; a new borders
    // detector starts its ledger anew.
    #[test]
    fn a_new_version_starts_its_ledger_anew() {
        let c = catalog(PathRules::Posix);
        c.record_failures(FilePass::Thumbs, &[failure(1, "panicked"), Failure { retry: true, ..failure(2, "b") }], 1)
            .unwrap();
        c.record_failures(FilePass::Borders, &[failure(3, "c")], 1).unwrap();
        let (one, two) = (raw::file_seq(&c, FileId::new(1)), raw::file_seq(&c, FileId::new(2)));
        crate::lifecycle::set_thumbs_version(&c.conn, versions().thumbs).unwrap();
        assert_eq!(serials(&c, "thumbs").len(), 2, "the same version changes nothing");
        crate::lifecycle::set_thumbs_version(&c.conn, versions().thumbs + 1).unwrap();
        assert!(raw::file_seq(&c, FileId::new(1)) > one, "the panicked file is listed again");
        assert_eq!(raw::file_seq(&c, FileId::new(2)), two);
        assert!(c.recorded_failures(FilePass::Thumbs).unwrap().is_empty());
        crate::lifecycle::set_borders_version(&c.conn, 0).unwrap();
        assert!(c.recorded_failures(FilePass::Borders).unwrap().is_empty());
    }

    // A Retry lists exactly the failing files again for the incremental thumbnail pass: past its checkpoint
    // (`files.seq`).
    #[test]
    fn a_retry_relists_exactly_the_failing_files() {
        let c = catalog(PathRules::Posix);
        c.record_failures(FilePass::Thumbs, &[failure(1, "a"), failure(3, "c")], 1).unwrap();
        let checkpoint = raw::meta_seq(&c);
        c.retry_failures(FilePass::Thumbs).unwrap();
        let relisted =
            raw::rows_of(&c, "SELECT id FROM files WHERE seq > ?1 ORDER BY id", [checkpoint], |r| r.get::<_, i64>(0));
        assert_eq!(relisted, [1, 3]);
    }

    // The ledger's writer writes in its transaction: one that doesn't commit stores nothing.
    #[test]
    fn the_ledger_writer_writes_in_its_transaction() {
        let mut c = catalog(PathRules::Posix);
        let tx = c.tx().unwrap();
        assert!(tx.ledger_writer().record(FilePass::Borders, &[failure(1, "a")], 1).unwrap());
        drop(tx);
        assert!(serials(&c, "borders").is_empty(), "rolled back with the batch");
        let tx = c.tx().unwrap();
        tx.ledger_writer().record(FilePass::Borders, &[failure(1, "a"), failure(2, "b")], 1).unwrap();
        assert!(tx.ledger_writer().clear(FilePass::Borders, &[FileId::new(2)]).unwrap());
        assert!(tx.ledger_writer().record(FilePass::Import, &[failure(3, "c")], 1).is_err());
        tx.commit().unwrap();
        assert_eq!(serials(&c, "borders").iter().map(|s| s.0).collect::<Vec<_>>(), [1]);
    }

    // The inbox's reads of a ledger seek its serial index: a file pass's the live rows' partial index, which a query
    // uses only with its condition spelled as the index's (`live_failures!`), and in order (no sort).
    #[test]
    fn the_ledgers_reads_seek_their_index() {
        let c = catalog(PathRules::Posix);
        let plan = raw::query_plan(&c, FILE_LEDGER_NEWEST, ["thumbs"]);
        assert!(plan.contains("file_failures_live") && !plan.contains("TEMP B-TREE"), "{plan}");
        let plan = raw::query_plan(&c, FILE_LEDGER_OLDEST, ["import"]);
        assert!(plan.contains("file_failures_live") && !plan.contains("TEMP B-TREE"), "{plan}");
        let plan = raw::query_plan(&c, FILE_LEDGER_COUNTS, params!["thumbs", 0, Option::<i64>::None]);
        assert!(plan.contains("file_failures_live"), "{plan}");
        let plan = raw::query_plan(&c, FOLDER_LEDGER_NEWEST, []);
        assert!(plan.contains("folder_failures_serial") && !plan.contains("TEMP B-TREE"), "{plan}");
    }

    // At scale a record of 100k failures and their Retry are set-based statements (no per-row round trip): each well
    // within a bound in a debug build. The bound is wide on purpose; it catches a per-row regression, not a slow host.
    #[test]
    fn a_record_and_a_retry_of_100k_failures_are_set_based() {
        const N: i64 = 100_000;
        const BOUND: std::time::Duration = std::time::Duration::from_secs(30);
        let c = catalog_of(PathRules::Posix, 0);
        c.conn
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?1)
                 INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, last_scan, missing,
                                   rotation, order_key)
                 SELECT i, 1, i, 1, CAST(i || '.jpg' AS BLOB), 'jpg', 'image', 10, 100, 1, 0, 0,
                        CAST(printf('%08d', i) AS BLOB)
                 FROM n",
                [N],
            )
            .unwrap();
        let failures: Vec<Failure> = (1..=N).map(|i| failure(i, "no")).collect();
        let started = std::time::Instant::now();
        assert!(c.record_failures(FilePass::Thumbs, &failures, 1).unwrap());
        let recorded = started.elapsed();
        assert_eq!(c.recorded_failures(FilePass::Thumbs).unwrap().len(), 100_000);
        let checkpoint = raw::meta_seq(&c);
        let started = std::time::Instant::now();
        c.retry_failures(FilePass::Thumbs).unwrap();
        let retried = started.elapsed();
        assert!(!c.any_failures(FilePass::Thumbs).unwrap());
        let relisted =
            raw::rows_of(&c, "SELECT count(*) FROM files WHERE seq > ?1", [checkpoint], |r| r.get::<_, i64>(0));
        assert_eq!(relisted, [N]);
        assert!(recorded < BOUND && retried < BOUND, "record {recorded:?}, retry {retried:?}");
    }

    #[test]
    fn an_error_is_kept_on_one_line_and_bounded() {
        assert_eq!(super::failure::tidy("a\nb\tc"), "a b c");
        let long = "é".repeat(ERROR_BYTES);
        let kept = super::failure::tidy(&long);
        assert!(kept.len() <= ERROR_BYTES && kept.ends_with('…'), "{}", kept.len());
    }
}
