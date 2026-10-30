//! Grid thumbnails (PLAN §6): made after the import by a background task, and on demand, at whatever height the
//! grid uses right now. The height is a runtime value, never a stored setting: the cache is keyed by
//! (file, height), so changing it only means new entries, and old heights age out.
//!
//! The cache is a separate SQLite file of blobs (one packed file instead of millions of loose ones), capped in
//! bytes. The background task fills it up to the cap and may evict other heights to make room for the current
//! one; on-demand requests evict the least recently used entries of any height.

use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use turbojpeg::{Compressor, Image, PixelFormat, Subsamp};

use crate::decode::{Decoder, MinSize};
use crate::resize::to_height;

/// Lanczos3 from a DCT-scaled decode keeping ≥ 4× the target matches a full decode (FINDINGS "Thumbnails").
const DECODE_MARGIN: usize = 4;
/// libjpeg-turbo q85, 4:2:0 (SSIMULACRA2 ~82; FINDINGS "Thumbnails"). The cache format is still open (PLAN §9.1):
/// JPEG is the fastest to make on demand (~1 ms) and to show (~0.6 ms).
const JPEG_QUALITY: i32 = 85;
pub const FORMAT_JPEG: &str = "jpeg";

/// A thumbnail `height` px high (or the image's own height, if smaller), aspect ratio kept. `turns` is the
/// file's catalog rotation.
pub fn make(
    bytes: &[u8],
    height: usize,
    turns: u8,
    decoder: &mut Decoder,
    encoder: &mut Compressor,
) -> Result<Vec<u8>> {
    let decoded = decoder.decode(bytes, MinSize::Upright(0, height * DECODE_MARGIN), turns)?;
    let img = &decoded.image;
    let small = to_height(img, height);
    let img = small.as_ref().unwrap_or(img);
    encoder.set_quality(JPEG_QUALITY)?;
    encoder.set_subsamp(Subsamp::Sub2x2)?;
    let out = encoder.compress_to_owned(Image {
        pixels: img.pixels.as_slice(),
        width: img.width,
        pitch: img.width * 3,
        height: img.height,
        format: PixelFormat::RGB,
    })?;
    Ok(out.to_vec())
}

pub struct ThumbCache {
    conn: Connection,
    cap_bytes: i64,
    total_bytes: i64,
}

/// What a thumbnail was made from: the file's mtime and its catalog rotation (`files.rotation`). A thumbnail is
/// current while both still match the file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Source {
    pub mtime: i64,
    pub rotation: u8,
}

pub struct Thumb {
    pub file_id: i64,
    /// The requested grid height (the image may be smaller).
    pub height: u32,
    pub source: Source,
    pub data: Vec<u8>,
}

/// A file the background task still has to thumbnail at the current height.
pub struct ThumbWork {
    pub id: i64,
    pub path: String,
    pub source: Source,
}

/// The cache's layout version (`PRAGMA user_version`). It is a cache: a file of another version is emptied and
/// refilled, never migrated.
const CACHE_VERSION: i64 = 2;

const CACHE_SCHEMA: &str = "
CREATE TABLE thumbs(
    file_id   INTEGER NOT NULL,  -- catalog files.id
    height    INTEGER NOT NULL,  -- requested grid height (the image may be smaller)
    mtime     INTEGER NOT NULL,  -- Source: stale when either differs from the file's
    rotation  INTEGER NOT NULL,
    format    TEXT NOT NULL,
    bytes     INTEGER NOT NULL,
    last_used INTEGER NOT NULL,  -- Unix seconds
    data      BLOB NOT NULL,
    PRIMARY KEY(file_id, height));
CREATE INDEX thumbs_lru ON thumbs(last_used);";

impl ThumbCache {
    pub fn open(path: &Path, cap_bytes: i64) -> Result<ThumbCache> {
        let conn = Connection::open(path)?;
        // auto_vacuum must be set before the first table exists.
        conn.execute_batch(
            "PRAGMA auto_vacuum = INCREMENTAL; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;",
        )?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version != CACHE_VERSION {
            conn.execute_batch(&format!(
                "BEGIN; DROP TABLE IF EXISTS thumbs; {CACHE_SCHEMA} PRAGMA user_version = {CACHE_VERSION}; COMMIT;
                 PRAGMA incremental_vacuum;"
            ))?;
        }
        let total_bytes = conn.query_row("SELECT coalesce(sum(bytes), 0) FROM thumbs", [], |r| r.get(0))?;
        Ok(ThumbCache { conn, cap_bytes, total_bytes })
    }

