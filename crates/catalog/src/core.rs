//! The catalog's lifecycle: `Catalog` prepared, opened and closed, its transaction `Tx` and the writers it hands out,
//! and a walk's lease (`ScanLease`, which `Catalog::begin_scan` takes). Uses `lifecycle`, `rebuild`, `writers`,
//! `schema`, `transaction`, `types` and `error`. Every other concern's methods are in its own module's `impl Catalog`
//! (`roots`, `lookup`, `work`, `borders`, `browse`, `queue`, `dups`, `pair`, `inbox`, `ledger`); this calls none of
//! them.

use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, anyhow};
use rusqlite::{Connection, OpenFlags};

use pg_model::ids::ScanId;
use pg_model::rules::{Path as RulePath, PathRules};

use crate::error::{Error, Result};
use crate::lifecycle::{
    check_rules, in_use, mark_unheld_missing, set_borders_version, set_import_version, set_taken_zone,
    set_thumbs_version, take_lock, take_shared, try_exclusive,
};
use crate::rebuild::{back_up, db_file, rebuild};
use crate::schema::{
    BASELINE, MIGRATIONS, SETTINGS, bring_to, current_expected, current_version, expected, mismatch, version_of,
};
use crate::transaction::{WriteTx, write_transaction};
use crate::types::{Rebuilt, Versions};
use crate::writers::{AnalysisWriter, BordersWriter, LedgerWriter, ScanWriter};

pub struct Catalog {
    pub(crate) conn: Connection,
    /// The path rules the catalog's paths are stored under (`meta.path_rules`, checked when it opens).
    pub(crate) rules: PathRules,
    /// The catalog this build moved aside to make this one, when [`Catalog::prepare`] made one.
    rebuilt: Option<Rebuilt>,
    /// The shared hold on the file (`pg_dbfile::Live`) for as long as the catalog is open: a rebuild elsewhere waits
    /// for it to go. `None` for `:memory:`. Declared after `conn`, so it is released after the connection closes.
    _live: Option<pg_dbfile::Live>,
}

