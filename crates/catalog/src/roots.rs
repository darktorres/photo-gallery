//! The library folders (`roots`) and their scans: which folders the library holds, a walk's start (with its lease) and
//! end, the files a finished walk didn't see, and the folder ledger's rows a walk records or sweeps. Methods of
//! `Catalog`.
//!
//! ### The folder ledger
//!
//! The folders a walk couldn't read are the `folder_failures` table: one row per folder, by its path key (`UNIQUE(path_key)`,
//! as `folders.path_key`, so a folder's spelling in another case is the same row), tied to the library folder that holds it (a foreign key: removing the library folder, or `add_root`
//! replacing the folders inside a new one, removes its rows), with the reason (`unreadable`, `mount-refused`, `refused`),
//! the error, the `scans` row of the walk that last saw it (`walk`), a serial and when it was first seen. It is the inbox's
//! `scan-folders-failed` entry, acknowledged as `scan-folders` in `ledger_ack` with the same rules; its detail lists the
//! folders grouped by library folder, with the reason and the error, Copy includes them, and Reveal opens a folder's parent.
//!
//! The rule is **written as it goes, swept at the end**:
//!
//! - **During the walk**, each folder it can't open or list, and each refused mount, is upserted inside the batch's
//!   transaction that commits around it (`ScanWriter`'s `folder_failed`), with `walk` this walk's id. The covering library
//!   folder is resolved in Rust once per walk (`types::covering_root`, since the Windows rules compare names
//!   Unicode-lowercased, which SQLite's `lower()` doesn't); the insert checks the library folder's id and path, so a walk
//!   of a folder no library folder covers, or one whose library folder was removed mid-walk, writes nothing and doesn't
//!   fail. An unchanged unreadable folder keeps its serial and `first_seen`: it is not news again.
//! - **At the end of a complete walk of folder F**, in `mark_missing`'s last transaction, the rows under F whose `walk` is
//!   older than this walk's go, except those strictly below a folder the walk skipped as unreadable or a refused mount: it
//!   never looked there, so it can't say they resolved. A reparse folder spares nothing. A root walk sweeps its library
//!   folder; a sub-folder walk sweeps exactly its subtree, so nothing is counted twice and nothing outside the walk is
//!   touched. `scans` ids only grow, and the scan lease means no overlapping walk runs meanwhile.
//! - **An incomplete walk sweeps nothing**: what it wrote is still true.
//! - **A refused root** (the walked folder itself: not NTFS, a FAT drive) is refused before any walk, so there is no scans
//!   row; `Catalog::folder_refused` writes it in a small transaction of its own with `walk` the newest `scans` id, so the
//!   next complete walk of it or of a folder above it sweeps it.
//!
//! The scan's other notes are split by what they describe (`ScanNote::lifecycle`): the conditions of a library folder
//! (`duplicates`, `reparse`, `ambiguous`, `unpaired`, `volume-id`) are states keyed `(root, code)` and replaced only by a
//! root walk; what one walk did (`replaced`, `rebound`) is an event of that walk, with its count.

use rusqlite::{Connection, OptionalExtension, params};

use pg_model::ids::{FileIdentity, RootId, ScanId};
use pg_model::num::count_u64;
use pg_model::rules::{Path as RulePath, PathRules};

use crate::convert::identity_to_sql;
use crate::core::{Catalog, ScanLease};
use crate::error::{Error, Result};
use crate::ledger::failure::kept_error;
use crate::removal::overlapping_marker;
use crate::transaction::write_transaction;
use crate::types::{
    FOLDER_FAILED, FolderRow, FolderRows, Prior, Root, SCAN_LEASE_SECS, Skip, SkipReason, Subtree, WRITE_BATCH,
    covering_root, ext_at, folders_under, kind_at, roots_on, skipped_folders, stored_path, under_params,
};
use pg_model::ledger::FolderFailure;
use pg_model::removal::Begun;

impl Catalog {
    /// The library folders, by path, with their ids.
    pub fn roots(&self) -> Result<Vec<Root>> {
        roots_on(&self.conn, self.rules)
    }

    /// The id of a library folder, by its path; `None` when the path is not one.
    pub fn root_id(&self, path: &RulePath) -> Result<Option<RootId>> {
        let id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM roots WHERE path = ?1", [self.rules.join(path)], |r| r.get(0))
            .optional()?;
        Ok(id.map(RootId::new))
    }