    /// The file was rewritten without changing its pixels (a rating written into it): its thumbnails still
    /// show it, so they follow it to the new mtime.
    pub fn retag(&self, file_id: i64, old_mtime: i64, new_mtime: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE thumbs SET mtime = ?3 WHERE file_id = ?1 AND mtime = ?2",
            params![file_id, old_mtime, new_mtime],
        )?;
        Ok(())
    }

    pub fn total_bytes(&self) -> i64 {
        self.total_bytes
    }

    pub fn is_full(&self) -> bool {
        self.total_bytes >= self.cap_bytes
    }

    /// The cached thumbnail closest to `height` that was made from the file as it is now (`source`): the smallest
    /// one at least that high, else the largest below it. Returns its height and bytes.
    pub fn get_nearest(&self, file_id: i64, height: u32, source: Source) -> Result<Option<(u32, Vec<u8>)>> {
        let found: Option<(u32, Vec<u8>)> = self
            .conn
            .prepare_cached(
                "SELECT height, data FROM thumbs WHERE file_id = ?1 AND mtime = ?3 AND rotation = ?4
                 ORDER BY height < ?2, abs(height - ?2) LIMIT 1",
            )?
            .query_row(params![file_id, height, source.mtime, source.rotation], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        if let Some((h, _)) = &found {
            self.conn
                .prepare_cached("UPDATE thumbs SET last_used = unixepoch() WHERE file_id = ?1 AND height = ?2")?
                .execute(params![file_id, h])?;
        }
        Ok(found)
    }

    /// Store a batch in one transaction. With `keep_height`, eviction spares that height (the background task
    /// filling the cache for the current grid); returns false, storing nothing more, once it can't make room.
    pub fn put_batch(&mut self, items: &[Thumb], keep_height: Option<u32>) -> Result<bool> {
        let incoming: i64 = items.iter().map(|t| t.data.len() as i64).sum();
        if !self.make_room(incoming, keep_height)? {
            return Ok(false);
        }
        let tx = self.conn.transaction()?;
        let mut delta = 0;
        for Thumb { file_id, height, source, data } in items {
            let old: Option<i64> = tx
                .prepare_cached("SELECT bytes FROM thumbs WHERE file_id = ?1 AND height = ?2")?
                .query_row(params![file_id, height], |r| r.get(0))
                .optional()?;
            tx.prepare_cached(
                "INSERT OR REPLACE INTO thumbs(file_id, height, mtime, rotation, format, bytes, last_used, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch(), ?7)",
            )?
            .execute(params![
                file_id,
                height,
                source.mtime,
                source.rotation,
                FORMAT_JPEG,
                data.len() as i64,
                data
            ])?;
            delta += data.len() as i64 - old.unwrap_or(0);
        }
        tx.commit()?;
        self.total_bytes += delta;
        Ok(true)
    }

    /// Evict least recently used entries until `incoming` more bytes fit under the cap.
    fn make_room(&mut self, incoming: i64, keep_height: Option<u32>) -> Result<bool> {
        let excess = self.total_bytes + incoming - self.cap_bytes;
        if excess <= 0 {
            return Ok(true);
        }
        // Evict a little extra, so a full cache doesn't evict on every batch.
        let target = excess + self.cap_bytes / 100;
        let mut stmt =
            self.conn.prepare("SELECT file_id, height, bytes FROM thumbs WHERE height != ?1 ORDER BY last_used")?;
        let mut victims = Vec::new();
        let mut freed = 0;
        let mut rows = stmt.query([keep_height.map_or(-1, |h| h as i64)])?;
        while freed < target {
            let Some(r) = rows.next()? else { break };
            freed += r.get::<_, i64>(2)?;
            victims.push((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?));
        }
        drop(rows);
        drop(stmt);
        if freed < excess {
            return Ok(false);
        }
        let tx = self.conn.transaction()?;
        for (file_id, height) in &victims {
            tx.prepare_cached("DELETE FROM thumbs WHERE file_id = ?1 AND height = ?2")?
                .execute(params![file_id, height])?;
        }
        tx.commit()?;
        self.conn.execute_batch("PRAGMA incremental_vacuum;")?;
        self.total_bytes -= freed;
        Ok(true)
    }

    /// Analyzed files without a current thumbnail at `height`, in on-disk order like the import.
    pub fn pending(&self, catalog: &Path, height: u32) -> Result<Vec<ThumbWork>> {
        self.conn.execute("ATTACH DATABASE ?1 AS cat", [catalog.to_string_lossy()])?;
        let result = (|| {
            let mut stmt = self.conn.prepare(
                "SELECT f.id, (SELECT path FROM cat.folders WHERE id = f.folder_id) || '\\' || f.name, f.mtime,
                        f.rotation
                 FROM cat.files f JOIN cat.analysis a ON a.file_id = f.id
                 LEFT JOIN thumbs t ON t.file_id = f.id AND t.height = ?1
                 WHERE f.missing = 0 AND a.error IS NULL AND a.mtime = f.mtime
                   AND (t.file_id IS NULL OR t.mtime != f.mtime OR t.rotation != f.rotation)
                 ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF",
            )?;
            let rows = stmt.query_map([height], |r| {
                Ok(ThumbWork {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    source: Source { mtime: r.get(2)?, rotation: r.get(3)? },
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })();
        self.conn.execute("DETACH DATABASE cat", [])?;
        result
    }
}