impl Catalog {
    /// A number that changes when another connection (in this process or another) commits a change to the catalog:
    /// what tells a poll that something it reads may have changed. Read outside a transaction.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA data_version", [], |r| r.get(0))?)
    }

    /// Merge the WAL into the file, then close the connection (on drop, too, without the merge). The last connection
    /// to a file to close deletes its WAL; the merge keeps the file whole if the process ends before the others close.
    pub fn close(self) -> Result<()> {
        self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }

    /// Prepare the catalog at `path` for this build, creating it when new. A file that is not this build's catalog is
    /// judged before anything is written to it: its schema (every object, and the keys its creation seeds) must be the
    /// one this build makes for its version. One that isn't (an earlier layout, another build's, or not a database at
    /// all) is rebuilt: a new catalog in this build's layout is made beside it, the old rows are copied into it with
    /// their ids (`carry_rows`), and the old file is moved aside (`pg_dbfile::move_aside`; see [`Catalog::rebuilt`]). A
    /// row the copy can't take (a column the new layout needs that the old one lacks) fails the copy, and the new catalog
    /// is made empty, with the reason said. A catalog of this build's schema at an earlier version is migrated forward.
    /// Its version marks are set to `versions`. The library calls this once, before anything else opens the catalog. A
    /// change to the file (a rebuild, or a migration) needs no other process or connection to have it open: otherwise
    /// it fails with the catalog untouched, and the one that has it open is left as it is.
    pub fn prepare(path: &Path, versions: Versions, rules: PathRules) -> Result<Catalog> {
        let memory = path == Path::new(":memory:");
        let _lock = if memory { None } else { Some(take_lock(path)?) };
        let current = current_version()?;
        let mut conn = Connection::open(db_file(path)?).with_context(|| format!("catalog {}", path.display()))?;
        let version = version_of(&conn)?;
        let why = match version {
            None => Some("not a database".to_string()),
            Some(found) if found > current => {
                return Err(Error::Other(anyhow!(
                    "catalog {}: schema version {found}, which this build doesn't know (it has {current}): made by \
                     another build",
                    path.display()
                )));
            }
            Some(found) => mismatch(&conn, found, &expected(found)?)?,
        };
        // A catalog of this build's schema whose paths were stored under other rules is refused before anything is
        // written to it: its migrations would convert its rows under these rules (a rebuild checks them itself).
        if why.is_none() && version.is_some_and(|found| found > 0) {
            check_rules(&conn, path, rules)?;
        }
        // The lock order of a change: the exclusive hold first, kept through the change. Without it a change is refused
        // before anything is moved; a catalog that needs no change is held shared, below, as any open catalog is.
        let changes = why.is_some() || version.is_some_and(|found| found != current);
        let exclusive = if memory { None } else { try_exclusive(path)? };
        if changes && !memory && exclusive.is_none() {
            return Err(in_use(path));
        }
        let mut rebuilt = None;
        if let Some(why) = why {
            rebuilt = Some(rebuild(conn, path, rules, why, version.is_some())?);
            conn = Connection::open(db_file(path)?).with_context(|| format!("catalog {}", path.display()))?;
        }
        conn.execute_batch(SETTINGS)?;
        // A catalog of this build's schema at an earlier version is copied aside before it is migrated (a rebuilt one
        // was: the old file is kept as it was).
        if let Some(found) = version.filter(|&found| rebuilt.is_none() && found > 0 && found < current && !memory) {
            back_up(&conn, path, found)?;
        }
        bring_to(&conn, BASELINE, MIGRATIONS, rules, current).with_context(|| format!("catalog {}", path.display()))?;
        if let Some(why) = mismatch(&conn, current, current_expected()?)? {
            return Err(Error::Other(anyhow!(
                "catalog {}: the catalog made is not this build's: {why}",
                path.display()
            )));
        }
        check_rules(&conn, path, rules)?;
        mark_unheld_missing(&conn).with_context(|| format!("catalog {}", path.display()))?;
        set_import_version(&conn, versions.import).with_context(|| format!("catalog {}", path.display()))?;
        set_borders_version(&conn, versions.borders).with_context(|| format!("catalog {}", path.display()))?;
        set_thumbs_version(&conn, versions.thumbs).with_context(|| format!("catalog {}", path.display()))?;
        set_taken_zone(&conn).with_context(|| format!("catalog {}", path.display()))?;
        pg_query::register_functions(&conn)?;
        // The change is made: the catalog's own shared hold takes the exclusive one's place (no other holder can have
        // come in meanwhile, since the preparer's lock is still held).
        drop(exclusive);
        let live = if memory { None } else { Some(take_shared(path)?) };
        Ok(Catalog { conn, rules, rebuilt, _live: live })
    }

    /// Open a catalog this process has prepared ([`Catalog::prepare`], which makes it, brings it to this build's schema
    /// and sets the build's version marks): the settings and the SQL functions. Every connection to it (the app's
    /// threads, the background job, the writer) runs this, so the check is the schema's itself, against the cached
    /// expected schema, and is not repeated for the marks. An error if the catalog isn't there, or its schema isn't this
    /// build's (another program changed it since it was prepared).
    pub fn open(path: &Path, rules: PathRules) -> Result<Catalog> {
        // Held before the file is looked at: a rebuild in progress is waited for, not read half-made.
        // A catalog that isn't there is the open's error below, and no marker is made for it.
        let live = if path == Path::new(":memory:") || !path.exists() { None } else { Some(take_shared(path)?) };
        let flags = OpenFlags::default() - OpenFlags::SQLITE_OPEN_CREATE;
        let conn = Connection::open_with_flags(db_file(path)?, flags)
            .with_context(|| format!("catalog {}", path.display()))?;
        conn.execute_batch(SETTINGS)?;
        let current = current_version()?;
        let version = pg_dbfile::user_version(&conn)?;
        if version != current {
            return Err(Error::Other(anyhow!(
                "catalog {}: schema version {version}, not this build's {current}",
                path.display()
            )));
        }
        if let Some(why) = mismatch(&conn, version, current_expected()?)? {
            return Err(Error::Other(anyhow!("catalog {}: {why}: it changed after it was prepared", path.display())));
        }
        check_rules(&conn, path, rules)?;
        pg_query::register_functions(&conn)?;
        Ok(Catalog { conn, rules, rebuilt: None, _live: live })
    }

    /// The path rules the catalog's paths are stored under.
    pub fn rules(&self) -> PathRules {
        self.rules
    }

    /// The catalog this build moved aside to make this one, when [`Catalog::prepare`] did; `None` when the catalog was
    /// kept (or opened with [`Catalog::open`]).
    pub fn rebuilt(&self) -> Option<&Rebuilt> {
        self.rebuilt.as_ref()
    }

    /// The id made with the catalog (`meta.catalog_id`).
    pub fn catalog_id(&self) -> Result<String> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = 'catalog_id'", [], |r| r.get(0))?)
    }

    /// Stop the statements on this connection while `cancel` is set (from any thread): each fails with an error for
    /// which [`Error::Interrupted`] is returned, including those started after it was set, until it is cleared. Unlike
    /// SQLite's interrupt, which only reaches the statements running at that moment, it can't fall between two
    /// statements of a read.
    pub fn cancel_when(&mut self, cancel: Arc<AtomicBool>) -> Result<()> {
        // Checked every 1000 virtual-machine steps: microseconds.
        Ok(self.conn.progress_handler(1000, Some(move || cancel.load(Ordering::Relaxed)))?)
    }

    /// Give back this connection's page cache (the reader does it after each list it made).
    pub fn release_memory(&self) -> Result<()> {
        Ok(self.conn.release_memory()?)
    }

    /// How long a statement on this connection waits for a lock another connection holds before it fails with
    /// `SQLITE_BUSY` ([`Error::is_busy`]). Every connection starts at 5 s (`SETTINGS`); a writer that retries on its
    /// own, and checks for a stop between tries, sets a shorter one.
    pub fn set_busy_timeout(&self, timeout: Duration) -> Result<()> {
        Ok(self.conn.busy_timeout(timeout)?)
    }
}

