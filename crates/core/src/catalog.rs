//! The catalog (PLAN §6): SQLite in WAL mode, the source of truth for files, analyses, pairs and ratings.
//!
//! File identity is (volume serial, NTFS file ID), so moves and renames need no re-analysis (PLAN §4). An analysis
//! row records the size, mtime and pipeline version it was computed from; when any differ, the file is pending
//! again. That is all the resumability there is: every batch is one transaction, and the work list is a query.

use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::analyze::{Analysis, IMPORT_VERSION};
use crate::pdq;

/// `MIGRATIONS[i]` takes the schema from version i to i + 1; `PRAGMA user_version` holds the current one.
const MIGRATIONS: &[&str] = &[SCHEMA_V1, SCHEMA_V2, SCHEMA_V3];

const SCHEMA_V1: &str = r"
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
";

/// M2: ratings live in the catalog, not in the files (PLAN §2: XMP only round-trips for JPEG).
const SCHEMA_V2: &str = r"
CREATE TABLE ratings(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    rating  INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 5),  -- unrated files have no row
    updated INTEGER NOT NULL                                  -- Unix seconds
);
";

/// The library: folders the user added, which a rescan walks again. None is inside another. Existing catalogs
/// start with the roots they were scanned from.
const SCHEMA_V3: &str = r"
CREATE TABLE roots(
    path  TEXT PRIMARY KEY,  -- absolute, no trailing backslash
    added INTEGER NOT NULL   -- Unix seconds
);
INSERT OR IGNORE INTO roots SELECT root, min(started) FROM scans GROUP BY root;
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
}

pub struct Status {
    pub files: i64,
    pub missing: i64,
    pub analyzed: i64,
    pub failed: i64,
    pub pending: i64,
    pub pdq_pairs: i64,
}

impl Catalog {
    pub fn open(path: &Path) -> Result<Catalog> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
             PRAGMA cache_size = -262144; PRAGMA temp_store = MEMORY;",
        )?;
        let version: usize = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > MIGRATIONS.len() {
            anyhow::bail!("catalog schema {version} is newer than this build ({})", MIGRATIONS.len());
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            conn.execute_batch(&format!("BEGIN; {sql} PRAGMA user_version = {}; COMMIT;", i + 1))?;
        }
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
            "SELECT f.id, (SELECT path FROM folders WHERE id = f.folder_id) || '\\' || f.name, f.size, f.mtime
             {PENDING} ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF"
        ))?;
        let rows = stmt.query_map(params![IMPORT_VERSION, retry_errors], |r| {
            Ok(WorkItem { id: r.get(0)?, path: r.get(1)?, size: r.get(2)?, mtime: r.get(3)? })
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
}