    /// Asks for library folder `root` to be walked, and keeps the request until its complete walk ends
    /// ([`Catalog::finish_scan_request`]), so a scan asked for before the app quits runs at the next start
    /// ([`Catalog::pending_scans`]). A path that is not a library folder asks for nothing.
    pub fn request_scan(&self, root: &RulePath) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO scan_requests(root) SELECT id FROM roots WHERE path = ?1",
            [self.rules.join(root)],
        )?;
        Ok(())
    }

    /// The library folders whose scan was asked for and has not completed, in the order they were asked for.
    pub fn pending_scans(&self) -> Result<Vec<RulePath>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT roots.path FROM scan_requests JOIN roots ON roots.id = scan_requests.root ORDER BY scan_requests.root",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
        let mut paths = Vec::new();
        for row in rows {
            paths.push(stored_path(self.rules, &row?)?);
        }
        Ok(paths)
    }

    /// The scan request of library folder `root` is done: its complete walk ended. A folder with no request is no change.
    pub fn finish_scan_request(&self, root: &RulePath) -> Result<()> {
        self.conn.execute(
            "DELETE FROM scan_requests WHERE root = (SELECT id FROM roots WHERE path = ?1)",
            [self.rules.join(root)],
        )?;
        Ok(())
    }

    /// Add a folder to the library. A folder already inside the library adds nothing (returns false); one that
    /// contains library folders replaces them (their notices and their folder ledger's rows go with them).
    pub fn add_root(&self, root: &RulePath) -> Result<bool> {
        // One transaction: the roots it replaces go only with the insert that replaces them (a crash between the two
        // would lose them), and the check reads what the replacement will see.
        let tx = write_transaction(&self.conn)?;
        // A folder whose removal is not finished can't come back until it is (its files are still going).
        if let Some(removing) = overlapping_marker(&tx, self.rules, root)? {
            return Err(Error::RemovalPending { path: self.rules.display(&removing) });
        }
        let roots = roots_on(&tx, self.rules)?;
        if roots.iter().any(|r| self.rules.is_under(root, &r.path)) {
            return Ok(false);
        }
        for r in roots.iter().filter(|r| self.rules.is_under(&r.path, root)) {
            tx.execute("DELETE FROM roots WHERE id = ?1", [r.id.get()])?;
        }
        tx.execute("INSERT INTO roots(path, added) VALUES (?1, unixepoch())", [self.rules.join(root)])?;
        tx.commit()?;
        Ok(true)
    }

    /// Take a folder out of the library, and its files out of the catalog for good: the removal run to its end
    /// (`begin_removal`, `hide_pages`, `drive_removal`, for this folder's marker alone) without thumbnails (the caller forgets them). Returns
    /// how many files went; `None`, with nothing changed, when `root` is not a library folder. A file with an unfinished
    /// write is kept, and so is its folder, and the marker stays for the write (or a discard) to finish.
    ///
    /// The walks of the folder, or of folders inside it, end: their leases go in the same transaction as the root, so
    /// each walk's next write finds its lease gone and writes nothing ([`Error::RootRemoved`]).
    pub fn remove_root(&self, root: RootId) -> Result<Option<u64>> {
        let Some(path) = self.roots()?.into_iter().find(|r| r.id == root).map(|r| r.path) else { return Ok(None) };
        match self.begin_removal(root, &path)? {
            Begun::Started { path, .. } => {
                self.hide_pages(&path)?;
                Ok(Some(self.drive_removal(&path, &mut |_| true)?.files))
            }
            Begun::NotFound | Begun::Changed | Begun::Pending(_) => Ok(None),
        }
    }

    /// Begin a walk of `root`: its `scans` row (the id the walk stamps its writes with), and its lease, in one write
    /// transaction. Refused with [`Error::ScanBusy`], with nothing written, while another walk of a folder that overlaps
    /// `root` (either is inside the other, by the rules) holds an unexpired lease, in this process or another; a lease
    /// with no heartbeat for [`SCAN_LEASE_SECS`] is a crashed walk's, and goes. The walk reads and writes through the
    /// lease, which gives itself back when the walk ends however it ends ([`ScanLease`]).
    pub fn begin_scan(&mut self, root: &RulePath) -> Result<ScanLease<'_>> {
        let rules = self.rules;
        let bytes = rules.join(root);
        ensure_seen(&self.conn)?;
        let tx = write_transaction(&self.conn)?;
        // A walk is of a library folder, or of one below it: a folder the library no longer holds (removed, or never
        // added) is not walked, and nothing is written for it.
        if covering_root(&tx, rules, root)?.is_none() {
            return Err(Error::RootRemoved { root: root.clone() });
        }
        tx.execute("DELETE FROM scan_leases WHERE heartbeat < unixepoch() - ?1", [SCAN_LEASE_SECS])?;
        let held: Vec<(Vec<u8>, String)> = tx
            .prepare("SELECT root, holder FROM scan_leases ORDER BY id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (other, holder) in held {
            let other = stored_path(rules, &other)?;
            if rules.is_under(root, &other) || rules.is_under(&other, root) {
                return Err(Error::ScanBusy { root: other, holder });
            }
        }
        let scan = open_walk(&tx, &bytes, &lease_holder())?;
        // A walk starts with no rows seen (the set is the walk's, and a walk that ended emptied it already).
        tx.execute("DELETE FROM temp.scan_seen", [])?;
        tx.commit()?;
        Ok(ScanLease::new(self, scan, root.clone()))
    }

    /// Records that the scan `scan` finished, stamping its `finished` time now.
    pub fn finish_scan(&self, scan: ScanId) -> Result<()> {
        self.conn.execute("UPDATE scans SET finished = unixepoch() WHERE id = ?1", [scan.get()])?;
        Ok(())
    }

    /// The scans begun and not finished: a walk that was cut short (a stop, a failure) and never finished. A run that was not
    /// interrupted owes none.
    pub fn unfinished_scans(&self) -> Result<u64> {
        let count: i64 =
            self.conn.query_row("SELECT count(*) FROM scans WHERE finished IS NULL", [], |row| row.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}

impl ScanLease<'_> {
    /// The end of a complete walk of the lease's folder. Files under it that the walk didn't see are marked missing;
    /// their rows stay (ratings, decisions). Files in `skipped` folders (and their subfolders) are left as they are: the
    /// walk didn't list them (it couldn't, they are mounts it refuses, or reparse folders it doesn't follow), so it saw
    /// nothing there to compare with. The marks are batches of at most `WRITE_BATCH` (1 000) folders or files, each in a short
    /// transaction of its own, so no other writer waits behind the whole pass.
    ///
    /// Then, in one last transaction, the folder ledger's rows under the folder that this walk didn't see (`walk`
    /// before it) are resolved and go, except those strictly below a folder it skipped as unreadable or a refused mount
    /// (it never looked there; a reparse folder spares nothing, since what is below it can't resolve while it is one),
    /// and the walk's lease is given back. The number of files marked.
    ///
    /// Each transaction holds the lease first ([`ScanLease::renew`]'s check): one that is gone (the folder removed
    /// from the library, which took it) writes nothing, and the marks end with [`Error::RootRemoved`] or
    /// [`Error::LeaseLost`]. Another walk of the folder may have begun then, whose rows these marks would hide.
    pub fn mark_missing(&self, skipped: &[Skip]) -> Result<u64> {
        let (rules, root, scan) = (self.rules, self.root(), self.id());
        let held = self.held();
        let mut pass = pg_perf::span("pass.mark_missing");
        let tx_at = crate::transaction::tx_mark();
        // The present rows the walk did not see, read once outside any write transaction: the lease keeps any other walk
        // of these folders out. Only the rows left are written, by id, in batches of `WRITE_BATCH`, and each write repeats
        // the read's predicate, so a row that became seen or missing since is left alone.
        let read = pg_perf::span("mark_missing.read");
        let unseen: Vec<(i64, i64)> = {
            let snapshot = self.conn.unchecked_transaction()?;
            let skipped = skipped_folders(&snapshot, rules, skipped)?;
            let folders: Vec<i64> =
                folders_under(&snapshot, rules, root)?.into_iter().filter(|f| !skipped.contains(f)).collect();
            let mut out = Vec::new();
            {
                let mut read = snapshot.prepare_cached(concat!(
                    "SELECT id FROM files WHERE folder_id = ?1 AND missing = 0 AND ",
                    not_seen!()
                ))?;
                for folder in &folders {
                    for id in read.query_map([folder], |r| r.get::<_, i64>(0))? {
                        out.push((id?, *folder));
                    }
                }
            }
            snapshot.commit()?;
            out
        };
        drop(read);
        let mut write = pg_perf::span("mark_missing.write");
        let mut marked: u64 = 0;
        for batch in unseen.chunks(WRITE_BATCH) {
            let tx = write_transaction(&self.conn)?;
            held.hold(&tx)?;
            {
                let mut mark = tx.prepare_cached(concat!(
                    "UPDATE files SET missing = 1 WHERE id = ?1 AND folder_id = ?2 AND missing = 0 AND ",
                    not_seen!()
                ))?;
                for &(id, folder) in batch {
                    marked += count_u64(mark.execute(params![id, folder])?);
                }
            }
            tx.commit()?;
        }
        write.items(marked);
        drop(write);
        let sweep = pg_perf::span("mark_missing.sweep");
        let tx = write_transaction(&self.conn)?;
        held.hold(&tx)?;
        sweep_folder_failures(&tx, rules, root, scan, skipped)?;
        tx.execute("DELETE FROM scan_leases WHERE id = ?1", [scan.get()])?;
        tx.execute("DELETE FROM temp.scan_seen", [])?;
        tx.commit()?;
        drop(sweep);
        pass.items(marked);
        crate::transaction::attach_tx(&mut pass, tx_at);
        Ok(marked)
    }
}

impl Catalog {
    /// A folder refused before a walk of it began (not NTFS, a FAT drive, and so on: the walked folder itself, so there
    /// is no walk and no scan id), with its error, in the folder ledger, in a small transaction of its own. Its `walk`
    /// is the newest `scans` row: any later walk has a larger id, so the next complete walk of the folder (or of one
    /// above it) sweeps the row, and a walk already running never does. Nothing is written when no library folder holds
    /// it. Whether a row was written.
    pub fn folder_refused(&self, path: &RulePath, error: &str) -> Result<bool> {
        let tx = write_transaction(&self.conn)?;
        let Some(root) = covering_root(&tx, self.rules, path)? else { return Ok(false) };
        let walk: i64 = tx.query_row("SELECT coalesce(max(id), 0) FROM scans", [], |r| r.get(0))?;
        let written = tx.execute(
            FOLDER_FAILED,
            params![
                root.id.get(),
                self.rules.join(&root.path),
                self.rules.join(path),
                self.rules.path_key(path),
                FolderFailure::Refused.key(),
                kept_error(error),
                walk
            ],
        )?;
        tx.commit()?;
        Ok(written > 0)
    }