/// A transaction on the catalog. It hands out the writers of the scan, the import, the borders and the failure
/// ledger, so a batch of their writes commits together, with `commit`.
pub struct Tx<'c> {
    inner: WriteTx<'c>,
    rules: PathRules,
}

impl Catalog {
    /// Begin a transaction on the catalog (see [`Tx`]).
    pub fn tx(&mut self) -> Result<Tx<'_>> {
        Ok(Tx { inner: write_transaction(&self.conn)?, rules: self.rules })
    }
}

impl Tx<'_> {
    /// The writer of the scan `scan` of `root`, in this transaction. Making it renews the walk's lease (the heartbeat
    /// of each batch the walk writes), and resolves the library folder that holds `root`, once, for the folder ledger.
    pub fn scan_writer(&self, scan: ScanId, root: &RulePath) -> Result<ScanWriter<'_>> {
        ScanWriter::new(&self.inner, scan, self.rules, root)
    }

    /// The writer of the failure ledger, in this transaction: what a batch failed on is stored with what it made, or
    /// neither.
    pub fn ledger_writer(&self) -> LedgerWriter<'_> {
        LedgerWriter::new(&self.inner)
    }

    /// The writer of the analyses, in this transaction. `import` is the build's IMPORT_VERSION (`Versions::import`),
    /// which the failures it stores are made against.
    pub fn analysis_writer(&self, import: i64) -> Result<AnalysisWriter<'_>> {
        AnalysisWriter::new(&self.inner, import)
    }

    /// The writer of the borders, in this transaction.
    pub fn borders_writer(&self) -> Result<BordersWriter<'_>> {
        BordersWriter::new(&self.inner)
    }

    /// Commit what was written through this transaction's writers.
    pub fn commit(self) -> Result<()> {
        Ok(self.inner.commit()?)
    }
}

/// A walk's lease of the folder it walks, which [`Catalog::begin_scan`] takes: while it holds, no other walk of a
/// folder that overlaps it begins (one is inside the other), in this process or another, so two walks never interleave
/// their writes. The walk renews it with each batch it writes (`Tx::scan_writer`, or [`ScanLease::renew`] while it
/// waits); a lease with no heartbeat for `SCAN_LEASE_SECS` is taken to be a crashed walk's, and expires.
///
/// It is released at the end of a complete walk, in `Catalog::mark_missing`'s last transaction, or by
/// [`ScanLease::release`], and in any case when it is dropped (a stop, an error or a panic leaves through here). It
/// borrows the catalog and is used as it: the walk reads and writes through the lease.
pub struct ScanLease<'c> {
    catalog: &'c mut Catalog,
    scan: ScanId,
}

