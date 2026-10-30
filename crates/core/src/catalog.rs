//! The catalog (PLAN §6): SQLite in WAL mode, the source of truth for files, analyses, pairs and ratings.
//!
//! File identity is (volume serial, NTFS file ID), so moves and renames need no re-analysis (PLAN §4). An analysis
//! row records the size, mtime and pipeline version it was computed from; when any differ, the file is pending
//! again. That is all the resumability there is: every batch is one transaction, and the work list is a query.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::analyze::{Analysis, IMPORT_VERSION};
use crate::pdq;

/// Schema versioning (`PRAGMA user_version`). A new catalog gets [`BASELINE`] at version [`BASELINE_VERSION`],
/// then every migration in turn; `MIGRATIONS[i]` takes version `BASELINE_VERSION + i` to the next.
///
/// Until there is a catalog worth keeping (the full-library run at the latest, PLAN M4), schema changes edit
/// the baseline and catalogs are rebuilt; from then on the baseline is frozen and every change is a migration.
/// Versions 1–3 were development catalogs from before the baseline: refused, to be deleted and re-imported.
const BASELINE_VERSION: i64 = 4;
const MIGRATIONS: &[&str] = &[];

const BASELINE: &str = r"
CREATE TABLE scans(
    id       INTEGER PRIMARY KEY,
    root     TEXT NOT NULL,
    started  INTEGER NOT NULL,   -- Unix seconds
    finished INTEGER
);
CREATE TABLE folders(
    id   INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE    -- absolute, no trailing backslash
);
CREATE TABLE files(
    id        INTEGER PRIMARY KEY,
    volume    INTEGER NOT NULL,  -- volume serial number
    file_ref  INTEGER NOT NULL,  -- NTFS file ID; the low 48 bits are the MFT record number
    folder_id INTEGER NOT NULL REFERENCES folders(id),
    name      TEXT NOT NULL,
    size      INTEGER NOT NULL,
    mtime     INTEGER NOT NULL,  -- FILETIME: 100 ns since 1601 UTC
    last_scan INTEGER NOT NULL REFERENCES scans(id),
    missing   INTEGER NOT NULL DEFAULT 0,
    rotation  INTEGER NOT NULL DEFAULT 0,  -- quarter turns clockwise kept here, for files that can't be rotated
                                           -- in place (metadata::rotation_method); applied after EXIF orientation
    UNIQUE(volume, file_ref)
);
CREATE INDEX files_folder ON files(folder_id, name);
CREATE TABLE analysis(
    file_id     INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,  -- analyze::IMPORT_VERSION
    size        INTEGER NOT NULL,  -- the file as analyzed
    mtime       INTEGER NOT NULL,
    analyzed    INTEGER NOT NULL,  -- Unix seconds
    error       TEXT,              -- set when the file couldn't be read or decoded; the rest is then NULL
    file_hash   BLOB,              -- tier 0a: BLAKE3 of the bytes
    pixel_hash  BLOB,              -- tier 0b: XXH3-128 of the upright decoded pixels
    format      TEXT,
    orientation INTEGER,
    width       INTEGER,           -- upright, full size
    height      INTEGER,
    pdq         BLOB,              -- tier 1: 32 bytes, Meta's byte order
    pdq_quality INTEGER,
    thumbhash   BLOB
);
CREATE INDEX analysis_file_hash ON analysis(file_hash);
CREATE INDEX analysis_pixel_hash ON analysis(pixel_hash);
-- Tier 1 candidates: PDQ distance <= pdq::MATCH_DISTANCE between reliable hashes, any of the 8 orientations.
CREATE TABLE pdq_pairs(
    a        INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,  -- a < b
    b        INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    distance INTEGER NOT NULL,
    variant  INTEGER NOT NULL,  -- dihedral variant of the newer file that matched (pdq::hash_rgb_dihedral)
    PRIMARY KEY(a, b)
) WITHOUT ROWID;
CREATE INDEX pdq_pairs_b ON pdq_pairs(b);
-- Pairs whose files are both present. A missing file keeps its pairs, in case it comes back.
CREATE VIEW present_pdq_pairs AS
    SELECT p.* FROM pdq_pairs p JOIN files fa ON fa.id = p.a JOIN files fb ON fb.id = p.b
    WHERE fa.missing = 0 AND fb.missing = 0;

