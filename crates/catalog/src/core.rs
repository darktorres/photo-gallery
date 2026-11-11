//! The catalog: `Catalog`, its transaction `Tx`, and their methods. Uses `lifecycle`, `writers`, `schema`, `types` and
//! `error`. `browse`, `writes`, `dups`, `pair` and `inbox` call it; it calls none of them.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, anyhow};
use rusqlite::types::Type;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};

use pg_model::format::Format;
use pg_model::ids::{FileId, RootId, ScanId};
use pg_model::media::Kind;
use pg_model::name::Name;
use pg_model::orientation::Orientation;
use pg_model::path::{PathArena, WorkList};
use pg_model::rules::{Path as RulePath, PathRules};

use crate::error::{Error, Result};
use crate::lifecycle::{
    back_up, check_rules, db_file, in_use, mark_unheld_missing, pending_import_sql, rebuild, set_borders_version,
    set_import_version, set_taken_zone, set_thumbs_version, take_lock, take_shared, try_exclusive, work_list,
};
use crate::schema::{
    BASELINE, MIGRATIONS, SETTINGS, bring_to, current_expected, current_version, expected, mismatch, version_of,
    write_transaction,
};
use crate::types::{
    CropSource, Rebuilt, Root, Status, Versions, WorkItem, WriteTarget, folders_under, skipped_folders, under_params,
};
use crate::writers::{AnalysisWriter, BordersWriter, ScanWriter};

pub struct Catalog {
    pub(crate) conn: Connection,
    /// The path rules the catalog's paths are stored under (`meta.path_rules`, checked when it opens).
    pub(crate) rules: PathRules,
    /// The catalog this build moved aside to make this one, when [`Catalog::prepare`] made one.
    rebuilt: Option<Rebuilt>,
    /// The shared hold on the file (`pg_schema::Live`) for as long as the catalog is open: a rebuild elsewhere waits
    /// for it to go. `None` for `:memory:`. Declared after `conn`, so it is released after the connection closes.
    _live: Option<pg_schema::Live>,
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
    /// their ids (`carry_rows`), and the old file is moved aside (`pg_schema::move_aside`; see [`Catalog::rebuilt`]). A
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
        let version = pg_schema::user_version(&conn)?;
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

