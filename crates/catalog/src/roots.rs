//! The library folders (`roots`) and their scans: which folders the library holds, a walk's start (with its lease) and
//! end, the files a finished walk didn't see, and the folder ledger's rows a walk records or sweeps. Methods of
//! `Catalog`.

use anyhow::anyhow;
use rusqlite::{Connection, OptionalExtension, params};

use pg_model::ids::{FileIdentity, RootId, ScanId};
use pg_model::rules::{Path as RulePath, PathRules};

use crate::core::{Catalog, ScanLease};
use crate::error::{Error, Result};
use crate::ledger::failure::tidy;
use crate::transaction::write_transaction;
use crate::types::{
    FOLDER_FAILED, FolderFailure, Prior, Root, SCAN_LEASE_SECS, Skip, SkipReason, WRITE_BATCH, covering_root,
    folders_under, roots_on, skipped_folders, under_params,
};

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

    /// Add a folder to the library. A folder already inside the library adds nothing (returns false); one that
    /// contains library folders replaces them (their notices and their folder ledger's rows go with them).
    pub fn add_root(&self, root: &RulePath) -> Result<bool> {
        // One transaction: the roots it replaces go only with the insert that replaces them (a crash between the two
        // would lose them), and the check reads what the replacement will see.
        let tx = write_transaction(&self.conn)?;
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

    /// Take a folder out of the library. Its files are marked missing, not deleted: adding it back (or the drive
    /// returning) brings them back with their analysis and ratings. Its notices and its folder ledger's rows go (their
    /// foreign key). Returns how many files it hid; `None`, with nothing changed, when `root` is not a library folder.
    pub fn remove_root(&self, root: RootId) -> Result<Option<usize>> {
        let tx = write_transaction(&self.conn)?;
        let path: Option<Vec<u8>> =
            tx.query_row("SELECT path FROM roots WHERE id = ?1", [root.get()], |r| r.get(0)).optional()?;
        let Some(path) = path else { return Ok(None) };
        let path = self.rules.parse_bytes(&path).ok_or_else(|| anyhow!("a stored library path has a NUL"))?;
        tx.execute("DELETE FROM roots WHERE id = ?1", [root.get()])?;
        let [p1, p2, p3] = under_params(self.rules, &path);
        let n = tx.execute(
            concat!("UPDATE files SET missing = 1 WHERE missing = 0 AND folder_id IN (", under_root!(), ")"),
            params![p1, p2, p3],
        )?;
        tx.commit()?;
        Ok(Some(n))
    }

    /// Begin a walk of `root`: its `scans` row (the id the walk stamps its writes with), and its lease, in one write
    /// transaction. Refused with [`Error::ScanBusy`], with nothing written, while another walk of a folder that overlaps
    /// `root` (either is inside the other, by the rules) holds an unexpired lease, in this process or another; a lease
    /// with no heartbeat for [`SCAN_LEASE_SECS`] is a crashed walk's, and goes. The walk reads and writes through the
    /// lease, which gives itself back when the walk ends however it ends ([`ScanLease`]).
    pub fn begin_scan(&mut self, root: &RulePath) -> Result<ScanLease<'_>> {
        let rules = self.rules;
        let bytes = rules.join(root);
        let tx = write_transaction(&self.conn)?;
        tx.execute("DELETE FROM scan_leases WHERE heartbeat < unixepoch() - ?1", [SCAN_LEASE_SECS])?;
        let held: Vec<(Vec<u8>, String)> = tx
            .prepare("SELECT root, holder FROM scan_leases ORDER BY id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (other, holder) in held {
            let other = rules.parse_bytes(&other).ok_or_else(|| anyhow!("a scan lease's folder has a NUL"))?;
            if rules.is_under(root, &other) || rules.is_under(&other, root) {
                return Err(Error::ScanBusy { root: rules.display(&other), holder });
            }
        }
        tx.execute("INSERT INTO scans(root, started) VALUES (?1, unixepoch())", [&bytes])?;
        let scan = ScanId::new(tx.last_insert_rowid());
        tx.execute(
            "INSERT INTO scan_leases(id, root, holder, heartbeat) VALUES (?1, ?2, ?3, unixepoch())",
            params![scan.get(), bytes, lease_holder()],
        )?;
        tx.commit()?;
        Ok(ScanLease::new(self, scan))
    }

    pub fn finish_scan(&self, scan: ScanId) -> Result<()> {
        self.conn.execute("UPDATE scans SET finished = unixepoch() WHERE id = ?1", [scan.get()])?;
        Ok(())
    }

    /// The end of a complete walk of `root` (`scan` its id). Files under `root` that it didn't see are marked missing;
    /// their rows stay (ratings, decisions). Files in `skipped` folders (and their subfolders) are left as they are: the
    /// walk didn't list them (it couldn't, they are mounts it refuses, or reparse folders it doesn't follow), so it saw
    /// nothing there to compare with. The marks are batches of at most `WRITE_BATCH` (1 000) folders or files, each in a short
    /// transaction of its own, so no other writer waits behind the whole pass.
    ///
    /// Then, in one last transaction, the folder ledger's rows under `root` that this walk didn't see (`walk` before
    /// it) are resolved and go, except those strictly below a folder it skipped as unreadable or a refused mount (it
    /// never looked there; a reparse folder spares nothing, since what is below it can't resolve while it is one), and
    /// the walk's lease is given back. The number of files marked.
    pub fn mark_missing(&self, root: &RulePath, scan: ScanId, skipped: &[Skip]) -> Result<usize> {
        let rules = self.rules;
        // Read once: the lease keeps any other walk of these folders out.
        let folders: Vec<i64> = {
            let snapshot = self.conn.unchecked_transaction()?;
            let skipped = skipped_folders(&snapshot, rules, skipped)?;
            let folders = folders_under(&snapshot, rules, root)?.into_iter().filter(|f| !skipped.contains(f)).collect();
            snapshot.commit()?;
            folders
        };
        let mut marked = 0;
        let mut pending = folders.as_slice();
        while !pending.is_empty() {
            let tx = write_transaction(&self.conn)?;
            let (mut done, mut rows) = (0, 0);
            {
                let mut mark = tx.prepare_cached(
                    "UPDATE files SET missing = 1 WHERE folder_id = ?1 AND missing = 0 AND last_scan < ?2",
                )?;
                for folder in pending {
                    rows += mark.execute(params![folder, scan.get()])?;
                    done += 1;
                    if done >= WRITE_BATCH || rows >= WRITE_BATCH {
                        break;
                    }
                }
            }
            tx.commit()?;
            marked += rows;
            pending = pending.get(done..).unwrap_or_default();
        }
        let tx = write_transaction(&self.conn)?;
        sweep_folder_failures(&tx, rules, root, scan, skipped)?;
        tx.execute("DELETE FROM scan_leases WHERE id = ?1", [scan.get()])?;
        tx.commit()?;
        Ok(marked)
    }

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
                FolderFailure::Refused.code(),
                tidy(error),
                walk
            ],
        )?;
        tx.commit()?;
        Ok(written > 0)
    }

    /// The catalog's rows for `identities` as they are now (their size and mtime; `None` for an identity with no row),
    /// read in one snapshot outside any transaction of a walk's: what a walk reads when it lists a folder, so that its
    /// batch, written later, changes only rows still so ([`crate::ScannedFile::prior`]).
    pub fn prior_states(&self, identities: &[FileIdentity]) -> Result<Vec<Option<Prior>>> {
        let snapshot = self.conn.unchecked_transaction()?;
        let mut out = Vec::with_capacity(identities.len());
        {
            let mut stmt =
                snapshot.prepare_cached("SELECT size, mtime FROM files WHERE volume = ?1 AND file_ref = ?2")?;
            for identity in identities {
                let (volume, file_ref) = (identity.volume.cast_signed(), identity.file_index.cast_signed());
                let prior = stmt
                    .query_row(params![volume, file_ref], |r| Ok(Prior { size: r.get(0)?, mtime: r.get(1)? }))
                    .optional()?;
                out.push(prior);
            }
        }
        snapshot.commit()?;
        Ok(out)
    }
}