-- The library: folders the user added, which a rescan walks again. None is inside another.
CREATE TABLE roots(
    path  TEXT PRIMARY KEY,  -- absolute, no trailing backslash
    added INTEGER NOT NULL   -- Unix seconds
);

-- Ratings: the catalog is the source of truth, for every format (PLAN §2). Unrated files have no row.
CREATE TABLE ratings(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    rating  INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 5),
    updated INTEGER NOT NULL  -- Unix seconds
);
-- The rating the file itself holds, as last read by the import or written by the app (0–5; no row: none).
-- A file rating that differs from this was changed by another program, and is adopted (store_analysis).
CREATE TABLE file_ratings(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    value   INTEGER NOT NULL CHECK (value BETWEEN 0 AND 5)
);
-- Changes waiting to be written into the files (ingest::writeback), so a write survives quitting. `value` is
-- the rating, or the quarter turns clockwise still to apply. `temp` is set once the new content is complete
-- beside the original, carrying `temp_value`; a row with it set is finished by copying it in (the file keeps its
-- NTFS identity). The user may change `value` meanwhile: what is left after `temp_value` is written stays queued.
CREATE TABLE file_writes(
    file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    kind     TEXT NOT NULL CHECK (kind IN ('rating', 'rotation')),
    value    INTEGER NOT NULL,
    queued   INTEGER NOT NULL,  -- Unix seconds
    attempts INTEGER NOT NULL DEFAULT 0,
    error    TEXT,
    temp       TEXT,
    temp_value INTEGER,
    PRIMARY KEY(file_id, kind)
);