    /// Whether the file can't be written: it is missing from the catalog, or no longer listed.
    pub fn file_gone(&self, file_id: FileId) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT coalesce((SELECT missing FROM files WHERE id = ?1), 1)",
            [file_id.get()],
            |r| r.get(0),
        )?)
    }

    /// The folder the file is in, as the catalog has it; `None` if there is no such file. A failed query is an error.
    pub fn file_folder(&self, file_id: FileId) -> Result<Option<RulePath>> {
        let bytes: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT fo.path FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1",
                [file_id.get()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(bytes.and_then(|bytes| self.rules.parse_bytes(&bytes)))
    }

    /// The folder the file is in while the library shows it (present); `None` for a file gone, or missing.
    pub fn present_file_folder(&self, file_id: FileId) -> Result<Option<RulePath>> {
        let bytes: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT fo.path FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1 AND f.missing = 0",
                [file_id.get()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(bytes.and_then(|bytes| self.rules.parse_bytes(&bytes)))
    }

    /// What a queued write needs of its file: its path, its analyzed format, and its kind, each read at the row (a
    /// format or a kind that doesn't read is a damaged row, and an error).
    pub fn write_target(&self, file_id: FileId) -> Result<WriteTarget> {
        let (folder, name, format, kind): (Vec<u8>, Vec<u8>, Option<Format>, Kind) = self.conn.query_row(
            "SELECT fo.path, f.name, a.format, f.kind FROM files f JOIN folders fo ON fo.id = f.folder_id
             LEFT JOIN analysis a ON a.file_id = f.id WHERE f.id = ?1",
            [file_id.get()],
            |r| Ok((r.get(0)?, r.get(1)?, crate::types::format_at(r, 2)?, crate::types::kind_at(r, 3)?)),
        )?;
        Ok(WriteTarget { path: self.file_path_of(&folder, &name)?, format, kind })
    }

    /// A file's path from its folder's stored bytes and its name's: the folder, the separator, the name.
    pub(crate) fn file_path_of(&self, folder: &[u8], name: &[u8]) -> Result<RulePath> {
        let mut parts = self.rules.parse_bytes(folder).ok_or_else(|| anyhow!("a stored folder path has a NUL"))?;
        parts.parts.push(Name::from_raw(name));
        Ok(parts)
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

    pub fn begin_scan(&self, root: &RulePath) -> Result<ScanId> {
        self.conn.execute("INSERT INTO scans(root, started) VALUES (?1, unixepoch())", [self.rules.join(root)])?;
        Ok(ScanId::new(self.conn.last_insert_rowid()))
    }

    pub fn finish_scan(&self, scan: ScanId) -> Result<()> {
        self.conn.execute("UPDATE scans SET finished = unixepoch() WHERE id = ?1", [scan.get()])?;
        Ok(())
    }

    /// Mark files under `root` that this scan didn't see as missing. Their rows stay (ratings, decisions). Files in
    /// `skipped` folders (and their subfolders) are left as they are: the scan didn't list them (it couldn't, or they
    /// are reparse folders it doesn't follow), so it saw nothing there to compare with.
    pub fn mark_missing(&self, root: &RulePath, scan: ScanId, skipped: &[RulePath]) -> Result<usize> {
        // One transaction: all of the folders are marked, or none, and the commit is one for the whole pass.
        let tx = write_transaction(&self.conn)?;
        let skipped = skipped_folders(&tx, self.rules, skipped)?;
        let mut marked = 0;
        {
            let mut mark =
                tx.prepare("UPDATE files SET missing = 1 WHERE folder_id = ?1 AND missing = 0 AND last_scan < ?2")?;
            for folder in folders_under(&tx, self.rules, root)? {
                if !skipped.contains(&folder) {
                    marked += mark.execute(params![folder, scan.get()])?;
                }
            }
        }
        tx.commit()?;
        Ok(marked)
    }

    /// The library folders, by path, with their ids.
    pub fn roots(&self) -> Result<Vec<Root>> {
        self.roots_on(&self.conn)
    }

    /// [`Catalog::roots`] on `conn`, which may be a transaction's.
    pub(crate) fn roots_on(&self, conn: &Connection) -> Result<Vec<Root>> {
        let mut stmt = conn.prepare("SELECT id, path FROM roots ORDER BY path")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
        let mut roots = Vec::new();
        for row in rows {
            let (id, bytes) = row?;
            let path = self.rules.parse_bytes(&bytes).ok_or_else(|| anyhow!("a stored library path has a NUL"))?;
            roots.push(Root { id: RootId::new(id), path });
        }
        Ok(roots)
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
    /// contains library folders replaces them.
    pub fn add_root(&self, root: &RulePath) -> Result<bool> {
        // One transaction: the roots it replaces go only with the insert that replaces them (a crash between the two
        // would lose them), and the check reads what the replacement will see.
        let tx = write_transaction(&self.conn)?;
        let roots = self.roots_on(&tx)?;
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
    /// returning) brings them back with their analysis and ratings. Returns how many files it hid; `None`, with
    /// nothing changed, when `root` is not a library folder.
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

    /// Files whose analysis is absent or stale (`files.pending`), and with `retry_errors` those that failed, in
    /// on-disk order (volume, then MFT record number), which reads ~3× faster than path order on an HDD (FINDINGS
    /// "I/O"). Without `retry_errors` it's read in that order from `files_to_import`, so nothing pending is one probe.
    pub fn pending_import(&self, retry_errors: bool) -> Result<WorkList<WorkItem>> {
        // Counted in the same snapshot: an upper bound (a failure may also be pending), each count read from its
        // index alone.
        let tx = self.conn.unchecked_transaction()?;
        let count = |sql: &str| -> Result<usize> { Ok(tx.query_row(sql, [], |r| r.get(0))?) };
        let mut bound = count("SELECT count(*) FROM files WHERE missing = 0 AND pending = 1")?;
        if retry_errors {
            bound += count("SELECT count(*) FROM analysis WHERE error IS NOT NULL")?;
        }
        let list = work_list(&tx, &mut tx.prepare(pending_import_sql(retry_errors))?, bound)?;
        tx.commit()?;
        Ok(list)
    }

    /// The reliable PDQ hashes of current analyses (PLAN §6), as ids and hash words: `ids[i]`'s hash is `hashes[i]`, and
    /// a file contributes its 8 orientations. The tier-1 index (`pg_dedup::pdq_index::PdqIndex::build`) is made from them for each import.
    pub fn pdq_hashes(&self) -> Result<(Vec<FileId>, Vec<[u64; 4]>)> {
        // Sized by the present files (an upper bound, counted from `files_present` alone): an exact count of the
        // join below costs a quarter of reading it.
        let present: usize = self.conn.query_row("SELECT count(*) FROM files WHERE missing = 0", [], |r| r.get(0))?;
        // An analysis made before the orientations were stored has its main hash but not these: its pairs would
        // be missing one way round. Those files are re-analyzed (`import --again`), not matched with what they lack.
        let without: i64 = self.conn.query_row(
            "SELECT count(*) FROM analysis a JOIN files f ON f.id = a.file_id
             WHERE f.missing = 0 AND f.pending = 0 AND a.pdq_dihedral IS NULL AND a.pdq_quality >= ?1",
            [pg_model::pdq::MIN_QUALITY],
            |r| r.get(0),
        )?;
        if without > 0 {
            return Err(Error::Other(anyhow!(
                "{without} analyses have no orientation hashes (made by an older build): re-import them"
            )));
        }
        let (mut ids, mut hashes) = (Vec::with_capacity(present * 8), Vec::with_capacity(present * 8));
        let mut stmt = self.conn.prepare(
            "SELECT a.file_id, a.pdq_dihedral FROM analysis a JOIN files f ON f.id = a.file_id
             WHERE f.missing = 0 AND f.pending = 0 AND a.pdq_dihedral IS NOT NULL AND a.pdq_quality >= ?1",
        )?;
        let mut rows = stmt.query([pg_model::pdq::MIN_QUALITY])?;
        while let Some(r) = rows.next()? {
            let id = FileId::new(r.get(0)?);
            let blob: Vec<u8> = r.get(1)?;
            let (orientations, _) = blob.as_chunks::<{ pg_model::pdq::HASH_BYTES }>();
            for orientation in orientations {
                ids.push(id);
                hashes.push(pg_model::pdq::Hash(*orientation).words());
            }
        }
        Ok((ids, hashes))
    }

    /// Analyses whose borders are due (new ones, and all after a new detector: `set_borders_version`), of
    /// files the import has nothing to do for, in on-disk order.
    pub fn pending_borders(&self) -> Result<WorkList<WorkItem>> {
        // An upper bound, from `analysis_borders_due` alone.
        let tx = self.conn.unchecked_transaction()?;
        let bound: usize = tx.query_row("SELECT count(*) FROM analysis WHERE borders_due = 1", [], |r| r.get(0))?;
        let mut stmt = tx.prepare(concat!(
            "SELECT ",
            work_columns!(),
            " FROM analysis a CROSS JOIN files f
             WHERE a.borders_due = 1 AND f.id = a.file_id AND f.missing = 0 AND f.pending = 0
             ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF"
        ))?;
        let list = work_list(&tx, &mut stmt, bound)?;
        drop(stmt);
        tx.commit()?;
        Ok(list)
    }

    /// The present file at an absolute path (Windows paths: any case).
    pub fn file_by_path(&self, path: &RulePath) -> Result<Option<FileId>> {
        let Some((folder, name)) = self.rules.split_file_name(path) else { return Ok(None) };
        let key = self.rules.folder_key(&folder.parts);
        let name_key = name.name_key();
        // The files of the folder's key (every drive's copy of it), then the one at this path by the rules: the same
        // drive, and a name equal by key, which is case-insensitive where the rules are (SQLite's lower() is ASCII).
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.id, f.name, fo.path FROM files f JOIN folders fo ON fo.id = f.folder_id
             WHERE fo.key = ?1 AND f.missing = 0",
        )?;
        let rows =
            stmt.query_map([key], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Vec<u8>>(2)?)))?;
        for row in rows {
            let (id, stored_name, stored_folder) = row?;
            let same_folder = self.rules.parse_bytes(&stored_folder).is_some_and(|f| self.rules.same(&f, &folder));
            let mut stored_key = Vec::new();
            Name::from_raw(&stored_name).push_name_key(&mut stored_key);
            if same_folder && stored_key == name_key {
                return Ok(Some(FileId::new(id)));
            }
        }
        Ok(None)
    }

    /// The present files in a folder and its subfolders (a path in the catalog's case, without a trailing
    /// separator).
    pub fn files_under(&self, folder: &RulePath) -> Result<Vec<FileId>> {
        let mut stmt = self.conn.prepare(concat!(
            "SELECT id FROM files WHERE missing = 0 AND folder_id IN (",
            under_root!(),
            ")"
        ))?;
        let [p1, p2, p3] = under_params(self.rules, folder);
        let ids: Vec<i64> = stmt.query_map(params![p1, p2, p3], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids.into_iter().map(FileId::new).collect())
    }

    /// Make these files due for the import again (`files.pending`), whatever their analysis: after a fix to what
    /// the import reads, which does not bump IMPORT_VERSION. Their next analysis
    /// clears it, as any does (`analysis_added`). Missing files are left as they are. Returns how many it marked.
    pub fn mark_pending(&self, ids: &[FileId]) -> Result<usize> {
        let tx = write_transaction(&self.conn)?;
        let mut marked = 0;
        {
            let mut mark = tx.prepare("UPDATE files SET pending = 1 WHERE id = ?1 AND missing = 0")?;
            for id in ids {
                marked += mark.execute([id.get()])?;
            }
        }
        tx.commit()?;
        Ok(marked)
    }

    /// What cropping a file's borders off needs: its borders as found in the file as it was analyzed (`size`,
    /// `mtime`: the caller checks the file is still that one), its path added to `paths`. `None` when it has none to
    /// crop: none found, marked as not a border, or not looked for in the file as it is.
    pub fn crop_source(&self, file_id: FileId, paths: &mut PathArena) -> Result<Option<CropSource>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT (SELECT path FROM folders WHERE id = f.folder_id), f.name, f.size, f.mtime,
                        a.format, a.orientation, a.width, a.height, a.cut_left, a.cut_top, a.cut_right, a.cut_bottom
                 FROM files f JOIN analysis a ON a.file_id = f.id
                 WHERE f.id = ?1 AND f.missing = 0 AND f.pending = 0 AND a.borders_due = 0
                   AND a.size = f.size AND a.mtime = f.mtime
                   AND a.cut_left + a.cut_top + a.cut_right + a.cut_bottom > 0
                   AND NOT EXISTS(SELECT 1 FROM border_marks WHERE file_id = f.id)",
            )?
            .query_row([file_id.get()], |r| {
                let (orientation, w, h): (Orientation, u32, u32) =
                    (Orientation::from_tag(r.get(5)?), r.get(6)?, r.get(7)?);
                let (width, height) = if orientation.swaps_axes() { (h, w) } else { (w, h) };
                let format = r.get_ref(4)?.as_str()?;
                let format = Format::from_name(format).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(4, Type::Text, format!("unknown format {format}").into())
                })?;
                let (folder, name) = (r.get_ref(0)?.as_blob()?, r.get_ref(1)?.as_blob()?);
                let path = self.rules.sep_byte();
                Ok(CropSource {
                    path: paths.push_parts(&[folder, &[path], name]),
                    size: r.get(2)?,
                    mtime: r.get(3)?,
                    format,
                    width,
                    height,
                    cut: [r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?],
                })
            })
            .optional()?)
    }

    /// Mark a file's borders as not a border, or take the mark back (the review).
    pub fn set_border_mark(&self, file_id: FileId, marked: bool) -> Result<()> {
        if marked {
            self.conn
                .prepare_cached("INSERT OR REPLACE INTO border_marks(file_id, marked) VALUES (?1, unixepoch())")?
                .execute([file_id.get()])?;
        } else {
            self.conn.prepare_cached("DELETE FROM border_marks WHERE file_id = ?1")?.execute([file_id.get()])?;
        }
        Ok(())
    }

    /// Where a file is: its folder and name, exact. `None` when the catalog has no such file.
    pub fn file_path(&self, file_id: FileId) -> Result<Option<RulePath>> {
        let row: Option<(Vec<u8>, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT fo.path, f.name FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1",
                [file_id.get()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(folder, name)| self.file_path_of(&folder, &name)).transpose()
    }

    /// Give back this connection's page cache (the reader does it after each list it made).
    pub fn release_memory(&self) -> Result<()> {
        Ok(self.conn.release_memory()?)
    }

    /// Each count from its own index where there is one (`files_present`, `files_to_import`, `analysis_failed`).
    pub fn status(&self) -> Result<Status> {
        // One read snapshot: the counts agree with each other while a scan writes.
        let snapshot = self.conn.unchecked_transaction()?;
        let one = |sql: &str| -> Result<i64> { Ok(snapshot.query_row(sql, [], |r| r.get(0))?) };
        let files = one("SELECT count(*) FROM files WHERE missing = 0")?;
        let failed = one("SELECT count(*) FROM analysis a CROSS JOIN files f
             WHERE a.error IS NOT NULL AND f.id = a.file_id AND f.missing = 0")?;
        Ok(Status {
            files,
            missing: one("SELECT count(*) FROM files")? - files,
            // Present files with an analysis that isn't a failure, current or not: the one count that reads them all.
            analyzed: one(
                "SELECT count(*) FROM analysis a JOIN files f ON f.id = a.file_id WHERE f.missing = 0 AND a.error IS NULL",
            )?,
            failed,
            pending: one("SELECT count(*) FROM files WHERE missing = 0 AND pending = 1")?,
            pdq_pairs: one("SELECT count(*) FROM present_pdq_pairs")?,
            queued_writes: one("SELECT count(*) FROM file_writes")?,
            failed_writes: one("SELECT count(*) FROM file_writes WHERE error IS NOT NULL")?,
        })
    }
}

/// A transaction on the catalog. It hands out the writers of the scan, the import and the borders, so a batch of
/// their writes commits together, with `commit`.
pub struct Tx<'c> {
    inner: Transaction<'c>,
    rules: PathRules,
}