    /// The rows of `folder` as they are now, with the change number now as their mark ([`FolderRows`]), read in one
    /// snapshot outside any transaction of a walk's. A walk reads them just before it lists the folder: so a write-back
    /// that commits after the read, while the folder is listed or later, leaves a row that no longer equals what was
    /// read, and the batch only marks the file seen ([`crate::ScannedFile::prior`]). Read after the listing, they
    /// would be the write-back's, and the batch would write the older listing over them.
    pub fn folder_rows(&self, folder: &RulePath) -> Result<FolderRows> {
        folder_rows_on(&self.conn, self.rules, folder)
    }

    /// The rows of `identities`, the files a walk listed in the folder whose rows are `rows` (read before the listing,
    /// [`Catalog::folder_rows`]), as the batch is to compare them ([`crate::ScannedFile::prior`]): `None` for an
    /// identity with no row.
    ///
    /// A listed file's row is among the folder's but for a file new to the folder (or to the catalog); those are looked
    /// up by identity, one by one, in one snapshot, after the listing (it gives the identities). Order alone doesn't
    /// make that read safe: a write-back opens the file by its row's path, and a hard link, or a file moved away and
    /// back, reaches the same file by another name, so it can rewrite the file the walk listed in this folder, and commit,
    /// before this read. So a row whose change number is past the folder's mark, which changed after the listing began,
    /// is `None` too: the file is only marked seen, and the next walk records it. An import or a rating in that
    /// window bumps the number as well, and recording the file's move waits for the next walk; nothing is lost.
    pub fn prior_states(&self, rows: &FolderRows, identities: &[FileIdentity]) -> Result<Vec<Option<Prior>>> {
        let key = |identity: &FileIdentity| (identity_to_sql(identity.volume), identity_to_sql(identity.file_index));
        let mut out: Vec<Option<Prior>> =
            identities.iter().map(|i| rows.rows.get(&key(i)).map(|r| Prior { size: r.size, mtime: r.mtime })).collect();
        if out.iter().all(Option::is_some) {
            return Ok(out);
        }
        let snapshot = self.conn.unchecked_transaction()?;
        {
            let mut by_identity = snapshot.prepare_cached(
                "SELECT size, mtime, changed FROM files
                 WHERE volume = (SELECT id FROM volumes WHERE serial = ?1) AND file_ref = ?2",
            )?;
            for (identity, prior) in identities.iter().zip(out.iter_mut()).filter(|(_, prior)| prior.is_none()) {
                let (volume, file_ref) = key(identity);
                let row = by_identity
                    .query_row(params![volume, file_ref], |r| {
                        Ok((Prior { size: r.get(0)?, mtime: r.get(1)? }, r.get::<_, i64>(2)?))
                    })
                    .optional()?;
                *prior = row.filter(|&(_, changed)| changed <= rows.mark).map(|(row, _)| row);
            }
        }
        snapshot.commit()?;
        Ok(out)
    }
}

/// The rows of `folder` on `conn`, read in one snapshot with the change number as their mark ([`Catalog::folder_rows`]).
pub(crate) fn folder_rows_on(conn: &Connection, rules: PathRules, folder: &RulePath) -> Result<FolderRows> {
    let snapshot = conn.unchecked_transaction()?;
    let rows = {
        let mark: i64 =
            snapshot.prepare_cached("SELECT value FROM meta WHERE key = 'seq'")?.query_row([], |r| r.get(0))?;
        let path_key = rules.path_key(folder);
        // A folder new to the catalog has no row, and no files: its merged folder is none (0, no `mfolders` id).
        let mid: i64 = snapshot
            .prepare_cached("SELECT mid FROM folders WHERE path_key = ?1")?
            .query_row([&path_key], |r| r.get(0))
            .optional()?
            .unwrap_or(0);
        // The rows by identity: the volume's serial (the interned id's), and the reference.
        let rows = snapshot
            .prepare_cached(
                "SELECT v.serial, f.file_ref, f.id, f.unpaired, f.missing, f.name, f.ext, f.kind, f.size, f.mtime, f.mid,
                        f.created
                 FROM files f JOIN volumes v ON v.id = f.volume
                 WHERE f.folder_id = (SELECT id FROM folders WHERE path_key = ?1)",
            )?
            .query_map([&path_key], |r| {
                let row = FolderRow {
                    id: r.get(2)?,
                    unpaired: r.get::<_, i64>(3)? != 0,
                    missing: r.get::<_, i64>(4)? != 0,
                    name: r.get(5)?,
                    ext: ext_at(r, 6)?,
                    kind: kind_at(r, 7)?,
                    size: r.get(8)?,
                    mtime: r.get(9)?,
                    mid: r.get(10)?,
                    created: r.get(11)?,
                };
                Ok(((r.get(0)?, r.get(1)?), row))
            })?
            .collect::<rusqlite::Result<_>>()?;
        FolderRows { mark, mid, rows }
    };
    snapshot.commit()?;
    Ok(rows)
}

/// The page cache of a [`FolderRowReader`], in KiB: a folder's rows, read a folder at a time.
const READER_CACHE_KIB: u32 = 2048;

/// A read-only connection of a scan's own, for its listing workers (`pg_scan`): a folder's rows as they are, read on the
/// worker's thread before it lists the folder ([`Catalog::folder_rows`]). It opens the catalog's file with a shared hold
/// of its own (`take_shared`), so it does not depend on the catalog being open, and it is made on the thread that reads
/// with it. An in-memory catalog has no file another connection could open, so its scans run without workers.
pub struct FolderRowReader {
    conn: Connection,
    rules: PathRules,
    /// Keeps the catalog's file shared while the reader lives: a rebuild of the file waits for it.
    _hold: pg_dbfile::Live,
}

impl FolderRowReader {
    /// A reader of the catalog at `file` (its [`Catalog::file`]). It fails while another program is changing the file.
    pub fn open(file: &str, rules: PathRules) -> Result<FolderRowReader> {
        let hold = crate::lifecycle::take_shared(std::path::Path::new(file))?;
        let conn = crate::core::read_only(file, rules, READER_CACHE_KIB)?;
        Ok(FolderRowReader { conn, rules, _hold: hold })
    }

    /// The rows of `folder` as they are now, with their mark ([`Catalog::folder_rows`]).
    pub fn folder_rows(&self, folder: &RulePath) -> Result<FolderRows> {
        folder_rows_on(&self.conn, self.rules, folder)
    }
}

/// The walk's set of the rows it has seen (`temp.scan_seen`): a temporary table of this connection, made here by the first
/// walk on it and kept. Its partial index holds the unpaired rows, which the pairing reads. Emptied when a walk begins and
/// when its lease is given back.
pub(crate) fn ensure_seen(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS scan_seen(id INTEGER PRIMARY KEY, unpaired INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS temp.scan_seen_unpaired ON scan_seen(id) WHERE unpaired = 1;",
    )?;
    Ok(())
}