-- Saved views (ImageRater's bookmarks): a folder, a rating filter and an order.
CREATE TABLE views(
    name      TEXT PRIMARY KEY,
    folder    TEXT NOT NULL,     -- merged path (browse::merged_path); '' = all folders
    recursive INTEGER NOT NULL,
    filter    TEXT NOT NULL,     -- browse::Filter::key
    sort      TEXT NOT NULL      -- browse::Sort::key
);
-- Where the user was (the open view and the selected file), restored at start.
CREATE TABLE session(
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// Folders equal to root ?1 or below it: path in [root\, root]), as ']' is the character after '\'.
const UNDER_ROOT: &str = "SELECT id FROM folders WHERE path = ?1 OR (path >= ?1 || '\\' AND path < ?1 || ']')";

/// Whether `path` is `root` or inside it (Windows paths: case-insensitive).
pub fn is_under(path: &str, root: &str) -> bool {
    let (p, r) = (path.to_lowercase(), root.to_lowercase());
    p == r || p.strip_prefix(&r).is_some_and(|rest| rest.starts_with('\\'))
}

/// Files whose analysis is absent or stale (?1 = IMPORT_VERSION, ?2 = also retry failed files).
const PENDING: &str = "FROM files f LEFT JOIN analysis a ON a.file_id = f.id
    WHERE f.missing = 0 AND (a.file_id IS NULL OR a.version < ?1 OR a.size != f.size OR a.mtime != f.mtime
          OR (?2 AND a.error IS NOT NULL))";

pub struct Catalog {
    pub conn: Connection,
}

/// Bring a catalog to the current schema: a new one gets the baseline, then each migration it lacks runs in its
/// own transaction.
fn migrate(conn: &Connection, baseline: &str, migrations: &[&str]) -> Result<()> {
    // Each step is all or nothing: the transaction rolls back when dropped on an error.
    let step = |sql: &str, to: i64| -> Result<()> {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", to)?;
        Ok(tx.commit()?)
    };
    let current = BASELINE_VERSION + migrations.len() as i64;
    let mut version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version == 0 {
        step(baseline, BASELINE_VERSION).context("creating the catalog")?;
        version = BASELINE_VERSION;
    }
    if version < BASELINE_VERSION {
        bail!("made by a development build before the baseline schema (version {version}): delete it and re-import");
    }
    if version > current {
        bail!("schema version {version} is newer than this build's ({current})");
    }
    for (i, sql) in migrations.iter().enumerate().skip((version - BASELINE_VERSION) as usize) {
        let to = BASELINE_VERSION + i as i64 + 1;
        step(sql, to).with_context(|| format!("migration to schema version {to}"))?;
    }
    Ok(())
}

/// A file as the scanner saw it.
pub struct ScannedFile<'a> {
    pub volume: u32,
    pub file_ref: u64,
    pub name: &'a str,
    pub size: u64,
    pub mtime: i64,
}

/// A file the import still has to analyze.
pub struct WorkItem {
    pub id: i64,
    pub path: String,
    pub size: i64,
    pub mtime: i64,
    /// Catalog rotation (`files.rotation`), applied when decoding.
    pub rotation: u8,
}

pub struct Status {
    pub files: i64,
    pub missing: i64,
    pub analyzed: i64,
    pub failed: i64,
    pub pending: i64,
    pub pdq_pairs: i64,
    /// Ratings and turns not yet written into the files, and those that keep failing.
    pub queued_writes: i64,
    pub failed_writes: i64,
}

impl Catalog {
    pub fn open(path: &Path) -> Result<Catalog> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
             PRAGMA cache_size = -262144; PRAGMA temp_store = MEMORY;",
        )?;
        migrate(&conn, BASELINE, MIGRATIONS).with_context(|| format!("catalog {}", path.display()))?;
        conn.create_collation("PATH", crate::browse::path_collation)?;
        Ok(Catalog { conn })
    }

    pub fn begin_scan(&self, root: &str) -> Result<i64> {
        self.conn.execute("INSERT INTO scans(root, started) VALUES (?1, unixepoch())", [root])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn finish_scan(&self, scan: i64) -> Result<()> {
        self.conn.execute("UPDATE scans SET finished = unixepoch() WHERE id = ?1", [scan])?;
        Ok(())
    }

    pub fn folder_id(tx: &Transaction, path: &str) -> Result<i64> {
        let mut get = tx.prepare_cached("SELECT id FROM folders WHERE path = ?1")?;
        if let Some(id) = get.query_row([path], |r| r.get(0)).optional()? {
            return Ok(id);
        }
        tx.prepare_cached("INSERT INTO folders(path) VALUES (?1)")?.execute([path])?;
        Ok(tx.last_insert_rowid())
    }

    /// Insert or update by file identity; a moved or renamed file keeps its row (and its analysis).
    pub fn upsert_file(tx: &Transaction, scan: i64, folder: i64, f: &ScannedFile) -> Result<()> {
        tx.prepare_cached(
            "INSERT INTO files(volume, file_ref, folder_id, name, size, mtime, last_scan)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(volume, file_ref) DO UPDATE SET
                folder_id = excluded.folder_id, name = excluded.name, size = excluded.size,
                mtime = excluded.mtime, last_scan = excluded.last_scan, missing = 0",
        )?
        .execute(params![f.volume, f.file_ref as i64, folder, f.name, f.size as i64, f.mtime, scan])?;
        Ok(())
    }

    /// Mark files under `root` that this scan didn't see as missing. Their rows stay (ratings, decisions).
    pub fn mark_missing(&self, root: &str, scan: i64) -> Result<usize> {
        Ok(self.conn.execute(
            &format!(
                "UPDATE files SET missing = 1 WHERE missing = 0 AND last_scan < ?2 AND folder_id IN ({UNDER_ROOT})"
            ),
            params![root, scan],
        )?)
    }

    /// The library folders, by path.
    pub fn roots(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM roots ORDER BY path")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Add a folder to the library. A folder already inside the library adds nothing (returns false); one that
    /// contains library folders replaces them.
    pub fn add_root(&self, root: &str) -> Result<bool> {
        let roots = self.roots()?;
        if roots.iter().any(|r| is_under(root, r)) {
            return Ok(false);
        }
        for r in roots.iter().filter(|r| is_under(r, root)) {
            self.conn.execute("DELETE FROM roots WHERE path = ?1", [r])?;
        }
        self.conn.execute("INSERT INTO roots(path, added) VALUES (?1, unixepoch())", [root])?;
        Ok(true)
    }

    /// Take a folder out of the library. Its files are marked missing, not deleted: adding it back (or the drive
    /// returning) brings them back with their analysis and ratings. Returns how many files it hid.
    pub fn remove_root(&self, root: &str) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM roots WHERE path = ?1", [root])?;
        let n = tx.execute(
            &format!("UPDATE files SET missing = 1 WHERE missing = 0 AND folder_id IN ({UNDER_ROOT})"),
            [root],
        )?;
        tx.commit()?;
        Ok(n)
    }

    /// Files whose analysis is absent or stale, in on-disk order (volume, then MFT record number), which reads
    /// ~3× faster than path order on an HDD (FINDINGS "I/O").
    pub fn pending_import(&self, retry_errors: bool) -> Result<Vec<WorkItem>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT f.id, (SELECT path FROM folders WHERE id = f.folder_id) || '\\' || f.name, f.size, f.mtime,
                    f.rotation
             {PENDING} ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF"
        ))?;
        let rows = stmt.query_map(params![IMPORT_VERSION, retry_errors], |r| {
            Ok(WorkItem { id: r.get(0)?, path: r.get(1)?, size: r.get(2)?, mtime: r.get(3)?, rotation: r.get(4)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Reliable PDQ hashes of current analyses, to rebuild the tier-1 index at startup (PLAN §6).
    pub fn pdq_hashes(&self) -> Result<Vec<(i64, pdq::Hash)>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.file_id, a.pdq FROM analysis a JOIN files f ON f.id = a.file_id
             WHERE f.missing = 0 AND a.version = ?1 AND a.size = f.size AND a.mtime = f.mtime
               AND a.pdq IS NOT NULL AND a.pdq_quality >= ?2",
        )?;
        let rows = stmt.query_map(params![IMPORT_VERSION, pdq::MIN_QUALITY], |r| {
            let bytes: Vec<u8> = r.get(1)?;
            Ok((r.get(0)?, pdq::Hash(bytes.try_into().unwrap_or([0; pdq::HASH_BYTES]))))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn store_analysis(tx: &Transaction, item: &WorkItem, a: &Analysis) -> Result<()> {
        // A re-analysis invalidates the file's tier-1 pairs; the caller re-queries it.
        tx.prepare_cached("DELETE FROM pdq_pairs WHERE a = ?1 OR b = ?1")?.execute([item.id])?;
        tx.prepare_cached(
            "INSERT OR REPLACE INTO analysis(file_id, version, size, mtime, analyzed, error, file_hash, pixel_hash,
                format, orientation, width, height, pdq, pdq_quality, thumbhash)
             VALUES (?1, ?2, ?3, ?4, unixepoch(), NULL, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        )?
        .execute(params![
            item.id,
            IMPORT_VERSION,
            item.size,
            item.mtime,
            &a.file_hash[..],
            &a.pixel_hash[..],
            a.format.name(),
            a.orientation,
            a.width,
            a.height,
            &a.pdq.hash.0[..],
            a.pdq.quality,
            &a.thumbhash,
        ])?;
        Self::sync_file_rating(tx, item.id, a.file_rating)
    }

    /// Three-way rating sync. `file_ratings` holds what the file was last known to hold; a file rating that
    /// differs from it was set by another program after that, so it becomes the catalog rating. Otherwise the
    /// catalog's stands (it may be newer and not written back yet, or the file can't hold one). A write of our own
    /// still queued is newer than anything in the file: nothing is adopted then.
    fn sync_file_rating(tx: &Transaction, file_id: i64, in_file: Option<u8>) -> Result<()> {
        let known: Option<u8> = tx
            .prepare_cached("SELECT value FROM file_ratings WHERE file_id = ?1")?
            .query_row([file_id], |r| r.get(0))
            .optional()?;
        let queued: bool = tx
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM file_writes WHERE file_id = ?1 AND kind = 'rating')")?
            .query_row([file_id], |r| r.get(0))?;
        if in_file == known || queued {
            return Ok(());
        }
        match in_file {
            Some(n @ 1..=5) => tx
                .prepare_cached(
                    "INSERT INTO ratings(file_id, rating, updated) VALUES (?1, ?2, unixepoch())
                     ON CONFLICT(file_id) DO UPDATE SET rating = excluded.rating, updated = excluded.updated",
                )?
                .execute(params![file_id, n])?,
            _ => tx.prepare_cached("DELETE FROM ratings WHERE file_id = ?1")?.execute([file_id])?,
        };
        Self::set_file_rating(tx, file_id, in_file)
    }

    /// Record what the file holds now (read by the import, or just written by the app).
    pub fn set_file_rating(conn: &Connection, file_id: i64, value: Option<u8>) -> Result<()> {
        match value {
            Some(v) => conn
                .prepare_cached(
                    "INSERT INTO file_ratings(file_id, value) VALUES (?1, ?2)
                     ON CONFLICT(file_id) DO UPDATE SET value = excluded.value",
                )?
                .execute(params![file_id, v.min(5)])?,
            None => conn.prepare_cached("DELETE FROM file_ratings WHERE file_id = ?1")?.execute([file_id])?,
        };
        Ok(())
    }

    /// The app rewrote a file in place (ingest::writeback): record its new size and mtime, so the scanner and the
    /// import see it as the file it is. `before` is the (size, mtime) the file had on disk just before the write.
    /// After a metadata-only change (`metadata_only` = the new BLAKE3) an analysis made from exactly those bytes
    /// stays valid and follows the file: nothing is re-imported. After a pixel change it is left stale, so the
    /// import redoes it (new upright size, hashes, PDQ).
    pub fn file_rewritten(
        conn: &Connection,
        file_id: i64,
        before: (i64, i64),
        after: (i64, i64),
        metadata_only: Option<[u8; 32]>,
    ) -> Result<()> {
        let tx = conn.unchecked_transaction()?;
        tx.execute("UPDATE files SET size = ?2, mtime = ?3 WHERE id = ?1", params![file_id, after.0, after.1])?;
        if let Some(file_hash) = metadata_only {
            tx.execute(
                "UPDATE analysis SET size = ?2, mtime = ?3, file_hash = ?4
                 WHERE file_id = ?1 AND size = ?5 AND mtime = ?6",
                params![file_id, after.0, after.1, &file_hash[..], before.0, before.1],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The turn is kept in the catalog (`rotate::Outcome::Catalog`): the file's analysis is redone with it, and
    /// its thumbnails no longer match (thumbs::Source).
    pub fn add_catalog_rotation(conn: &Connection, file_id: i64, turns: u8) -> Result<()> {
        let tx = conn.unchecked_transaction()?;
        tx.execute("UPDATE files SET rotation = (rotation + ?2) % 4 WHERE id = ?1", params![file_id, turns])?;
        tx.execute("UPDATE analysis SET version = 0 WHERE file_id = ?1", [file_id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn store_failure(tx: &Transaction, item: &WorkItem, error: &str) -> Result<()> {
        tx.prepare_cached("DELETE FROM pdq_pairs WHERE a = ?1 OR b = ?1")?.execute([item.id])?;
        tx.prepare_cached(
            "INSERT OR REPLACE INTO analysis(file_id, version, size, mtime, analyzed, error)
             VALUES (?1, ?2, ?3, ?4, unixepoch(), ?5)",
        )?
        .execute(params![item.id, IMPORT_VERSION, item.size, item.mtime, error])?;
        Ok(())
    }

    pub fn store_pdq_pair(tx: &Transaction, a: i64, b: i64, distance: u32, variant: usize) -> Result<()> {
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        tx.prepare_cached(
            "INSERT INTO pdq_pairs(a, b, distance, variant) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(a, b) DO UPDATE SET distance = excluded.distance, variant = excluded.variant
             WHERE excluded.distance < pdq_pairs.distance",
        )?
        .execute(params![a, b, distance, variant])?;
        Ok(())
    }

    pub fn status(&self) -> Result<Status> {
        let one = |sql: &str| -> Result<i64> { Ok(self.conn.query_row(sql, [], |r| r.get(0))?) };
        Ok(Status {
            files: one("SELECT count(*) FROM files WHERE missing = 0")?,
            missing: one("SELECT count(*) FROM files WHERE missing = 1")?,
            analyzed: one(
                "SELECT count(*) FROM analysis a JOIN files f ON f.id = a.file_id WHERE f.missing = 0 AND a.error IS NULL",
            )?,
            failed: one("SELECT count(*) FROM analysis a JOIN files f ON f.id = a.file_id
                 WHERE f.missing = 0 AND a.error IS NOT NULL")?,
            pending: self.conn.query_row(
                &format!("SELECT count(*) {PENDING}"),
                params![IMPORT_VERSION, false],
                |r| r.get(0),
            )?,
            pdq_pairs: one("SELECT count(*) FROM present_pdq_pairs")?,
            queued_writes: one("SELECT count(*) FROM file_writes")?,
            failed_writes: one("SELECT count(*) FROM file_writes WHERE error IS NOT NULL")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_never_nest() {
        let c = Catalog::open(Path::new(":memory:")).unwrap();
        assert!(c.add_root(r"D:\a\b").unwrap());
        assert!(c.add_root(r"D:\a\bc").unwrap());
        assert!(!c.add_root(r"d:\A\b\c").unwrap(), "inside an existing root");
        assert!(c.add_root(r"D:\a").unwrap(), "replaces both");
        assert_eq!(c.roots().unwrap(), [r"D:\a"]);
        c.remove_root(r"D:\a").unwrap();
        assert!(c.roots().unwrap().is_empty());
    }

    fn version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn migrations_apply_once_in_order() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn, BASELINE, &[]).unwrap();
        assert_eq!(version(&conn), BASELINE_VERSION);
        // A later build adds two migrations: both run, in order, once.
        let later = ["CREATE TABLE extra(x INTEGER);", "INSERT INTO extra VALUES (7);"];
        migrate(&conn, BASELINE, &later).unwrap();
        migrate(&conn, BASELINE, &later).unwrap();
        assert_eq!(version(&conn), BASELINE_VERSION + 2);
        assert_eq!(conn.query_row("SELECT count(*) FROM extra", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        // An older build refuses the newer catalog.
        assert!(migrate(&conn, BASELINE, &later[..1]).is_err());
        // A failed migration leaves the catalog at the last good version.
        let broken = ["CREATE TABLE extra(x INTEGER);", "INSERT INTO extra VALUES (7);", "NOT SQL"];
        assert!(migrate(&conn, BASELINE, &broken).is_err());
        assert_eq!(version(&conn), BASELINE_VERSION + 2);
    }

    #[test]
    fn refuses_pre_baseline_catalogs() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE files(id INTEGER); PRAGMA user_version = 3;").unwrap();
        let err = migrate(&conn, BASELINE, MIGRATIONS).unwrap_err();
        assert!(format!("{err:#}").contains("delete it and re-import"));
    }

    /// A catalog with one file in it.
    fn one_file() -> (Catalog, i64) {
        let c = Catalog::open(Path::new(":memory:")).unwrap();
        c.conn
            .execute_batch(
                "INSERT INTO scans(id, root, started) VALUES (1, 'D:', 0);
                 INSERT INTO folders(id, path) VALUES (1, 'D:\\x');
                 INSERT INTO files(id, volume, file_ref, folder_id, name, size, mtime, last_scan)
                    VALUES (1, 1, 1, 1, 'a.jpg', 10, 100, 1);",
            )
            .unwrap();
        (c, 1)
    }

    fn rating(c: &Catalog, id: i64) -> Option<u8> {
        c.conn.query_row("SELECT rating FROM ratings WHERE file_id = ?1", [id], |r| r.get(0)).optional().unwrap()
    }

    fn set_rating(c: &Catalog, id: i64, n: u8) {
        crate::browse::set_rating(&c.conn, id, n, false).unwrap();
    }

    #[test]
    fn three_way_rating_sync() {
        let (c, id) = one_file();
        let sync = |in_file| {
            let tx = c.conn.unchecked_transaction().unwrap();
            Catalog::sync_file_rating(&tx, id, in_file).unwrap();
            tx.commit().unwrap();
        };
        // First import of a file rated elsewhere (ImageRater, Explorer): adopted.
        sync(Some(4));
        assert_eq!(rating(&c, id), Some(4));
        // Rated in the app, not written back (or the format can't hold it): the file still says 4, as known, so
        // re-imports keep the catalog's 2.
        set_rating(&c, id, 2);
        sync(Some(4));
        assert_eq!(rating(&c, id), Some(2));
        // Another program changes the file's rating: that is newer, and adopted.
        sync(Some(5));
        assert_eq!(rating(&c, id), Some(5));
        // Another program clears it (writes 0, or removes it): the catalog follows.
        sync(Some(0));
        assert_eq!(rating(&c, id), None);
        sync(None);
        assert_eq!(rating(&c, id), None);
        // A write of the app's own still queued wins over the file.
        set_rating(&c, id, 3);
        c.conn
            .execute("INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, 'rating', 3, 0)", [id])
            .unwrap();
        sync(Some(1));
        assert_eq!(rating(&c, id), Some(3));
    }
}
