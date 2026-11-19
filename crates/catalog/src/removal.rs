//! Removing a folder from the library for good: its files go from the catalog, with their analyses, ratings and queued
//! writes, and its folders, and the walks' rows under it. A folder that is missing (an offline drive, a folder a walk no
//! longer sees) is not removed: its files stay, marked missing, as before.
//!
//! The removal runs in steps, each idempotent, and the marker `root_removals` says it is not finished:
//!
//! - **A** [`Catalog::begin_removal`]: one transaction takes the library folder and its leases away and makes the marker.
//!   From here the folder is not a library folder, and no walk of it can write.
//! - **B** [`Catalog::hide_pages`]: its files are marked missing, in batches, so the grid and every pass stop seeing them
//!   before anything is deleted.
//! - **C** [`Catalog::drive_removals`]: its files are deleted in batches of `WRITE_BATCH`, each batch in one short
//!   transaction (the cascades go with it: analyses, ratings, pairs, failures). A file with a recorded write (`file_writes.
//!   temp`, the only whole copy of an edit) is kept, and so is its folder: the write finishes it, or the user discards it
//!   ([`Catalog::discard_removal`]).
//! - **D** (the end of C): the folders with no file left go, the walks' `scans` rows under the folder go, and the marker
//!   goes, in one transaction; while files are kept, the marker records how many.
//! - **E** [`Catalog::shrink`]: the freed pages are given back to the file system.
//!
//! A crash after any step leaves the marker, and the next drive finishes the rest. The thumbnails are the caller's: the
//! `before_page` callback of [`Catalog::drive_removals`] forgets each batch's, before the batch is deleted.

use std::rc::Rc;

use anyhow::anyhow;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use pg_model::ids::{FileId, RootId};
use pg_model::rules::{Path as RulePath, PathRules};

use crate::core::Catalog;
use crate::error::{Error, Result};
use crate::transaction::write_transaction;
use crate::types::{WRITE_BATCH, roots_on};

/// What [`Catalog::begin_removal`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Begun {
    /// The folder left the library and its marker is made: the removal runs from here.
    Started(RulePath),
    /// A removal of this folder is already under way: the marker is there, and the folder is not in the library.
    Pending(RulePath),
    /// No library folder has that id.
    NotFound,
    /// The id names a library folder, but not the one the caller showed: the list changed since. Nothing was changed.
    Changed,
}

/// What a drive of the removals did: the files deleted, the files kept for an unfinished write, and whether the drive
/// was stopped before the end (the removal goes on at the next drive).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Driven {
    pub files: usize,
    pub deferred: usize,
    pub stopped: bool,
}

/// A removal that is not finished: its folder, and the files kept for an unfinished write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub path: RulePath,
    pub deferred: usize,
}

/// What [`Catalog::discard_removal`] deleted: the rows, and the temp files their writes had recorded, as the rules' bytes.
/// The temps are the only copies of the edits they held; the caller unlinks them and says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discarded {
    pub rows: usize,
    pub temps: Vec<Vec<u8>>,
}

/// `SELECT` of the files a batch deletes: those under the folders, without a recorded write.
const PAGE: &str = "SELECT id FROM files WHERE folder_id IN rarray(?1) AND NOT EXISTS \
                    (SELECT 1 FROM file_writes w WHERE w.file_id = files.id AND w.temp IS NOT NULL) LIMIT ";

/// The batch's files, deleted if they are still under the folders and still have no recorded write (a write recorded
/// since the page was read keeps its file: its temp is the only copy of the edit).
const DELETE_FILES: &str = "DELETE FROM files WHERE id IN rarray(?1) AND folder_id IN rarray(?2) AND NOT EXISTS \
                            (SELECT 1 FROM file_writes w WHERE w.file_id = files.id AND w.temp IS NOT NULL)";