impl ScanLease<'_> {
    /// Records rows as seen by this walk, in the set (`(id, unpaired)`), in one deferred transaction that touches only the
    /// temporary database: it neither waits for nor holds the catalog's write lock.
    pub fn mark_seen(&self, rows: &[(i64, bool)]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut insert = tx.prepare_cached("INSERT OR IGNORE INTO temp.scan_seen(id, unpaired) VALUES (?1, ?2)")?;
            for &(id, unpaired) in rows {
                insert.execute(params![id, unpaired])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

/// Who holds a lease this process takes, for the words of another's refusal.
fn lease_holder() -> String {
    format!("process {}", std::process::id())
}

/// Opens a walk of the folder `bytes` (as stored): its `scans` row, and its lease held by `holder`, on `conn` (a
/// transaction, for `begin_scan`, or the connection in a test). Returns the walk's id.
pub(crate) fn open_walk(conn: &Connection, bytes: &[u8], holder: &str) -> rusqlite::Result<ScanId> {
    conn.execute("INSERT INTO scans(root, started) VALUES (?1, unixepoch())", [bytes])?;
    let scan = ScanId::new(conn.last_insert_rowid());
    conn.execute(
        "INSERT INTO scan_leases(id, root, holder, heartbeat) VALUES (?1, ?2, ?3, unixepoch())",
        params![scan.get(), bytes, holder],
    )?;
    Ok(scan)
}

/// The folder ledger's sweep at the end of a complete walk of `root` (`scan`), on `conn` ([`ScanLease::mark_missing`]).
fn sweep_folder_failures(
    conn: &Connection,
    rules: PathRules,
    root: &RulePath,
    scan: ScanId,
    skipped: &[Skip],
) -> Result<()> {
    let under = under_params(rules, root);
    let rows: Vec<(i64, Vec<u8>)> = conn
        .prepare_cached(
            "SELECT id, path_key FROM folder_failures WHERE path_key >= ?1 AND (?2 IS NULL OR path_key < ?2) AND walk < ?3",
        )?
        .query_map(params![under.key, under.upper, scan.get()], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    // The subtrees the walk never looked into: a row strictly below one of them is not known to be resolved. A row's
    // own folder is in its subtree, so the row's key equal to a subtree's key is the subtree's own row.
    let unseen: Vec<Subtree> = skipped
        .iter()
        .filter(|s| matches!(s.reason, SkipReason::Unreadable | SkipReason::RefusedMount))
        .map(|s| under_params(rules, &s.path))
        .collect();
    let mut delete = conn.prepare_cached("DELETE FROM folder_failures WHERE id = ?1")?;
    for (id, key) in rows {
        let below_unseen = unseen.iter().any(|s| key > s.key && s.upper.as_ref().is_none_or(|u| key < *u));
        if !below_unseen {
            delete.execute([id])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::browse::fixtures::{spelled, under_both_rules};
    use crate::fixture::{self, WriteHolds, raw};
    use crate::ledger::Failure;
    use crate::test_support::{begin, lease, listed, p, rooted, versions};
    use crate::types::rules_name;
    use crate::{Catalog, Error, Prior, Recorded, SCAN_LEASE_SECS, ScanLease, ScannedFile, Skip, SkipReason};
    use pg_model::ids::{FileIdentity, ScanId};
    use pg_model::ledger::FolderFailure;
    use pg_model::media::Ext;
    use pg_model::name::Name;
    use pg_model::notice::{FilePass, LedgerCounts, LedgerKind, Notice};
    use pg_model::rules::PathRules;
    use pg_model::turns::Turns;

    const UNREADABLE: FolderFailure = FolderFailure::Unreadable;

    /// A catalog whose library is `D:\r`.
    /// A walk of `root` that couldn't take the folders `failed` (each recorded, and skipped as the walk skips it) and
    /// skipped `also`: complete (its end sweeps the ledger) or stopped (its lease dropped, nothing swept). Its id.
    fn walk(c: &mut Catalog, root: &str, failed: &[(&str, FolderFailure)], also: &[Skip], complete: bool) -> ScanId {
        let rules = c.rules();
        let root = p(rules, root);
        let mut lease = c.begin_scan(&root).unwrap();
        let scan = lease.id();
        let mut skipped = also.to_vec();
        let tx = lease.tx().unwrap();
        let mut writer = tx.scan_writer(scan, &root).unwrap();
        for &(folder, failure) in failed {
            assert!(writer.folder_failed(&p(rules, folder), failure, "access is denied").unwrap());
            let reason = match failure {
                FolderFailure::MountRefused => SkipReason::RefusedMount,
                FolderFailure::Unreadable | FolderFailure::Refused => SkipReason::Unreadable,
            };
            skipped.push(Skip { path: p(rules, folder), reason });
        }
        drop(writer);
        tx.commit().unwrap();
        if complete {
            lease.mark_missing(&skipped).unwrap();
        }
        scan
    }

    /// The folder ledger's rows: path (as text), reason and serial, by path.
    fn ledger(c: &Catalog) -> Vec<(String, String, i64)> {
        raw::rows_of(c, "SELECT CAST(path AS TEXT), reason, serial FROM folder_failures ORDER BY path", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
    }

    fn paths(c: &Catalog) -> Vec<String> {
        ledger(c).into_iter().map(|row| row.0).collect()
    }

    /// The paths, as `rules` store them, in the ledger's order (their bytes').
    fn stored(rules: PathRules, texts: &[&str]) -> Vec<String> {
        let mut paths: Vec<String> = texts.iter().map(|t| spelled(rules, t)).collect();
        paths.sort();
        paths
    }

    // A walk records each folder it couldn't take; a complete walk of a folder resolves the rows of its subtree it
    // didn't see, and only those: a walk of a folder below replaces its subtree's, and a later walk of the library
    // folder that takes everything resolves the rest.
    fn a_complete_walk_resolves_the_folders_of_its_subtree(rules: PathRules) {
        let mut c = rooted(rules);
        walk(&mut c, r"D:\r", &[(r"D:\r\a", UNREADABLE), (r"D:\r\b\c", UNREADABLE)], &[], true);
        assert_eq!(paths(&c), stored(rules, &[r"D:\r\a", r"D:\r\b\c"]));
        walk(&mut c, r"D:\r\b", &[(r"D:\r\b\d", UNREADABLE)], &[], true);
        assert_eq!(paths(&c), stored(rules, &[r"D:\r\a", r"D:\r\b\d"]));
        walk(&mut c, r"D:\r", &[], &[], true);
        assert!(paths(&c).is_empty());
    }

    under_both_rules!(
        a_complete_walk_resolves_the_folders_of_its_subtree,
        a_complete_walk_resolves_the_folders_of_its_subtree_windows,
        a_complete_walk_resolves_the_folders_of_its_subtree_posix
    );

    // The subtree is a range of bytes: a sibling whose name starts with the folder's (`X2`), or has a byte a pattern
    // would take for a wildcard (`X_`, `X%`), is not in it.
    fn the_sweep_takes_the_subtree_by_its_bytes(rules: PathRules) {
        let mut c = rooted(rules);
        let all = [r"D:\r\X\y", r"D:\r\X2\y", r"D:\r\X_\y", r"D:\r\X%\y"];
        let failed: Vec<(&str, FolderFailure)> = all.iter().map(|f| (*f, UNREADABLE)).collect();
        walk(&mut c, r"D:\r", &failed, &[], true);
        assert_eq!(paths(&c), stored(rules, &all));
        walk(&mut c, r"D:\r\X", &[], &[], true);
        assert_eq!(paths(&c), stored(rules, &[r"D:\r\X2\y", r"D:\r\X_\y", r"D:\r\X%\y"]));
    }

    under_both_rules!(
        the_sweep_takes_the_subtree_by_its_bytes,
        the_sweep_takes_the_subtree_by_its_bytes_windows,
        the_sweep_takes_the_subtree_by_its_bytes_posix
    );

    // A row is a folder by its path key: one row per folder, so two spellings of one folder are one row under the Windows
    // rules (which fold case), and two rows under the POSIX rules (whose names are case-sensitive). The row keeps the
    // spelling the walk last saw. A folder recorded again keeps its serial (it is not news again), whatever its reason now.
    fn a_folder_is_one_row_by_its_path_key_and_keeps_its_serial(rules: PathRules) {
        let mut c = rooted(rules);
        let both = [(r"D:\r\a", UNREADABLE), (r"D:\r\A", UNREADABLE)];
        walk(&mut c, r"D:\r", &both, &[], true);
        let first = ledger(&c);
        let rows = match rules {
            PathRules::Windows => 1,
            PathRules::Posix => 2,
        };
        assert_eq!(first.len(), rows, "{first:?}");
        walk(&mut c, r"D:\r", &both, &[], true);
        assert_eq!(ledger(&c), first, "the same rows, the same serials");
        walk(&mut c, r"D:\r", &[(r"D:\r\a", FolderFailure::MountRefused)], &[], true);
        // The row the last walk saw under this spelling: under Windows the one row (spelled `A` by the walk before).
        let seen = match rules {
            PathRules::Windows => r"D:\r\A",
            PathRules::Posix => r"D:\r\a",
        };
        let kept = first.iter().find(|row| row.0 == spelled(rules, seen)).unwrap();
        assert_eq!(ledger(&c), [(spelled(rules, r"D:\r\a"), "mount-refused".to_string(), kept.2)]);
    }

    under_both_rules!(
        a_folder_is_one_row_by_its_path_key_and_keeps_its_serial,
        a_folder_is_one_row_by_its_path_key_and_keeps_its_serial_windows,
        a_folder_is_one_row_by_its_path_key_and_keeps_its_serial_posix
    );

    // The insert trigger stamps a new folder's serial past the ones already there (`folder_failures_stamp`). A
    // folder recorded again keeps its stamp, so the walk that follows sees only the new one as news.
    fn a_new_folder_failure_is_stamped_past_the_older_ones(rules: PathRules) {
        let mut c = rooted(rules);
        walk(&mut c, r"D:\r", &[(r"D:\r\a", UNREADABLE)], &[], true);
        let first = ledger(&c);
        assert_eq!(first.len(), 1, "{first:?}");
        assert!(first[0].2 > 0, "the insert is stamped: {first:?}");
        walk(&mut c, r"D:\r", &[(r"D:\r\a", UNREADABLE), (r"D:\r\b", UNREADABLE)], &[], true);
        let both = ledger(&c);
        let a = both.iter().find(|row| row.0 == spelled(rules, r"D:\r\a")).unwrap();
        let b = both.iter().find(|row| row.0 == spelled(rules, r"D:\r\b")).unwrap();
        assert_eq!(a.2, first[0].2, "a folder recorded again keeps its serial");
        assert!(b.2 > a.2, "a new folder is stamped past the older one: {both:?}");
    }

    under_both_rules!(
        a_new_folder_failure_is_stamped_past_the_older_ones,
        a_new_folder_failure_is_stamped_past_the_older_ones_windows,
        a_new_folder_failure_is_stamped_past_the_older_ones_posix
    );

    // Only a complete walk resolves anything. Below a folder it couldn't read the rows stay (it never looked), below a
    // reparse folder they go (nothing below one can resolve while it is one), and a reparse folder is no row. The rows
    // go with their library folder: removed, or replaced by one above it.
    fn the_ledger_follows_complete_walks_and_the_library(rules: PathRules) {
        let mut c = rooted(rules);
        walk(&mut c, r"D:\r", &[(r"D:\r\a\b", UNREADABLE), (r"D:\r\j\k", UNREADABLE)], &[], true);
        walk(&mut c, r"D:\r", &[], &[], false);
        assert_eq!(paths(&c).len(), 2, "a stopped walk resolves nothing");
        let reparse = Skip { path: p(rules, r"D:\r\j"), reason: SkipReason::Reparse };
        walk(&mut c, r"D:\r", &[(r"D:\r\a", UNREADABLE)], &[reparse], true);
        assert_eq!(paths(&c), stored(rules, &[r"D:\r\a", r"D:\r\a\b"]));
        let root = c.root_id(&p(rules, r"D:\r")).unwrap().unwrap();
        c.remove_root(root).unwrap();
        assert!(paths(&c).is_empty(), "removed with its library folder");
        assert!(c.add_root(&p(rules, r"D:\r\s")).unwrap());
        walk(&mut c, r"D:\r\s", &[(r"D:\r\s\t", UNREADABLE)], &[], true);
        assert_eq!(paths(&c).len(), 1);
        assert!(c.add_root(&p(rules, r"D:\r")).unwrap());
        assert!(paths(&c).is_empty(), "replaced by the library folder above it");
    }

    under_both_rules!(
        the_ledger_follows_complete_walks_and_the_library,
        the_ledger_follows_complete_walks_and_the_library_windows,
        the_ledger_follows_complete_walks_and_the_library_posix
    );

    // A sub-folder refused by its file system (a mount, or a file system the library refuses) is a row of the ledger
    // while the walk of its parent refuses it, and the next complete walk of the parent, which sees it again, sweeps it.
    fn a_refused_sub_folder_is_swept_by_its_parents_next_complete_walk(rules: PathRules) {
        let mut c = rooted(rules);
        let refused = [(r"D:\r\m", FolderFailure::MountRefused), (r"D:\r\n", FolderFailure::Refused)];
        walk(&mut c, r"D:\r", &refused, &[], true);
        assert_eq!(paths(&c), stored(rules, &[r"D:\r\m", r"D:\r\n"]));
        let reasons: Vec<String> = ledger(&c).into_iter().map(|row| row.1).collect();
        assert_eq!(reasons, ["mount-refused", "refused"]);
        walk(&mut c, r"D:\r", &[], &[], true);
        assert!(paths(&c).is_empty(), "the parent's complete walk resolves them");
    }

    under_both_rules!(
        a_refused_sub_folder_is_swept_by_its_parents_next_complete_walk,
        a_refused_sub_folder_is_swept_by_its_parents_next_complete_walk_windows,
        a_refused_sub_folder_is_swept_by_its_parents_next_complete_walk_posix
    );

    // Nothing is recorded for a folder no library folder holds, nor once its library folder is removed mid-walk.
    #[test]
    fn a_folder_outside_the_library_is_not_recorded() {
        let rules = PathRules::Posix;
        let mut c = rooted(rules);
        let elsewhere = p(rules, r"D:\elsewhere");
        assert!(
            matches!(c.begin_scan(&elsewhere).err(), Some(Error::RootRemoved { .. })),
            "a walk of a folder no library folder holds is refused"
        );
        // Removed between two batches of a walk: the removal took the walk's lease, so the next batch writes nothing,
        // and says why.
        let root = p(rules, r"D:\r");
        let mut lease = c.begin_scan(&root).unwrap();
        let scan = lease.id();
        let id = lease.root_id(&root).unwrap().unwrap();
        lease.remove_root(id).unwrap();
        let tx = lease.tx().unwrap();
        assert!(matches!(tx.scan_writer(scan, &root).err(), Some(Error::RootRemoved { .. })));
        drop(tx);
        assert!(paths(&lease).is_empty());
    }

    // Removing a library folder ends the walks of it and of the folders inside it, in any process: their leases go with
    // the marks, and each one's next write (a batch, the missing marks, a renewal) writes nothing and says why. A walk of
    // another library folder goes on. A walk whose lease expired and was taken over is told apart: its folder is still
    // the library's.
    fn removing_a_folder_ends_the_walks_under_it(rules: PathRules) {
        let mut c = rooted(rules);
        assert!(c.add_root(&p(rules, r"D:\s")).unwrap());
        let (r, sub, s) = (p(rules, r"D:\r"), p(rules, r"D:\r\sub"), p(rules, r"D:\s"));
        let (of_r, of_sub, of_s) = (begin(&mut c, &r), begin(&mut c, &sub), begin(&mut c, &s));
        let id = c.root_id(&r).unwrap().unwrap();
        c.remove_root(id).unwrap();
        let removed = |e: Option<Error>| matches!(e, Some(Error::RootRemoved { .. }));
        {
            let tx = c.tx().unwrap();
            assert!(removed(tx.scan_writer(of_r, &r).err()), "a batch of the removed folder's walk");
            assert!(removed(tx.scan_writer(of_sub, &sub).err()), "a batch of a walk of a folder inside it");
            drop(tx.scan_writer(of_s, &s).unwrap());
        }
        assert!(removed(lease(&mut c, of_r, &r).mark_missing(&[]).err()), "the missing marks");
        assert!(removed(lease(&mut c, of_sub, &sub).renew().err()), "a renewal");
        c.conn.execute("DELETE FROM scan_leases WHERE id = ?1", [of_s.get()]).unwrap();
        assert!(matches!(lease(&mut c, of_s, &s).renew(), Err(Error::LeaseLost { .. })), "an expired lease");
    }

    under_both_rules!(
        removing_a_folder_ends_the_walks_under_it,
        removing_a_folder_ends_the_walks_under_it_windows,
        removing_a_folder_ends_the_walks_under_it_posix
    );

    // A folder refused before its walk began is a row of the ledger (one, however often refused); a walk already
    // running doesn't resolve it, the next complete walk does.
    fn a_refused_folder_is_resolved_by_its_next_complete_walk(rules: PathRules) {
        let mut c = rooted(rules);
        let root = p(rules, r"D:\r");
        let lease = c.begin_scan(&root).unwrap();
        assert!(lease.folder_refused(&root, "not an NTFS volume").unwrap());
        assert!(lease.folder_refused(&root, "not an NTFS volume").unwrap());
        let rows = ledger(&lease);
        assert_eq!(
            rows.iter().map(|r| (r.0.as_str(), r.1.as_str())).collect::<Vec<_>>(),
            [(spelled(rules, r"D:\r").as_str(), "refused")]
        );
        lease.mark_missing(&[]).unwrap();
        assert_eq!(ledger(&lease).len(), 1, "the walk running when it was refused doesn't resolve it");
        drop(lease);
        walk(&mut c, r"D:\r", &[], &[], true);
        assert!(ledger(&c).is_empty());
        assert!(!c.folder_refused(&p(rules, r"D:\elsewhere"), "not NTFS").unwrap(), "no library folder holds it");
    }

    under_both_rules!(
        a_refused_folder_is_resolved_by_its_next_complete_walk,
        a_refused_folder_is_resolved_by_its_next_complete_walk_windows,
        a_refused_folder_is_resolved_by_its_next_complete_walk_posix
    );

    // The scans' ledger is one inbox entry, its folders the entry's detail, grouped by library folder; new until read,
    // and counted apart from the earlier ones once some are new.
    fn the_scans_ledger_is_one_entry_with_its_folders(rules: PathRules) {
        let mut c = rooted(rules);
        walk(&mut c, r"D:\r", &[(r"D:\r\a", UNREADABLE), (r"D:\r\m", FolderFailure::MountRefused)], &[], true);
        let [entry] = c.ledger_entries(true).unwrap()[..] else { panic!("one entry") };
        assert_eq!(entry.kind, LedgerKind::ScanFolders);
        assert_eq!(entry.counts.map(|n| (n.total, n.new)), Some((2, 2)));
        assert_eq!(entry.notice(), Notice::ScanFoldersFailed { counts: None });
        let listed = c.failed_folders(10).unwrap();
        let seen: Vec<_> = listed.iter().map(|f| (f.path.clone(), f.failure, f.new)).collect();
        assert_eq!(
            seen,
            [(p(rules, r"D:\r\a"), UNREADABLE, true), (p(rules, r"D:\r\m"), FolderFailure::MountRefused, true)]
        );
        assert_eq!((&listed[0].root_path, listed[0].error.as_str()), (&p(rules, r"D:\r"), "access is denied"));
        assert!(c.mark_ledger_read(LedgerKind::ScanFolders, entry.newest, 1).unwrap());
        assert!(c.failed_folders(10).unwrap().iter().all(|f| !f.new));
        let again = [(r"D:\r\a", UNREADABLE), (r"D:\r\m", FolderFailure::MountRefused), (r"D:\r\n", UNREADABLE)];
        walk(&mut c, r"D:\r", &again, &[], true);
        let [entry] = c.ledger_entries(true).unwrap()[..] else { panic!("one entry") };
        assert!(!entry.read);
        assert_eq!(entry.notice(), Notice::ScanFoldersFailed { counts: Some(LedgerCounts { new: 1, earlier: 2 }) });
    }

    under_both_rules!(
        the_scans_ledger_is_one_entry_with_its_folders,
        the_scans_ledger_is_one_entry_with_its_folders_windows,
        the_scans_ledger_is_one_entry_with_its_folders_posix
    );

    // A walk holds its lease, in any connection, until it ends: a walk of the same folder, one above or one below (by
    // the rules: by case too under Windows') is refused with the holder's words and nothing written, a sibling's isn't.
    // It is given back when the lease is dropped, by the end of a complete walk, and by `release`; a heartbeat (a batch
    // or `renew`) keeps it, and one missing for `SCAN_LEASE_SECS` is a crashed walk's, taken over.
    fn a_walk_holds_its_folders_until_it_ends(rules: PathRules) {
        let dir = pg_testkit::run_folder("scratch_catalog", &format!("scan-lease-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        let mut a = Catalog::prepare(&path, versions(), rules).unwrap();
        let mut b = Catalog::open(&path, rules).unwrap();
        a.add_root(&p(rules, r"D:\r")).unwrap();
        let scans = |c: &Catalog| raw::rows_of(c, "SELECT count(*) FROM scans", [], |r| r.get::<_, i64>(0)).remove(0);
        // The holder's folder when `b`'s walk of `text` is refused; `None` when it may begin (it ends at once).
        let busy = |b: &mut Catalog, text: &str| -> Option<pg_model::rules::Path> {
            let before = scans(&*b);
            // The lease of a walk that may begin is dropped at once (given back).
            let began = b.begin_scan(&p(rules, text)).map(drop);
            match began {
                Err(Error::ScanBusy { root, holder }) => {
                    assert!(holder.starts_with("process "), "{holder}");
                    assert_eq!(scans(&*b), before, "a refused walk writes nothing");
                    Some(root)
                }
                Err(e) => panic!("{e}"),
                Ok(()) => None,
            }
        };
        {
            let _lease = a.begin_scan(&p(rules, r"D:\r\sub")).unwrap();
            assert_eq!(busy(&mut b, r"D:\r\sub"), Some(p(rules, r"D:\r\sub")));
            assert!(busy(&mut b, r"D:\r").is_some(), "a folder above");
            assert!(busy(&mut b, r"D:\r\sub\deep").is_some(), "a folder below");
            assert_eq!(busy(&mut b, r"D:\r\sub2"), None, "a sibling");
            let windows = rules == PathRules::Windows;
            assert_eq!(busy(&mut b, r"D:\r\SUB").is_some(), windows, "the same folder by the rules");
        }
        assert_eq!(busy(&mut b, r"D:\r"), None, "given back when dropped");
        let lease = a.begin_scan(&p(rules, r"D:\r")).unwrap();
        lease.mark_missing(&[]).unwrap();
        assert_eq!(busy(&mut b, r"D:\r"), None, "given back at the end of a complete walk");
        drop(lease);
        a.begin_scan(&p(rules, r"D:\r")).unwrap().release().unwrap();
        assert_eq!(busy(&mut b, r"D:\r"), None, "given back by release");
        // The heartbeat.
        let mut lease = a.begin_scan(&p(rules, r"D:\r")).unwrap();
        let stale = |c: &Catalog| {
            c.conn.execute("UPDATE scan_leases SET heartbeat = heartbeat - ?1", [SCAN_LEASE_SECS + 1]).unwrap();
        };
        stale(&b);
        lease.renew().unwrap();
        assert!(busy(&mut b, r"D:\r").is_some(), "renewed");
        stale(&b);
        let scan = lease.id();
        let tx = lease.tx().unwrap();
        drop(tx.scan_writer(scan, &p(rules, r"D:\r")).unwrap());
        tx.commit().unwrap();
        assert!(busy(&mut b, r"D:\r").is_some(), "renewed by a batch");
        stale(&b);
        assert_eq!(busy(&mut b, r"D:\r"), None, "expired: a crashed walk's");
        assert!(matches!(lease.renew(), Err(Error::LeaseLost { .. })), "the expired lease is gone");
        drop(lease);
        drop((a, b));
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        a_walk_holds_its_folders_until_it_ends,
        a_walk_holds_its_folders_until_it_ends_windows,
        a_walk_holds_its_folders_until_it_ends_posix
    );

    // A walk's batch changes a row only as the walk listed it: a write-back between the listing and the batch, or one
    // in progress, survives it (the row is only seen); an import, a retry or a new import version, which change nothing
    // on disk, don't keep a real change out.
    fn a_walk_changes_a_row_only_as_it_listed_it(rules: PathRules) {
        let mut c = rooted(rules);
        let root = p(rules, r"D:\r");
        let identity = FileIdentity::new(1, 7);
        let found = |c: &mut Catalog, scan: ScanId, size: u64, mtime: i64, prior: Option<Prior>| -> Recorded {
            let tx = c.tx().unwrap();
            let mut writer = tx.scan_writer(scan, &root).unwrap();
            let folder = writer.folder(&root).unwrap();
            let name = Name::from("a.jpg");
            let ext = Ext::of_name("a.jpg").unwrap();
            let file = ScannedFile { identity, name: &name, ext, size, mtime, created: None, prior };
            let recorded = writer.record_file(&folder, &file).unwrap();
            drop(writer);
            tx.commit().unwrap();
            recorded
        };
        let listed = |c: &Catalog| listed(c, &root, &[identity]).remove(0);
        let scan = begin(&mut c, &root);
        let prior = listed(&c);
        assert_eq!(found(&mut c, scan, 10, 100, prior), Recorded::Changed);
        let id = fixture::state::file_id(&c, "a.jpg").unwrap();
        // The folder says where to look first, not what is found: the row of a file listed in another folder (moved
        // there, or a second name of it), or in one the catalog has no row for, is the same; an identity with no row
        // has none.
        let new = p(rules, r"D:\r\new");
        let rows = c.folder_rows(&new).unwrap();
        let elsewhere = c.prior_states(&rows, &[identity, FileIdentity::new(1, 8)]).unwrap();
        assert_eq!(elsewhere, [Some(Prior { size: 10, mtime: 100 }), None]);
        // Unless that row changed after the folder's rows were read, its listing begun: a write-back through the row's
        // own name can have rewritten the file the folder listed. It has no row to compare with then, and is only seen.
        c.conn.execute("UPDATE files SET mtime = 101 WHERE id = ?1", [id.get()]).unwrap();
        assert_eq!(c.prior_states(&rows, &[identity]).unwrap(), [None]);
        assert_eq!(found(&mut c, scan, 10, 100, None), Recorded::Same);
        assert_eq!(raw::file_row(&c).2, 101, "the write-back's row is kept");
        // A row of the folder itself was read before the listing, so it is as it was then, whatever changed since.
        let rows = c.folder_rows(&root).unwrap();
        c.conn.execute("UPDATE files SET mtime = 100 WHERE id = ?1", [id.get()]).unwrap();
        assert_eq!(c.prior_states(&rows, &[identity]).unwrap(), [Some(Prior { size: 10, mtime: 101 })]);
        // Listed, then rewritten by the app before the batch: the row is the newer.
        let prior = listed(&c);
        assert_eq!(prior, Some(Prior { size: 10, mtime: 100 }));
        c.conn.execute("UPDATE files SET size = 12, mtime = 102 WHERE id = ?1", [id.get()]).unwrap();
        assert_eq!(found(&mut c, scan, 10, 100, prior), Recorded::Same);
        assert_eq!(raw::file_size(&c, id), 12);
        // Listed, then imported, retried and a new import version: none is a change on disk.
        let prior = listed(&c);
        fixture::state::stub_analysis_of_all(&c).unwrap();
        let failure =
            Failure { file_id: id, mtime: 102, size: 12, turns: Turns::NONE, error: "no".into(), retry: false };
        c.record_failures(FilePass::Thumbs, &[failure], 1).unwrap();
        c.retry_failures(FilePass::Thumbs).unwrap();
        raw::older_import_version(&c);
        raw::start_import_version(&c);
        assert_eq!(found(&mut c, scan, 20, 200, prior), Recorded::Changed);
        assert_eq!(raw::file_size(&c, id), 20);
        // A write into the file in progress: its temp is beside it, and the row is the writer's to change.
        let prior = listed(&c);
        fixture::state::queue_finished_copy(&c, id, "rating", 3, &p(rules, r"D:\r\a.jpg.tmp")).unwrap();
        assert_eq!(found(&mut c, scan, 30, 300, prior), Recorded::Same);
        assert_eq!(raw::file_size(&c, id), 20);
    }

    under_both_rules!(
        a_walk_changes_a_row_only_as_it_listed_it,
        a_walk_changes_a_row_only_as_it_listed_it_windows,
        a_walk_changes_a_row_only_as_it_listed_it_posix
    );

    // The end of a walk marks in batches of at most `WRITE_BATCH` files or folders, each a write transaction of its
    // own, so no other writer waits for the whole pass: 25 000 files, and 1 500 folders with nothing to mark.
    #[test]
    fn the_end_of_a_walk_writes_in_short_batches() {
        const FOLDERS: i64 = 1500;
        const FILES: i64 = 25_000;
        let rules = PathRules::Posix;
        let mut c = rooted(rules);
        fixture::state::add_scan(&c, 1, "/r").unwrap();
        for id in 1..=FOLDERS {
            fixture::state::add_folder(&c, id, &format!("/r/f{id}")).unwrap();
        }
        c.conn.execute("INSERT INTO volumes(id, serial) VALUES (1, 1)", []).unwrap();
        c.conn
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?1)
                 INSERT INTO files(id, volume, file_ref, folder_id, mid, name, ext, kind, size, mtime, missing, turns)
                 SELECT i, 1, i, i % ?2 + 1, (SELECT mid FROM folders WHERE id = i % ?2 + 1), CAST(i || '.jpg' AS BLOB),
                        ?3, ?4, 10, 100, 0, 0
                 FROM n",
                [FILES, FOLDERS, Ext::of("jpg").unwrap().code(), pg_model::media::Kind::Image.code()],
            )
            .unwrap();
        let root = p(rules, r"D:\r");
        let scan = begin(&mut c, &root);
        let holds = WriteHolds::watch();
        assert_eq!(lease(&mut c, scan, &root).mark_missing(&[]).unwrap(), 25_000);
        let held = holds.take();
        // About 1 000 files a batch (a folder's files are marked together): 25 batches, then the sweep.
        assert!((25..=27).contains(&held.len()), "{} holds", held.len());
        assert_eq!(raw::missing_count(&c), FILES);
        // Nothing left to mark: no batch of marks, so the write lock is taken only by the last transaction, which gives the
        // lease back (Phase 3: `mark_missing` reads first and writes only when a row is to be marked).
        let scan = begin(&mut c, &root);
        let holds = WriteHolds::watch();
        assert_eq!(lease(&mut c, scan, &root).mark_missing(&[]).unwrap(), 0);
        assert_eq!(holds.take().len(), 1);
    }

    // The folder's read recognises a listing that matches its row, which the walk then only marks seen; a listing that
    // differs in any compared column is for the batch.
    fn the_folder_read_recognises_a_listing_that_matches_its_row(rules: PathRules) {
        let mut c = rooted(rules);
        let root = p(rules, r"D:\r");
        let scan = begin(&mut c, &root);
        let name = Name::from("a.jpg");
        let ext = Ext::of_name("a.jpg").unwrap();
        let file = |size: u64| ScannedFile {
            identity: FileIdentity::new(1, 7),
            name: &name,
            ext,
            size,
            mtime: 100,
            created: None,
            prior: None,
        };
        let tx = c.tx().unwrap();
        let mut writer = tx.scan_writer(scan, &root).unwrap();
        let folder = writer.folder(&root).unwrap();
        writer.record_file(&folder, &file(10)).unwrap();
        drop(writer);
        tx.commit().unwrap();
        let rows = c.folder_rows(&root).unwrap();
        assert!(rows.unchanged(&file(10)).is_some(), "the listing matches the row");
        assert!(rows.unchanged(&file(11)).is_none(), "a size change is for the batch");
    }

    under_both_rules!(
        the_folder_read_recognises_a_listing_that_matches_its_row,
        the_folder_read_recognises_a_listing_that_matches_its_row_windows,
        the_folder_read_recognises_a_listing_that_matches_its_row_posix
    );

    // A walk's set of seen rows goes with its lease: giving the lease back (here, its drop) empties it.
    fn a_lease_given_back_empties_the_walks_set(rules: PathRules) {
        let mut c = rooted(rules);
        let root = p(rules, r"D:\r");
        let scan = begin(&mut c, &root);
        let seen =
            |c: &Catalog| -> i64 { c.conn.query_row("SELECT count(*) FROM temp.scan_seen", [], |r| r.get(0)).unwrap() };
        {
            let lease = ScanLease::new(&mut c, scan, root.clone());
            lease.mark_seen(&[(1, false), (2, true)]).unwrap();
        }
        assert_eq!(seen(&c), 0, "the set went with the lease");
    }

    under_both_rules!(
        a_lease_given_back_empties_the_walks_set,
        a_lease_given_back_empties_the_walks_set_windows,
        a_lease_given_back_empties_the_walks_set_posix
    );

    // A connection waits for a locked catalog only as long as its busy timeout, and the error says it was busy.
    #[test]
    fn a_busy_catalog_fails_after_the_busy_timeout_and_says_so() {
        let dir = pg_testkit::run_folder("scratch_catalog", "busy-timeout").unwrap();
        let path = dir.join("catalog.sqlite");
        let rules = pg_platform::native::PATH_RULES;
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        let holder = raw::on_disk(&path);
        holder.execute_batch("BEGIN IMMEDIATE").unwrap();
        c.set_busy_timeout(Duration::from_millis(100)).unwrap();
        let started = Instant::now();
        let err = c.add_root(&p(rules, r"D:\r")).unwrap_err();
        assert!(err.is_busy(), "{err:#}");
        // The error comes after the busy timeout has run. A lower bound (half the timeout) is what the test proves; an upper
        // bound would depend on the machine's load.
        assert!(started.elapsed() >= Duration::from_millis(50), "{:?}", started.elapsed());
        holder.execute_batch("ROLLBACK").unwrap();
        assert!(c.add_root(&p(rules, r"D:\r")).unwrap());
        drop((c, holder));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // `remove_root` over a folder of more files than one write batch: every row under it is deleted, the root and the
    // leases of the walks under it go, and the other library folder's files and its walk's lease stay.
    #[test]
    fn removing_a_root_deletes_every_row_of_several_batches_and_leaves_the_others_alone() {
        use crate::types::WRITE_BATCH;
        use pg_model::ids::{FileId, FolderId};
        let rules = PathRules::Posix;
        let mut c = rooted(rules);
        assert!(c.add_root(&p(rules, r"D:\s")).unwrap());
        crate::fixture::state::add_scan(&c, 1, &spelled(rules, "D:")).unwrap();
        for (id, path) in [(1, r"D:\r"), (2, r"D:\r\sub"), (3, r"D:\s")] {
            crate::fixture::state::add_folder(&c, id, &spelled(rules, path)).unwrap();
        }
        // 2 500 files under D:\r (in it and in D:\r\sub): three batches. 30 under D:\s.
        let under: i64 = 2_500;
        assert!(usize::try_from(under).unwrap() > WRITE_BATCH);
        let mut files: Vec<fixture::File> = (1..=under)
            .map(|id| {
                let folder = FolderId::new(if id % 5 == 0 { 2 } else { 1 });
                fixture::File::new(FileId::new(id), folder, "a.jpg", 10, 100)
            })
            .collect();
        files.extend(
            (under + 1..=under + 30).map(|id| fixture::File::new(FileId::new(id), FolderId::new(3), "b.jpg", 10, 100)),
        );
        fixture::insert(&c, &files).unwrap();
        let (r, sub, s) = (p(rules, r"D:\r"), p(rules, r"D:\r\sub"), p(rules, r"D:\s"));
        begin(&mut c, &r);
        begin(&mut c, &sub);
        begin(&mut c, &s);
        let root = c.root_id(&r).unwrap().unwrap();
        assert_eq!(c.remove_root(root).unwrap(), Some(2_500), "every file under the root is hidden");
        assert!(c.root_id(&r).unwrap().is_none(), "the root is gone");
        assert!(c.root_id(&s).unwrap().is_some(), "the other root stays");
        assert_eq!(raw::missing_count(&c), 0, "deleted: no hidden row of the removed folder is left");
        let rows: i64 = c.conn.query_row("SELECT count(*) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 30, "the other root's files are the only rows");
        let present: Vec<i64> =
            crate::fixture::state::present_files(&c).unwrap().iter().map(|(id, _)| id.get()).collect();
        assert_eq!(present.len(), 30, "the other root's files are as they were");
        assert!(present.iter().all(|&id| id > under), "{present:?}");
        let leases: i64 = c.conn.query_row("SELECT count(*) FROM scan_leases", [], |r| r.get(0)).unwrap();
        assert_eq!(leases, 1, "the walks of the removed folder and under it lost their leases; the other root's stays");
        assert!(matches!(c.begin_scan(&r).err(), Some(Error::RootRemoved { .. })), "no library folder holds it now");
        assert!(matches!(c.begin_scan(&s).err(), Some(Error::ScanBusy { .. })), "the other root's lease stays");
    }
}