/// Who holds a lease this process takes, for the words of another's refusal.
fn lease_holder() -> String {
    format!("process {}", std::process::id())
}

/// The folder ledger's sweep at the end of a complete walk of `root` (`scan`), on `conn` ([`Catalog::mark_missing`]).
fn sweep_folder_failures(
    conn: &Connection,
    rules: PathRules,
    root: &RulePath,
    scan: ScanId,
    skipped: &[Skip],
) -> Result<()> {
    let [p1, p2, p3] = under_params(rules, root);
    let rows: Vec<(i64, Vec<u8>)> = conn
        .prepare_cached(
            "SELECT id, path FROM folder_failures WHERE (path = ?1 OR (path >= ?2 AND path < ?3)) AND walk < ?4",
        )?
        .query_map(params![p1, p2, p3, scan.get()], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    // The subtrees the walk never looked into: a row strictly below one of them is not known to be resolved.
    let unseen: Vec<[Vec<u8>; 3]> = skipped
        .iter()
        .filter(|s| matches!(s.reason, SkipReason::Unreadable | SkipReason::RefusedMount))
        .map(|s| under_params(rules, &s.path))
        .collect();
    let mut delete = conn.prepare_cached("DELETE FROM folder_failures WHERE id = ?1")?;
    for (id, path) in rows {
        let below_unseen = unseen.iter().any(|[at, lo, hi]| path != *at && path >= *lo && path < *hi);
        if !below_unseen {
            delete.execute([id])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use crate::browse::fixtures::{spelled, under_both_rules};
    use crate::fixture::{self, WriteHolds, raw};
    use crate::ledger::Failure;
    use crate::test_support::{begin, p, versions};
    use crate::types::rules_name;
    use crate::{Catalog, Error, FolderFailure, Prior, Recorded, SCAN_LEASE_SECS, ScannedFile, Skip, SkipReason};
    use pg_model::ids::{FileIdentity, ScanId};
    use pg_model::media::Ext;
    use pg_model::name::Name;
    use pg_model::notice::{FilePass, LedgerCounts, LedgerKind, Notice};
    use pg_model::rules::PathRules;
    use pg_model::turns::Turns;

    const UNREADABLE: FolderFailure = FolderFailure::Unreadable;

    /// A catalog whose library is `D:\r`.
    fn library(rules: PathRules) -> Catalog {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        assert!(c.add_root(&p(rules, r"D:\r")).unwrap());
        c
    }

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
            lease.mark_missing(&root, scan, &skipped).unwrap();
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
        let mut c = library(rules);
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
        let mut c = library(rules);
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

    // A row is a folder by its exact path: two spellings are two rows, under either rules. A folder recorded again
    // keeps its serial (it is not news again), whatever its reason now.
    fn a_folder_is_its_exact_path_and_keeps_its_serial(rules: PathRules) {
        let mut c = library(rules);
        let both = [(r"D:\r\a", UNREADABLE), (r"D:\r\A", UNREADABLE)];
        walk(&mut c, r"D:\r", &both, &[], true);
        let first = ledger(&c);
        assert_eq!(first.len(), 2, "{first:?}");
        walk(&mut c, r"D:\r", &both, &[], true);
        assert_eq!(ledger(&c), first, "the same rows, the same serials");
        walk(&mut c, r"D:\r", &[(r"D:\r\a", FolderFailure::MountRefused)], &[], true);
        let lower = first.iter().find(|row| row.0 == spelled(rules, r"D:\r\a")).unwrap();
        assert_eq!(ledger(&c), [(lower.0.clone(), "mount-refused".to_string(), lower.2)]);
    }

    under_both_rules!(
        a_folder_is_its_exact_path_and_keeps_its_serial,
        a_folder_is_its_exact_path_and_keeps_its_serial_windows,
        a_folder_is_its_exact_path_and_keeps_its_serial_posix
    );

    // Only a complete walk resolves anything. Below a folder it couldn't read the rows stay (it never looked), below a
    // reparse folder they go (nothing below one can resolve while it is one), and a reparse folder is no row. The rows
    // go with their library folder: removed, or replaced by one above it.
    fn the_ledger_follows_complete_walks_and_the_library(rules: PathRules) {
        let mut c = library(rules);
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

    // Nothing is recorded for a folder no library folder holds, nor once its library folder is removed mid-walk.
    #[test]
    fn a_folder_outside_the_library_is_not_recorded() {
        let rules = PathRules::Posix;
        let mut c = library(rules);
        let elsewhere = p(rules, r"D:\elsewhere");
        let mut lease = c.begin_scan(&elsewhere).unwrap();
        let scan = lease.id();
        let tx = lease.tx().unwrap();
        let mut writer = tx.scan_writer(scan, &elsewhere).unwrap();
        assert!(!writer.folder_failed(&p(rules, r"D:\elsewhere\a"), UNREADABLE, "denied").unwrap());
        drop(writer);
        tx.commit().unwrap();
        assert!(!lease.folder_refused(&elsewhere, "not NTFS").unwrap());
        drop(lease);
        // Removed between two batches of a walk: the next batch records nothing.
        let root = p(rules, r"D:\r");
        let mut lease = c.begin_scan(&root).unwrap();
        let scan = lease.id();
        let id = lease.root_id(&root).unwrap().unwrap();
        lease.remove_root(id).unwrap();
        let tx = lease.tx().unwrap();
        let mut writer = tx.scan_writer(scan, &root).unwrap();
        assert!(!writer.folder_failed(&p(rules, r"D:\r\a"), UNREADABLE, "denied").unwrap());
        drop(writer);
        tx.commit().unwrap();
        assert!(paths(&lease).is_empty());
    }

    // A folder refused before its walk began is a row of the ledger (one, however often refused); a walk already
    // running doesn't resolve it, the next complete walk does.
    fn a_refused_folder_is_resolved_by_its_next_complete_walk(rules: PathRules) {
        let mut c = library(rules);
        let root = p(rules, r"D:\r");
        let lease = c.begin_scan(&root).unwrap();
        let running = lease.id();
        assert!(lease.folder_refused(&root, "not an NTFS volume").unwrap());
        assert!(lease.folder_refused(&root, "not an NTFS volume").unwrap());
        let rows = ledger(&lease);
        assert_eq!(
            rows.iter().map(|r| (r.0.as_str(), r.1.as_str())).collect::<Vec<_>>(),
            [(spelled(rules, r"D:\r").as_str(), "refused")]
        );
        lease.mark_missing(&root, running, &[]).unwrap();
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
        let mut c = library(rules);
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
        let dir = pg_testkit::test_folder("scratch_catalog", &format!("scan-lease-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        let mut a = Catalog::prepare(&path, versions(), rules).unwrap();
        let mut b = Catalog::open(&path, rules).unwrap();
        a.add_root(&p(rules, r"D:\r")).unwrap();
        let scans = |c: &Catalog| raw::rows_of(c, "SELECT count(*) FROM scans", [], |r| r.get::<_, i64>(0)).remove(0);
        // The holder's folder when `b`'s walk of `text` is refused; `None` when it may begin (it ends at once).
        let busy = |b: &mut Catalog, text: &str| -> Option<String> {
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
            assert_eq!(busy(&mut b, r"D:\r\sub"), Some(rules.display(&p(rules, r"D:\r\sub"))));
            assert!(busy(&mut b, r"D:\r").is_some(), "a folder above");
            assert!(busy(&mut b, r"D:\r\sub\deep").is_some(), "a folder below");
            assert_eq!(busy(&mut b, r"D:\r\sub2"), None, "a sibling");
            let windows = rules == PathRules::Windows;
            assert_eq!(busy(&mut b, r"D:\r\SUB").is_some(), windows, "the same folder by the rules");
        }
        assert_eq!(busy(&mut b, r"D:\r"), None, "given back when dropped");
        let lease = a.begin_scan(&p(rules, r"D:\r")).unwrap();
        lease.mark_missing(&p(rules, r"D:\r"), lease.id(), &[]).unwrap();
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
        assert!(lease.renew().unwrap());
        assert!(busy(&mut b, r"D:\r").is_some(), "renewed");
        stale(&b);
        let scan = lease.id();
        let tx = lease.tx().unwrap();
        drop(tx.scan_writer(scan, &p(rules, r"D:\r")).unwrap());
        tx.commit().unwrap();
        assert!(busy(&mut b, r"D:\r").is_some(), "renewed by a batch");
        stale(&b);
        assert_eq!(busy(&mut b, r"D:\r"), None, "expired: a crashed walk's");
        assert!(!lease.renew().unwrap(), "the expired lease is gone");
        drop(lease);
        drop((a, b));
        std::fs::remove_dir_all(&dir).unwrap();
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
        let mut c = library(rules);
        let root = p(rules, r"D:\r");
        let identity = FileIdentity::new(1, 7);
        let found = |c: &mut Catalog, scan: ScanId, size: u64, mtime: i64, prior: Option<Prior>| -> Recorded {
            let tx = c.tx().unwrap();
            let mut writer = tx.scan_writer(scan, &root).unwrap();
            let folder = writer.folder(&root).unwrap();
            let name = Name::from("a.jpg");
            let ext = Ext::of_name("a.jpg").unwrap();
            let file = ScannedFile { identity, name: &name, ext, size, mtime, created: None, prior };
            let recorded = writer.upsert_file(&folder, &file).unwrap();
            drop(writer);
            tx.commit().unwrap();
            recorded
        };
        let listed = |c: &Catalog| c.prior_states(&[identity]).unwrap().remove(0);
        let scan = begin(&mut c, &root);
        let prior = listed(&c);
        assert_eq!(found(&mut c, scan, 10, 100, prior), Recorded::Changed);
        let id = fixture::state::file_id(&c, "a.jpg").unwrap();
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
            Failure { file_id: id, mtime: 102, size: 12, rotation: Turns::NONE, error: "no".into(), retry: false };
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
        let mut c = library(rules);
        fixture::state::add_scan(&c, 1, "/r").unwrap();
        for id in 1..=FOLDERS {
            fixture::state::add_folder(&c, id, &format!("/r/f{id}")).unwrap();
        }
        c.conn
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?1)
                 INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, last_scan, missing,
                                   rotation, order_key)
                 SELECT i, 1, i, i % ?2 + 1, CAST(i || '.jpg' AS BLOB), 'jpg', 'image', 10, 100, 1, 0, 0,
                        CAST(printf('%08d', i) AS BLOB)
                 FROM n",
                [FILES, FOLDERS],
            )
            .unwrap();
        let root = p(rules, r"D:\r");
        let scan = begin(&mut c, &root);
        let holds = WriteHolds::watch();
        assert_eq!(c.mark_missing(&root, scan, &[]).unwrap(), 25_000);
        let held = holds.take();
        // About 1 000 files a batch (a folder's files are marked together): 25 batches, then the sweep.
        assert!((25..=27).contains(&held.len()), "{} holds", held.len());
        assert_eq!(raw::missing_count(&c), FILES);
        // Nothing left to mark: 1 000 folders a batch, then the sweep.
        let scan = begin(&mut c, &root);
        let holds = WriteHolds::watch();
        assert_eq!(c.mark_missing(&root, scan, &[]).unwrap(), 0);
        assert_eq!(holds.take().len(), 3);
    }

    // A connection waits for a locked catalog only as long as its busy timeout, and the error says it was busy.
    #[test]
    fn a_busy_catalog_fails_after_the_busy_timeout_and_says_so() {
        let dir = pg_testkit::test_folder("scratch_catalog", "busy-timeout").unwrap();
        let path = dir.join("catalog.sqlite");
        let rules = pg_platform::native::PATH_RULES;
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        let holder = raw::on_disk(&path);
        holder.execute_batch("BEGIN IMMEDIATE").unwrap();
        c.set_busy_timeout(Duration::from_millis(100)).unwrap();
        let started = Instant::now();
        let err = c.add_root(&p(rules, r"D:\r")).unwrap_err();
        assert!(err.is_busy(), "{err:#}");
        assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());
        holder.execute_batch("ROLLBACK").unwrap();
        assert!(c.add_root(&p(rules, r"D:\r")).unwrap());
        drop((c, holder));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