impl<'c> ScanLease<'c> {
    pub(crate) fn new(catalog: &'c mut Catalog, scan: ScanId) -> ScanLease<'c> {
        ScanLease { catalog, scan }
    }

    /// The walk's `scans` row, which its writes are stamped with.
    pub fn id(&self) -> ScanId {
        self.scan
    }

    /// The walk is alive: its heartbeat is now. One short write, outside any transaction of the walk's. False when the
    /// lease is gone (released, or expired and taken by another walk): the walk can no longer tell that it is alone.
    pub fn renew(&self) -> Result<bool> {
        let renewed = self
            .catalog
            .conn
            .prepare_cached("UPDATE scan_leases SET heartbeat = unixepoch() WHERE id = ?1")?
            .execute([self.scan.get()])?;
        Ok(renewed > 0)
    }

    /// Give the lease back now, with the error if it can't be (dropping it gives it back too, but says nothing).
    pub fn release(self) -> Result<()> {
        release_lease(&self.catalog.conn, self.scan)
    }
}

impl Deref for ScanLease<'_> {
    type Target = Catalog;

    fn deref(&self) -> &Catalog {
        &*self.catalog
    }
}

impl DerefMut for ScanLease<'_> {
    fn deref_mut(&mut self) -> &mut Catalog {
        &mut *self.catalog
    }
}

impl Drop for ScanLease<'_> {
    /// A lease that can't be given back now (the catalog locked past the busy timeout) expires in `SCAN_LEASE_SECS`.
    fn drop(&mut self) {
        let _ = release_lease(&self.catalog.conn, self.scan);
    }
}