impl Catalog {
    /// Step A: takes the library folder `root` out of the library, with its leases, and makes its marker, in one
    /// transaction. `expected` is the folder the caller showed (as the rules have it, not as text): when the id names
    /// another folder now, nothing changes ([`Begun::Changed`]). The folder's files are still there, and visible, until
    /// [`Catalog::hide_pages`] runs.
    pub fn begin_removal(&self, root: RootId, expected: &RulePath) -> Result<Begun> {
        let rules = self.rules;
        let tx = write_transaction(&self.conn)?;
        let stored: Option<Vec<u8>> =
            tx.query_row("SELECT path FROM roots WHERE id = ?1", [root.get()], |r| r.get(0)).optional()?;
        let Some(bytes) = stored else { return Ok(Begun::NotFound) };
        let path = rules.parse_bytes(&bytes).ok_or_else(|| anyhow!("a stored library path has a NUL"))?;
        if !rules.same(&path, expected) {
            return Ok(Begun::Changed);
        }
        if marker_present(&tx, &bytes)? {
            return Ok(Begun::Pending(path));
        }
        tx.execute("DELETE FROM roots WHERE id = ?1", [root.get()])?;
        release_leases(&tx, rules, &path)?;
        tx.execute("INSERT INTO root_removals(path, started) VALUES (?1, unixepoch())", [&bytes])?;
        tx.commit()?;
        Ok(Begun::Started(path))
    }