impl Catalog {
    /// Begin a transaction on the catalog (see [`Tx`]).
    pub fn tx(&mut self) -> Result<Tx<'_>> {
        Ok(Tx { inner: write_transaction(&self.conn)?, rules: self.rules })
    }
}

impl Tx<'_> {
    /// The writer of the scan `scan` of `root`, in this transaction.
    pub fn scan_writer(&self, scan: ScanId, root: &RulePath) -> Result<ScanWriter<'_>> {
        ScanWriter::new(&self.inner, scan, self.rules, root)
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

#[cfg(test)]
mod tests {
    use super::Catalog;
    use crate::browse::fixtures::{spelled, under_both_rules};
    use crate::fixture::{self, raw};
    use crate::test_support::*;
    use crate::{Error, ScannedFile, writes};
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
        let scan = c.begin_scan(&p(rules, r"D:\x")).unwrap();
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
            let job = writes::Job { file_id: id, write, temp: None, attempts: 0 };
            let rewrite = writes::Rewrite { before: Some(before), after, metadata_only };
            writes::done(c, &job, writes::WriteEffect::TurnedBack { rewrite, turns: Turns::NONE }).unwrap();
        };
        rewritten(&c, (11, 101), (12, 102), Some([0; 32]));
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
        let next = c.begin_scan(&p(rules, r"D:\x")).unwrap();
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
        let put = |c: &mut Catalog, scan: i64, folder: &str, name: &str, file_ref: u64| {
            let tx = c.tx().unwrap();
            let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
            let folder = writer.folder(&p(rules, folder)).unwrap();
            let ext = Ext::of_name(name).unwrap();
            let identity = FileIdentity::new(1, file_ref);
            writer
                .upsert_file(
                    &folder,
                    &ScannedFile { identity, name: &Name::from(name), ext, size: 1, mtime: 1, created: None },
                )
                .unwrap();
            drop(writer);
            tx.commit().unwrap();
        };
        let first = c.begin_scan(&p(rules, r"D:\x")).unwrap();
        put(&mut c, first, r"D:\x\ok", "a.jpg", 1);
        put(&mut c, first, r"D:\x\locked", "b.jpg", 2);
        put(&mut c, first, r"D:\x\locked\deep", "c.jpg", 3);

        // The next scan lists only D:\x\ok: D:\x\locked couldn't be read, so its files weren't seen, and aren't missing.
        let next = c.begin_scan(&p(rules, r"D:\x")).unwrap();
        put(&mut c, next, r"D:\x\ok", "a.jpg", 1);
        assert_eq!(c.mark_missing(&p(rules, r"D:\x"), next, &[p(rules, r"D:\x\locked")]).unwrap(), 0);
        assert_eq!(raw::missing_count(&c), 0);

        // Once it can be listed again, a scan that doesn't see its files marks them.
        let later = c.begin_scan(&p(rules, r"D:\x")).unwrap();
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
        raw::add_scan(&c, 1, &spelled(rules, "D:"));
        raw::add_folder(&c, 1, &spelled(rules, r"D:\x"));
        raw::add_folder(&c, 2, &spelled(rules, r"D:\x\y"));
        raw::add_folder(&c, 3, &spelled(rules, r"D:\xy"));
        raw::add_folder(&c, 4, &spelled(rules, r"D:\x\y\z"));
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