/// Give back the lease of walk `scan`, on `conn`; nothing is written when it is gone already (a complete walk gave it
/// back in its last transaction).
pub(crate) fn release_lease(conn: &Connection, scan: ScanId) -> Result<()> {
    let held: bool = conn
        .prepare_cached("SELECT EXISTS(SELECT 1 FROM scan_leases WHERE id = ?1)")?
        .query_row([scan.get()], |r| r.get(0))?;
    if held {
        conn.prepare_cached("DELETE FROM scan_leases WHERE id = ?1")?.execute([scan.get()])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Catalog;
    use crate::browse::fixtures::{spelled, under_both_rules};
    use crate::fixture::{self, raw};
    use crate::test_support::*;
    use crate::{Error, ScannedFile, queue};
    use pg_model::ids::{FileId, FileIdentity, FolderId};
    use pg_model::media::{Ext, Kind};
    use pg_model::name::Name;
    use pg_model::rules::PathRules;
    use pg_model::turns::Turns;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn roots_never_nest(rules: PathRules) {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        assert!(c.add_root(&p(rules, r"D:\a\b")).unwrap());
        assert!(c.add_root(&p(rules, r"D:\a\bc")).unwrap());
        // The same folder spelled another way: on Windows another drive letter and case, which the root covers. POSIX
        // names are case-sensitive, so there the spelling is the same path.
        let same_folder = match rules {
            PathRules::Windows => r"d:\A\b\c",
            PathRules::Posix => "/a/b/c",
        };
        assert!(!c.add_root(&p(rules, same_folder)).unwrap(), "inside an existing root");
        assert!(c.add_root(&p(rules, r"D:\a")).unwrap(), "replaces both");
        let paths =
            |c: &Catalog| c.roots().unwrap().into_iter().map(|r| c.rules().display(&r.path)).collect::<Vec<_>>();
        assert_eq!(paths(&c), [spelled(rules, r"D:\a")]);
        let id = c.root_id(&p(rules, r"D:\a")).unwrap().expect("a library folder has an id");
        assert_eq!(c.remove_root(id).unwrap(), Some(0));
        assert!(paths(&c).is_empty());
        assert_eq!(c.remove_root(id).unwrap(), None, "not a library folder any more");
    }

    under_both_rules!(roots_never_nest, roots_never_nest_windows, roots_never_nest_posix);

    fn cancelled_until_cleared(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        c.cancel_when(cancel.clone()).unwrap();
        let run = || raw::slow_count(&c).map_err(Error::from);
        assert_eq!(run().unwrap(), 100_000);
        // Set between statements (SQLite's own interrupt would be lost there): every statement fails.
        cancel.store(true, Ordering::Relaxed);
        assert!(matches!(run().unwrap_err(), Error::Interrupted));
        assert!(matches!(run().unwrap_err(), Error::Interrupted));
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(run().unwrap(), 100_000);
    }

    under_both_rules!(cancelled_until_cleared, cancelled_until_cleared_windows, cancelled_until_cleared_posix);

    fn pending_follows_what_the_analysis_was_made_from(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let scan = begin(&mut c, &p(rules, r"D:\x"));
        // New: pending.
        scanned(&mut c, scan, "a.jpg", 10, 100);
        let first = next_work(&c);
        let id = first.id;
        assert!(due(&c, id));
        // Analyzed as it is: done. A failure counts as done; only --retry redoes it.
        analyzed(&mut c, &first);
        assert!(!due(&c, id));
        assert!(c.pending_import(false).unwrap().is_empty());
        assert_eq!(c.pending_import(true).unwrap().len(), 1);
        // A rescan that changes nothing changes nothing, for the import or the thumbnails.
        let seq = raw::file_seq(&c, id);
        scanned(&mut c, scan, "a.jpg", 10, 100);
        assert_eq!(state(&c, id), (false, seq));
        // Changed while it was being analyzed: what's stored is of the old file.
        let old = c.pending_import(true).unwrap().items.remove(0);
        scanned(&mut c, scan, "a.jpg", 11, 101);
        analyzed(&mut c, &old);
        assert!(due(&c, id));
        let current = next_work(&c);
        analyzed(&mut c, &current);
        assert!(!due(&c, id));
        // Turned in the catalog: analyzed again with the turn, and the thumbnails' source moved on.
        turn_kept(&c, id, 1);
        let (pending, after) = state(&c, id);
        assert!(pending && after > seq);
        let turned = next_work(&c);
        assert_eq!(turned.rotation, Turns::new(1));
        analyzed(&mut c, &turned);
        // Rewritten by the app: a metadata write carries the analysis along; a pixel write leaves it stale.
        let rewritten = |c: &Catalog, before, after, metadata_only| {
            // A write that leaves what the file holds as it was (no turn), so only its size and mtime change.
            let write = pg_model::write::Write::Turns(Turns::NONE);
            let job = queue::Job { file_id: id, write, temp: None, attempts: 0 };
            let rewrite = queue::Rewrite { before: Some(before), after, metadata_only };
            c.write_done(&job, queue::WriteEffect::TurnedBack { rewrite, turns: Turns::NONE }).unwrap();
        };
        rewritten(&c, (11, 101), (12, 102), Some(pg_model::analysis::FileHash([0; 32])));
        assert!(!due(&c, id));
        rewritten(&c, (12, 102), (13, 103), None);
        assert!(due(&c, id));
        let rewritten = next_work(&c);
        analyzed(&mut c, &rewritten);
        // Renamed to another kind: analyzed again as that.
        scanned(&mut c, scan, "a.mp4", 13, 103);
        let renamed = next_work(&c);
        assert_eq!(renamed.kind, Kind::Video);
        analyzed(&mut c, &renamed);
        assert!(!due(&c, id));
        // Gone, then back: nothing to import, but the thumbnails look at it both times.
        let seq = raw::file_seq(&c, id);
        let next = begin(&mut c, &p(rules, r"D:\x"));
        c.mark_missing(&p(rules, r"D:\x"), next, &[]).unwrap();
        let gone = raw::file_seq(&c, id);
        scanned(&mut c, next, "a.mp4", 13, 103);
        assert_eq!(c.pending_import(false).unwrap().len(), 0);
        assert!(seq < gone && gone < raw::file_seq(&c, id));
    }

    under_both_rules!(
        pending_follows_what_the_analysis_was_made_from,
        pending_follows_what_the_analysis_was_made_from_windows,
        pending_follows_what_the_analysis_was_made_from_posix
    );

    fn a_folder_the_scan_could_not_list_keeps_its_files(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let put = |c: &mut Catalog, scan: pg_model::ids::ScanId, folder: &str, name: &str, file_ref: u64| {
            let identity = FileIdentity::new(1, file_ref);
            let prior = c.prior_states(&p(rules, folder), &[identity]).unwrap().remove(0);
            let tx = c.tx().unwrap();
            let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
            let folder = writer.folder(&p(rules, folder)).unwrap();
            let ext = Ext::of_name(name).unwrap();
            let file = ScannedFile { identity, name: &Name::from(name), ext, size: 1, mtime: 1, created: None, prior };
            writer.upsert_file(&folder, &file).unwrap();
            drop(writer);
            tx.commit().unwrap();
        };
        let first = begin(&mut c, &p(rules, r"D:\x"));
        put(&mut c, first, r"D:\x\ok", "a.jpg", 1);
        put(&mut c, first, r"D:\x\locked", "b.jpg", 2);
        put(&mut c, first, r"D:\x\locked\deep", "c.jpg", 3);

        // The next scan lists only D:\x\ok: D:\x\locked couldn't be read, so its files weren't seen, and aren't missing.
        let next = begin(&mut c, &p(rules, r"D:\x"));
        put(&mut c, next, r"D:\x\ok", "a.jpg", 1);
        let locked = crate::Skip { path: p(rules, r"D:\x\locked"), reason: crate::SkipReason::Unreadable };
        assert_eq!(c.mark_missing(&p(rules, r"D:\x"), next, &[locked]).unwrap(), 0);
        assert_eq!(raw::missing_count(&c), 0);

        // Once it can be listed again, a scan that doesn't see its files marks them.
        let later = begin(&mut c, &p(rules, r"D:\x"));
        put(&mut c, later, r"D:\x\ok", "a.jpg", 1);
        assert_eq!(c.mark_missing(&p(rules, r"D:\x"), later, &[]).unwrap(), 2);
        assert_eq!(raw::missing_count(&c), 2);
    }

    under_both_rules!(
        a_folder_the_scan_could_not_list_keeps_its_files,
        a_folder_the_scan_could_not_list_keeps_its_files_windows,
        a_folder_the_scan_could_not_list_keeps_its_files_posix
    );

    fn files_marked_are_imported_again_once(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        crate::fixture::state::add_scan(&c, 1, &spelled(rules, "D:")).unwrap();
        crate::fixture::state::add_folder(&c, 1, &spelled(rules, r"D:\x")).unwrap();
        crate::fixture::state::add_folder(&c, 2, &spelled(rules, r"D:\x\y")).unwrap();
        crate::fixture::state::add_folder(&c, 3, &spelled(rules, r"D:\xy")).unwrap();
        crate::fixture::state::add_folder(&c, 4, &spelled(rules, r"D:\x\y\z")).unwrap();
        let files = [(1, 1, "a.jpg"), (2, 2, "b.jpg"), (3, 3, "c.jpg"), (4, 4, "d.jpg"), (5, 4, "e.jpg")];
        let files =
            files.map(|(id, folder, name)| fixture::File::new(FileId::new(id), FolderId::new(folder), name, 10, 100));
        fixture::insert(&c, &files).unwrap();
        for item in c.pending_import(false).unwrap().items {
            analyzed(&mut c, &item);
        }
        // Analyzed, then gone.
        raw::set_missing(&c, FileId::new(5));
        assert!(c.pending_import(false).unwrap().is_empty());

        // A folder holds the files of its subfolders, not those of a folder named like it; missing ones aren't there.
        let mut under = c.files_under(&p(rules, r"D:\x\y")).unwrap();
        under.sort();
        assert_eq!(under, [FileId::new(2), FileId::new(4)]);
        assert_eq!(c.files_under(&p(rules, r"D:\x")).unwrap().len(), 3);
        assert!(c.files_under(&p(rules, r"D:\q")).unwrap().is_empty());

        // Marked: due, whatever their analysis, the missing one left; the change number doesn't move.
        let seq = raw::file_seq(&c, FileId::new(2));
        assert_eq!(c.mark_pending(&[FileId::new(2), FileId::new(4), FileId::new(5)]).unwrap(), 2);
        assert_eq!(state(&c, FileId::new(2)), (true, seq));
        assert!(due(&c, FileId::new(4)) && !due(&c, FileId::new(5)) && !due(&c, FileId::new(1)));
        let work = c.pending_import(false).unwrap();
        assert_eq!(work.items.iter().map(|i| i.id).collect::<Vec<_>>(), [FileId::new(2), FileId::new(4)]);
        // Analyzed again, as they are: done.
        for item in work.items {
            analyzed(&mut c, &item);
        }
        assert!(c.pending_import(false).unwrap().is_empty());
    }

    under_both_rules!(
        files_marked_are_imported_again_once,
        files_marked_are_imported_again_once_windows,
        files_marked_are_imported_again_once_posix
    );
}