    /// Step B: hides the files under `path`, in batches. Idempotent: a batch that is hidden already changes nothing, so
    /// the resume of a removal can run it again. Returns the files hidden by this call.
    pub fn hide_pages(&self, path: &RulePath) -> Result<usize> {
        let folders = folder_ids_under(&self.conn, self.rules, path)?;
        let ids: Vec<i64> = {
            let mut stmt = self.conn.prepare("SELECT id FROM files WHERE missing = 0 AND folder_id IN rarray(?1)")?;
            let rows = stmt.query_map([Rc::new(values(&folders))], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut hidden = 0;
        for batch in ids.chunks(WRITE_BATCH) {
            let tx = write_transaction(&self.conn)?;
            {
                let mut hide = tx.prepare_cached("UPDATE files SET missing = 1 WHERE id = ?1 AND missing = 0")?;
                for &id in batch {
                    hidden += hide.execute([id])?;
                }
            }
            tx.commit()?;
        }
        Ok(hidden)
    }

    /// Steps C and D for every marker, as far as they go. `before_page` is called with each batch's file ids before the
    /// batch is deleted (the caller forgets their thumbnails there); it returns `false` to stop: the drive then ends with
    /// [`Driven::stopped`], and the marker stays for the next drive. A marker is dropped without a deletion when a library
    /// folder that overlaps it is found (an older build added one meanwhile): the rows stay hidden, and that folder's walk
    /// takes them back.
    pub fn drive_removals(&self, before_page: &mut dyn FnMut(&[i64]) -> bool) -> Result<Driven> {
        let mut driven = Driven::default();
        for marker in self.removal_markers()? {
            if !drive_one(self, &marker.path, before_page, &mut driven)? {
                driven.stopped = true;
                break;
            }
        }
        Ok(driven)
    }

    /// The removals not finished, with the files each keeps.
    pub fn removal_markers(&self) -> Result<Vec<Marker>> {
        let rules = self.rules;
        let rows: Vec<(Vec<u8>, i64)> = self
            .conn
            .prepare("SELECT path, deferred FROM root_removals ORDER BY id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut markers = Vec::with_capacity(rows.len());
        for (bytes, deferred) in rows {
            let path =
                rules.parse_bytes(&bytes).ok_or_else(|| Error::Other(anyhow!("a removal's folder has a NUL")))?;
            markers.push(Marker { path, deferred: usize::try_from(deferred).unwrap_or(0) });
        }
        Ok(markers)
    }

    /// The files under the removals not finished (kept ones included): the total of their progress.
    pub fn removal_files(&self) -> Result<usize> {
        let mut folders = Vec::new();
        for marker in self.removal_markers()? {
            folders.extend(folder_ids_under(&self.conn, self.rules, &marker.path)?);
        }
        if folders.is_empty() {
            return Ok(0);
        }
        let count: i64 = self.conn.query_row(
            "SELECT count(*) FROM files WHERE folder_id IN rarray(?1)",
            [Rc::new(values(&folders))],
            |r| r.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Whether `file` is under a removal not finished: its files are going, and no write of the app's may change them.
    pub fn under_marker(&self, file: FileId) -> Result<bool> {
        let folder: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT f.path FROM files x JOIN folders f ON f.id = x.folder_id WHERE x.id = ?1",
                [file.get()],
                |r| r.get(0),
            )
            .optional()?;
        let Some(folder) = folder else { return Ok(false) };
        let folder = self.rules.parse_bytes(&folder).ok_or_else(|| anyhow!("a stored folder path has a NUL"))?;
        Ok(self.removal_markers()?.iter().any(|m| self.rules.is_under(&folder, &m.path)))
    }

    /// Gives up the removal of `path` that waits for writes: its remaining rows go with their recorded writes, and the
    /// folders and the marker with them. Refused when a library folder overlaps it. `None` when no removal of it is
    /// pending. The caller unlinks the returned temps (see [`Discarded`]).
    pub fn discard_removal(&self, path: &RulePath) -> Result<Option<Discarded>> {
        let rules = self.rules;
        let bytes = rules.join(path);
        let tx = write_transaction(&self.conn)?;
        if !marker_present(&tx, &bytes)? {
            return Ok(None);
        }
        if overlapping_root(&tx, rules, path)?.is_some() {
            return Err(Error::Other(anyhow!(
                "{} is a library folder again: its removal is not discarded",
                rules.display(path)
            )));
        }
        let folders = folder_ids_under(&tx, rules, path)?;
        let temps: Vec<Vec<u8>> = {
            let mut stmt = tx.prepare(
                "SELECT w.temp FROM file_writes w JOIN files f ON f.id = w.file_id \
                 WHERE w.temp IS NOT NULL AND f.folder_id IN rarray(?1)",
            )?;
            let rows = stmt.query_map([Rc::new(values(&folders))], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let rows = tx.execute("DELETE FROM files WHERE folder_id IN rarray(?1)", [Rc::new(values(&folders))])?;
        finish_folders(&tx, rules, path, &folders)?;
        tx.commit()?;
        Ok(Some(Discarded { rows, temps }))
    }

    /// Gives the free pages of the catalog file back to the file system (step E). Only with incremental auto-vacuum,
    /// which a catalog has once its conversion has run (`vacuum_if_owed`): without it there is nothing to return here.
    /// Each step is a short write of its own, and the file is then truncated by a checkpoint; a checkpoint that a reader
    /// holds off falls back to a passive one.
    pub fn shrink(&self) -> Result<()> {
        let mode: i64 = self.conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
        if mode != 2 {
            return Ok(());
        }
        let mut free: i64 = self.conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
        while free > 0 {
            self.conn.execute_batch("PRAGMA incremental_vacuum(4096)")?;
            let now: i64 = self.conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
            if now >= free {
                break;
            }
            free = now;
        }
        let busy: i64 = self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        if busy != 0 {
            self.conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE)")?;
        }
        Ok(())
    }
}

/// The removal of one marker's folder, steps C and D. Returns `false` when `before_page` stopped the drive. A free
/// function rather than a method of `Catalog`: the method's body would make `core` depend on this module's `Driven`.
fn drive_one(
    catalog: &Catalog,
    path: &RulePath,
    before_page: &mut dyn FnMut(&[i64]) -> bool,
    driven: &mut Driven,
) -> Result<bool> {
    let rules = catalog.rules;
    let bytes = rules.join(path);
    let folders = folder_ids_under(&catalog.conn, rules, path)?;
    let page = format!("{PAGE}{WRITE_BATCH}");
    loop {
        let ids: Vec<i64> = {
            let mut stmt = catalog.conn.prepare_cached(&page)?;
            let rows = stmt.query_map([Rc::new(values(&folders))], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        if ids.is_empty() {
            break;
        }
        if !before_page(&ids) {
            return Ok(false);
        }
        let tx = write_transaction(&catalog.conn)?;
        if !marker_present(&tx, &bytes)? {
            // Finished or discarded by another drive: nothing is left to do here.
            return Ok(true);
        }
        if overlapping_root(&tx, rules, path)?.is_some() {
            tx.execute("DELETE FROM root_removals WHERE path = ?1", [&bytes])?;
            tx.commit()?;
            return Ok(true);
        }
        let deleted =
            tx.prepare_cached(DELETE_FILES)?.execute(params![Rc::new(values(&ids)), Rc::new(values(&folders))])?;
        tx.commit()?;
        driven.files += deleted;
    }
    // Step D: the folders that hold no file, the walks under the folder, and the marker, once nothing is kept.
    let tx = write_transaction(&catalog.conn)?;
    if !marker_present(&tx, &bytes)? {
        return Ok(true);
    }
    let kept: i64 =
        tx.query_row("SELECT count(*) FROM files WHERE folder_id IN rarray(?1)", [Rc::new(values(&folders))], |r| {
            r.get(0)
        })?;
    driven.deferred = usize::try_from(kept).unwrap_or(0);
    if kept == 0 {
        finish_folders(&tx, rules, path, &folders)?;
    } else {
        // Folders that hold no file go even while files are kept.
        delete_empty_folders(&tx, &folders)?;
        tx.execute("UPDATE root_removals SET deferred = ?2 WHERE path = ?1", params![bytes, kept])?;
    }
    tx.commit()?;
    Ok(true)
}

/// The ids of the folders at `path` or below it, by the rules. Folders are matched by the rules, not by the bytes of
/// `path`: under the Windows rules a folder spelled in another case is under the library folder all the same.
pub(crate) fn folder_ids_under(conn: &Connection, rules: PathRules, path: &RulePath) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached("SELECT id, path FROM folders")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
    let mut ids = Vec::new();
    for row in rows {
        let (id, bytes) = row?;
        let folder = rules.parse_bytes(&bytes).ok_or_else(|| anyhow!("a stored folder path has a NUL"))?;
        if rules.is_under(&folder, path) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// The paths of the removals not finished, as the rules have them, on `conn`.
pub(crate) fn marker_paths(conn: &Connection, rules: PathRules) -> Result<Vec<RulePath>> {
    let rows: Vec<Vec<u8>> = conn
        .prepare("SELECT path FROM root_removals ORDER BY id")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut paths = Vec::with_capacity(rows.len());
    for bytes in &rows {
        paths.push(rules.parse_bytes(bytes).ok_or_else(|| Error::Other(anyhow!("a removal's folder has a NUL")))?);
    }
    Ok(paths)
}

/// A removal not finished that overlaps `path` (equal, inside or containing), if there is one: a library folder can't be
/// added there until the removal is finished (`Catalog::add_root`).
pub(crate) fn overlapping_marker(conn: &Connection, rules: PathRules, path: &RulePath) -> Result<Option<RulePath>> {
    Ok(marker_paths(conn, rules)?
        .into_iter()
        .find(|m| rules.same(m, path) || rules.is_under(m, path) || rules.is_under(path, m)))
}

/// The library folder that overlaps `path` (equal, inside or containing), on `conn`.
fn overlapping_root(conn: &Transaction<'_>, rules: PathRules, path: &RulePath) -> Result<Option<RulePath>> {
    Ok(roots_on(conn, rules)?
        .into_iter()
        .map(|r| r.path)
        .find(|r| rules.same(r, path) || rules.is_under(r, path) || rules.is_under(path, r)))
}

fn marker_present(conn: &Connection, bytes: &[u8]) -> Result<bool> {
    let count: i64 = conn.query_row("SELECT count(*) FROM root_removals WHERE path = ?1", [bytes], |r| r.get(0))?;
    Ok(count > 0)
}

/// Deletes the leases of the walks under `path`, as `begin_scan` matches them (by the rules).
fn release_leases(tx: &Transaction<'_>, rules: PathRules, path: &RulePath) -> Result<()> {
    let leases: Vec<(i64, Vec<u8>)> = tx
        .prepare("SELECT id, root FROM scan_leases")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, walked) in leases {
        let walked = rules.parse_bytes(&walked).ok_or_else(|| anyhow!("a scan lease's folder has a NUL"))?;
        if rules.is_under(&walked, path) {
            tx.execute("DELETE FROM scan_leases WHERE id = ?1", [id])?;
        }
    }
    Ok(())
}

/// Step D's end: the folders under `path` with no file go, the walks' rows under it go, and the marker goes.
fn finish_folders(tx: &Transaction<'_>, rules: PathRules, path: &RulePath, folders: &[i64]) -> Result<()> {
    delete_empty_folders(tx, folders)?;
    let scans: Vec<(i64, Vec<u8>)> = tx
        .prepare("SELECT id, root FROM scans")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, walked) in scans {
        let walked = rules.parse_bytes(&walked).ok_or_else(|| anyhow!("a scan's folder has a NUL"))?;
        if rules.is_under(&walked, path) {
            tx.execute("DELETE FROM scans WHERE id = ?1", [id])?;
        }
    }
    tx.execute("DELETE FROM root_removals WHERE path = ?1", [rules.join(path)])?;
    Ok(())
}

/// The folders in `folders` that no file is in, deleted.
fn delete_empty_folders(tx: &Transaction<'_>, folders: &[i64]) -> Result<()> {
    tx.execute(
        "DELETE FROM folders WHERE id IN rarray(?1) AND NOT EXISTS (SELECT 1 FROM files WHERE folder_id = folders.id)",
        [Rc::new(values(folders))],
    )?;
    Ok(())
}

/// The ids as the `rarray` parameter takes them.
fn values(ids: &[i64]) -> Vec<Value> {
    ids.iter().copied().map(Value::Integer).collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use pg_model::ids::FolderId;

    use super::*;
    use crate::browse::fixtures::{spelled, under_both_rules};
    use crate::fixture::{self, File};
    use crate::test_support::{p, versions};

    fn library(rules: PathRules) -> Catalog {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        assert!(c.add_root(&p(rules, r"D:\r")).unwrap());
        crate::fixture::state::add_folder(&c, 1, &spelled(rules, r"D:\r")).unwrap();
        crate::fixture::state::add_folder(&c, 2, &spelled(rules, r"D:\r\sub")).unwrap();
        fixture::insert(
            &c,
            &[
                File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100),
                File::new(FileId::new(2), FolderId::new(2), "b.jpg", 10, 100),
            ],
        )
        .unwrap();
        c.conn.execute("INSERT INTO ratings(file_id, rating, updated) VALUES (1, 3, 0)", []).unwrap();
        c
    }

    fn rows(c: &Catalog, table: &str) -> i64 {
        c.conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap()
    }

    fn begin(c: &Catalog, rules: PathRules) {
        let root = c.root_id(&p(rules, r"D:\r")).unwrap().unwrap();
        assert!(matches!(c.begin_removal(root, &p(rules, r"D:\r")).unwrap(), Begun::Started(_)));
    }

    /// The whole of a removal: its files, their children (the rating goes with its file), its folders and its marker.
    fn a_removed_folder_is_deleted_with_its_children(rules: PathRules) {
        let c = library(rules);
        begin(&c, rules);
        c.hide_pages(&p(rules, r"D:\r")).unwrap();
        let driven = c.drive_removals(&mut |_| true).unwrap();
        assert_eq!(driven.files, 2);
        assert_eq!(driven.deferred, 0);
        assert_eq!((rows(&c, "files"), rows(&c, "ratings"), rows(&c, "folders")), (0, 0, 0));
        assert!(c.removal_markers().unwrap().is_empty());
        assert!(c.root_id(&p(rules, r"D:\r")).unwrap().is_none());
    }

    under_both_rules!(
        a_removed_folder_is_deleted_with_its_children,
        a_removed_folder_is_deleted_with_its_children_windows,
        a_removed_folder_is_deleted_with_its_children_posix
    );

    /// A removal begun but not yet hidden (the process ended between the two) keeps its rows: they are hidden, and then
    /// deleted, by the next run of the removal.
    fn a_removal_begun_and_not_hidden_keeps_its_rows_until_the_next_run(rules: PathRules) {
        let c = library(rules);
        begin(&c, rules);
        assert_eq!(rows(&c, "files"), 2, "nothing is deleted before the rows are hidden");
        assert_eq!(c.removal_markers().unwrap().len(), 1);
        c.hide_pages(&p(rules, r"D:\r")).unwrap();
        assert_eq!(c.drive_removals(&mut |_| true).unwrap().files, 2);
        assert_eq!(rows(&c, "files"), 0);
    }

    under_both_rules!(
        a_removal_begun_and_not_hidden_keeps_its_rows_until_the_next_run,
        a_removal_begun_and_not_hidden_keeps_its_rows_until_the_next_run_windows,
        a_removal_begun_and_not_hidden_keeps_its_rows_until_the_next_run_posix
    );

    /// While a removal is pending its folder can't be added again (the rows would be the removed ones, still going).
    fn add_is_refused_over_a_pending_removal(rules: PathRules) {
        let c = library(rules);
        begin(&c, rules);
        for folder in [r"D:\r", r"D:\r\sub"] {
            let refused = c.add_root(&p(rules, folder));
            assert!(matches!(refused, Err(Error::RemovalPending { .. })), "{folder}: {refused:?}");
        }
    }

    under_both_rules!(
        add_is_refused_over_a_pending_removal,
        add_is_refused_over_a_pending_removal_windows,
        add_is_refused_over_a_pending_removal_posix
    );

    /// A file with a change still to write keeps its row and its folder (the change's temp is the only copy); the other
    /// rows go. Discarding the removal gives the row up and returns its temp for the caller to unlink.
    fn a_deferred_row_is_kept_until_discarded_and_its_temp_is_returned(rules: PathRules) {
        let c = library(rules);
        let temp = spelled(rules, r"D:\r\.a.jpg.rating.pg-write");
        c.conn
            .execute(
                "INSERT INTO file_writes(file_id, kind, value, queued, temp, temp_value) \
                 VALUES (1, 'rating', 3, 0, ?1, 3)",
                [rules.join(&p(rules, &temp))],
            )
            .unwrap();
        begin(&c, rules);
        c.hide_pages(&p(rules, r"D:\r")).unwrap();
        let driven = c.drive_removals(&mut |_| true).unwrap();
        assert_eq!((driven.files, driven.deferred), (1, 1));
        assert_eq!(rows(&c, "files"), 1, "the deferred row stays");
        assert_eq!(c.removal_markers().unwrap().len(), 1, "and so does its marker");

        let discarded = c.discard_removal(&p(rules, r"D:\r")).unwrap().expect("a removal is pending");
        assert_eq!(discarded.rows, 1);
        assert_eq!(discarded.temps.len(), 1);
        assert_eq!((rows(&c, "files"), rows(&c, "folders")), (0, 0));
        assert!(c.removal_markers().unwrap().is_empty());
        assert!(c.discard_removal(&p(rules, r"D:\r")).unwrap().is_none(), "nothing is left to discard");
    }

    under_both_rules!(
        a_deferred_row_is_kept_until_discarded_and_its_temp_is_returned,
        a_deferred_row_is_kept_until_discarded_and_its_temp_is_returned_windows,
        a_deferred_row_is_kept_until_discarded_and_its_temp_is_returned_posix
    );
}
