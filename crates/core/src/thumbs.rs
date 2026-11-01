//! Grid thumbnails (PLAN §6): made after the import by a background task, and on demand, at whatever height the
//! grid uses right now. The height is a runtime value, never a stored setting: the cache is keyed by
//! (file, height), so changing it only means new entries, and old heights age out.
//!
//! The cache is a separate SQLite file (one packed file instead of millions of loose ones), capped in bytes. The
//! cap is kept in the file, so every process that writes it (the app, `gallery thumbs`) keeps it to the same one;
//! `open` changes it when asked. The background task fills it up to the cap and may evict other heights to make
//! room for the current one; on-demand requests evict the least recently used entries of any height.
//!
//! The layout keeps every scan off the pixels (FINDINGS "Startup"): the entries (a few dozen bytes each) and the
//! JPEGs are separate tables, so the entries pack densely while the JPEGs stay in their own pages, and the totals
//! are one row kept by triggers, so nothing ever sums the entries. Opening is O(1) at any size, and the totals are
//! right for every connection and process that writes the file (the app, `gallery thumbs`).
//!
//! Reads take no lock: each thread reads through its own connection (WAL readers never wait for the writer).
//! Writes go through one writer thread per cache, in IMMEDIATE transactions: an on-demand thumbnail is handed to it
//! and shown at once, whatever the write does. What fails there has no caller to return to: it goes to the
//! cache's `Report`.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params};
use turbojpeg::{Compressor, Image, PixelFormat, Subsamp};

use crate::decode::{Decoder, MinSize, Rgb};
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
    encode(&decoded.image, height, encoder)
}

/// A thumbnail `height` px high (or the image's own height) of pixels decoded elsewhere (a video's frame).
pub fn encode(img: &Rgb, height: usize, encoder: &mut Compressor) -> Result<Vec<u8>> {
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
    /// For a video, the time of its poster frame (`video::poster_ms`).
    pub poster_ms: Option<i64>,
}

/// The cache's layout version (`PRAGMA user_version`). Before the first release, layout changes edit `SCHEMA` in
/// place and it stays 1: an existing cache file is deleted by hand, like the catalog. A file of another version is
/// refused (the cache is then disabled), never migrated.
pub const CACHE_VERSION: i64 = 1;

/// The cap of a new cache, until one is set (`ThumbCache::open`).
pub const DEFAULT_CAP_BYTES: i64 = 50_000_000_000;

/// The cache file in the catalog folder.
pub fn cache_path(dir: &Path) -> PathBuf {
    dir.join("thumbs.sqlite")
}

/// A hit not touched for this long is touched again: least recently used to the hour, which is what eviction
/// needs, without a write for every thumbnail shown.
const TOUCH_AGE_S: i64 = 3600;
/// Touches are written together, at the latest this long after the first one waiting (or after the last failed
/// try).
const TOUCH_FLUSH: Duration = Duration::from_secs(60);
/// Eviction frees this much more than it must (1% of the cap, at most this), so a full cache doesn't evict on
/// every batch.
const EVICT_SLACK_MAX: i64 = 64 << 20;
/// No transaction evicts much more than this: a cache far over its cap (the cap lowered) is brought down in slices,
/// each its own transaction, so the file is never locked for long (another process waits 5 s at most).
const EVICT_SLICE: i64 = if cfg!(test) { 1000 } else { 256 << 20 };
/// Shrinking (after a pass, at idle) starts when this many pages are free, and frees this many per step, for at
/// most this many steps; freed pages are otherwise reused by later writes.
const SHRINK_ABOVE_PAGES: i64 = if cfg!(test) { 16 } else { 16_384 };
const SHRINK_STEP_PAGES: i64 = 4_096;
const SHRINK_STEPS: usize = 16;

const SCHEMA: &str = "
CREATE TABLE entries(
    id        INTEGER PRIMARY KEY,
    file_id   INTEGER NOT NULL,  -- catalog files.id
    height    INTEGER NOT NULL,  -- requested grid height (the image may be smaller)
    mtime     INTEGER NOT NULL,  -- Source: stale when either differs from the file's
    rotation  INTEGER NOT NULL,
    format    TEXT NOT NULL,
    bytes     INTEGER NOT NULL,
    last_used INTEGER NOT NULL,  -- Unix seconds, to the hour (TOUCH_AGE_S)
    UNIQUE(file_id, height));
-- Eviction reads only this index.
CREATE INDEX entries_lru ON entries(last_used, height, bytes);
-- The JPEG of entry `id`.
CREATE TABLE data(
    id   INTEGER PRIMARY KEY,
    jpeg BLOB NOT NULL);
-- Sum and count of entries, kept by the triggers in the statements that change them.
CREATE TABLE totals(
    one   INTEGER PRIMARY KEY CHECK (one = 1),
    bytes INTEGER NOT NULL,
    count INTEGER NOT NULL);
INSERT INTO totals VALUES (1, 0, 0);
-- The size limit, in bytes, for every process that writes the file (the row is made with the file).
CREATE TABLE limits(
    one INTEGER PRIMARY KEY CHECK (one = 1),
    cap INTEGER NOT NULL);
CREATE TRIGGER entries_added AFTER INSERT ON entries BEGIN
    UPDATE totals SET bytes = bytes + new.bytes, count = count + 1;
END;
CREATE TRIGGER entries_removed AFTER DELETE ON entries BEGIN
    UPDATE totals SET bytes = bytes - old.bytes, count = count - 1;
    DELETE FROM data WHERE id = old.id;
END;
CREATE TRIGGER entries_resized AFTER UPDATE OF bytes ON entries BEGIN
    UPDATE totals SET bytes = bytes - old.bytes + new.bytes;
END;";

/// Settings for every connection to a cache file. Recursive triggers make a REPLACE fire the delete trigger
/// (this code never replaces entries; anything else writing the file might).
const CONNECTION_PRAGMAS: &str =
    "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA recursive_triggers = ON;";

/// Where the writer's own failures go (an on-demand put, touches): nothing waits for those, and the thumbnail was
/// shown anyway.
pub type Report = Box<dyn Fn(&anyhow::Error) + Send>;

/// What a cache holds (bytes, entries) and may hold (`cap`, bytes).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Usage {
    pub bytes: i64,
    pub count: i64,
    pub cap: i64,
}

/// The totals as they were and as the entries have them, when `heal` had to fix them: (bytes, count) each.
pub type Drift = ((i64, i64), (i64, i64));

/// The thumbnail cache. Without a file (it couldn't be opened, `disabled`), nothing is kept: thumbnails are made
/// every time they are shown.
pub struct ThumbCache {
    store: Option<Store>,
}

struct Store {
    path: PathBuf,
    ops: Mutex<Option<Sender<Op>>>,
    writer: Mutex<Option<JoinHandle<()>>>,
}

/// The writer's work, in order.
enum Op {
    Put {
        thumbs: Vec<Thumb>,
        keep_height: Option<u32>,
        reply: Option<Sender<Result<bool>>>,
    },
    /// An entry was shown (its id).
    Touch(i64),
    Retag {
        file_id: i64,
        old_mtime: i64,
        new_mtime: i64,
        reply: Sender<Result<()>>,
    },
    /// Check the totals against the entries, and fix them.
    Heal(Sender<Result<Option<Drift>>>),
    Shrink(Sender<Result<()>>),
    /// Write what's waiting (touches), then answer.
    Flush(Sender<()>),
}

impl ThumbCache {
    /// Open (or create) the cache in the catalog folder `dir` and start its writer. With `cap_bytes`, that is the
    /// cache's cap from now on, for every process (one far over it comes down at the next write); without, the cap
    /// stays as it is (`DEFAULT_CAP_BYTES` for a new cache).
    pub fn open(dir: &Path, cap_bytes: Option<i64>, report: Report) -> Result<ThumbCache> {
        if let Some(cap) = cap_bytes
            && cap < 0
        {
            bail!("a thumbnail cache cap of {cap} bytes");
        }
        let path = cache_path(dir);
        let conn = open_writer(&path, cap_bytes)?;
        let (ops, inbox) = mpsc::channel();
        let writer = std::thread::Builder::new()
            .name("thumbnail cache".into())
            .spawn(move || write_loop(conn, inbox, &report))?;
        Ok(ThumbCache { store: Some(Store { path, ops: Mutex::new(Some(ops)), writer: Mutex::new(Some(writer)) }) })
    }

    /// A cache that keeps nothing, for when the file can't be opened: a cache must never keep the app from
    /// starting.
    pub fn disabled() -> ThumbCache {
        ThumbCache { store: None }
    }

    fn send(&self, op: Op) -> Result<()> {
        let store = self.store.as_ref().ok_or_else(|| anyhow!("no thumbnail cache"))?;
        let ops = store.ops.lock().unwrap();
        ops.as_ref()
            .ok_or_else(|| anyhow!("thumbnail cache closed"))?
            .send(op)
            .map_err(|_| anyhow!("thumbnail cache writer stopped"))
    }

    /// The cached thumbnail closest to `height` that was made from the file as it is now (`source`): the smallest
    /// one at least that high, else the largest below it. Returns its height and bytes.
    pub fn get_nearest(&self, file_id: i64, height: u32, source: Source) -> Result<Option<(u32, Vec<u8>)>> {
        let Some(store) = &self.store else { return Ok(None) };
        let found = with_reader(&store.path, |conn| {
            Ok(conn
                .prepare_cached(
                    "SELECT e.id, e.height, e.last_used, (SELECT jpeg FROM data WHERE id = e.id) FROM entries e
                     WHERE e.file_id = ?1 AND e.mtime = ?3 AND e.rotation = ?4
                     ORDER BY e.height < ?2, abs(e.height - ?2) LIMIT 1",
                )?
                .query_row(params![file_id, height, source.mtime, source.rotation], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?, r.get::<_, i64>(2)?, r.get::<_, Option<Vec<u8>>>(3)?))
                })
                .optional()?)
        })?;
        // An entry without its JPEG (another process deleting it now) is a miss.
        let Some((id, h, last_used, Some(jpeg))) = found else { return Ok(None) };
        if last_used < unix_now() - TOUCH_AGE_S {
            // Best-effort: a cache closing doesn't fail the read.
            let _ = self.send(Op::Touch(id));
        }
        Ok(Some((h, jpeg)))
    }

    /// Store a thumbnail made on demand, in the background, evicting the least recently used entries of any
    /// height if needed. A failure goes to the `Report`: the thumbnail was shown anyway.
    pub fn put(&self, thumb: Thumb) {
        if self.store.is_some() {
            let _ = self.send(Op::Put { thumbs: vec![thumb], keep_height: None, reply: None });
        }
    }

    /// Store a batch in one transaction, waiting for it. With `keep_height`, eviction spares that height (the
    /// background task filling the cache for the current grid); returns false, storing nothing, once it can't make
    /// room. A disabled cache stores nothing either (false: the task stops).
    pub fn put_batch(&self, thumbs: Vec<Thumb>, keep_height: Option<u32>) -> Result<bool> {
        if self.store.is_none() {
            return Ok(false);
        }
        let (reply, answer) = mpsc::channel();
        self.send(Op::Put { thumbs, keep_height, reply: Some(reply) })?;
        answer.recv().map_err(|_| anyhow!("thumbnail cache writer stopped"))?
    }

    /// The file was rewritten without changing its pixels (a rating written into it): its thumbnails still
    /// show it, so they follow it to the new mtime. Waits for it.
    pub fn retag(&self, file_id: i64, old_mtime: i64, new_mtime: i64) -> Result<()> {
        if self.store.is_none() {
            return Ok(());
        }
        let (reply, answer) = mpsc::channel();
        self.send(Op::Retag { file_id, old_mtime, new_mtime, reply })?;
        answer.recv().map_err(|_| anyhow!("thumbnail cache writer stopped"))?
    }

    /// What the cache in the catalog folder `dir` holds and may hold, read without opening it for writing (nothing
    /// created, nothing deleted); none if there is no cache file.
    pub fn read_usage(dir: &Path) -> Result<Option<Usage>> {
        let path = cache_path(dir);
        if !path.exists() {
            return Ok(None);
        }
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Some(conn.query_row("SELECT t.bytes, t.count, l.cap FROM totals t, limits l", [], |r| {
            Ok(Usage { bytes: r.get(0)?, count: r.get(1)?, cap: r.get(2)? })
        })?))
    }

    /// The cap, as the file has it now (another process may have changed it).
    pub fn cap(&self) -> Result<i64> {
        let Some(store) = &self.store else { return Ok(0) };
        with_reader(&store.path, |conn| Ok(conn.query_row("SELECT cap FROM limits", [], |r| r.get(0))?))
    }

    /// Bytes and entries in the cache.
    pub fn totals(&self) -> Result<(i64, i64)> {
        let Some(store) = &self.store else { return Ok((0, 0)) };
        with_reader(&store.path, |conn| {
            Ok(conn.query_row("SELECT bytes, count FROM totals", [], |r| Ok((r.get(0)?, r.get(1)?)))?)
        })
    }

    /// Analyzed files without a current thumbnail at `height`, in on-disk order like the import. Reads on its own
    /// connection, with the catalog (`catalog`) attached; it never waits for the writer, nor the writer for it.
    pub fn pending(&self, catalog: &Path, height: u32) -> Result<Vec<ThumbWork>> {
        let Some(store) = &self.store else { return Ok(Vec::new()) };
        pending_in(&store.path, catalog, height)
    }

    /// Check the totals against the entries, and fix them if they differ (only a writer that bypassed the
    /// triggers could cause that); returns what was fixed. Cheap: the entries are small.
    pub fn heal(&self) -> Result<Option<Drift>> {
        if self.store.is_none() {
            return Ok(None);
        }
        let (reply, answer) = mpsc::channel();
        self.send(Op::Heal(reply))?;
        answer.recv().map_err(|_| anyhow!("thumbnail cache writer stopped"))?
    }

    /// Give back free pages to the file system, a bounded amount (the task does this at idle).
    pub fn shrink(&self) -> Result<()> {
        if self.store.is_none() {
            return Ok(());
        }
        let (reply, answer) = mpsc::channel();
        self.send(Op::Shrink(reply))?;
        answer.recv().map_err(|_| anyhow!("thumbnail cache writer stopped"))?
    }

    /// Wait until everything handed to the writer so far is written (touches included).
    pub fn flush(&self) {
        let (reply, done) = mpsc::channel();
        if self.send(Op::Flush(reply)).is_ok() {
            let _ = done.recv();
        }
    }

    /// Write what is waiting and stop the writer; later writes are dropped and reads keep working. Idempotent.
    pub fn close(&self) {
        let Some(store) = &self.store else { return };
        drop(store.ops.lock().unwrap().take());
        if let Some(writer) = store.writer.lock().unwrap().take() {
            let _ = writer.join();
        }
    }
}

impl Drop for ThumbCache {
    fn drop(&mut self) {
        self.close();
    }
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The writer's connection, with the layout created in a new file, and the cap set if one is given.
fn open_writer(path: &Path, cap_bytes: Option<i64>) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    // auto_vacuum must be set before the first table exists.
    conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
    conn.execute_batch(CONNECTION_PRAGMAS)?;
    let version = |c: &Connection| -> Result<i64> { Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?) };
    if version(&conn)? == 0 {
        // Another process may be creating it too: decide inside the write lock.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if version(&tx)? == 0 {
            tx.execute_batch(&format!("{SCHEMA} PRAGMA user_version = {CACHE_VERSION};"))?;
            tx.execute("INSERT INTO limits VALUES (1, ?1)", [cap_bytes.unwrap_or(DEFAULT_CAP_BYTES)])?;
        }
        tx.commit()?;
    }
    let v = version(&conn)?;
    if v != CACHE_VERSION {
        bail!("{}: layout version {v}, expected {CACHE_VERSION}; it is only a cache: delete it", path.display());
    }
    if let Some(cap) = cap_bytes {
        conn.execute("UPDATE limits SET cap = ?1", [cap])?;
    }
    Ok(conn)
}

/// `ThumbCache::pending` on the cache file `cache` (a free function: the bench calls it without a writer).
pub fn pending_in(cache: &Path, catalog: &Path, height: u32) -> Result<Vec<ThumbWork>> {
    let conn = Connection::open(cache)?;
    // The whole entries table fits in the page cache (~40 bytes an entry); the sort stays in memory.
    conn.execute_batch("PRAGMA query_only = ON; PRAGMA temp_store = MEMORY; PRAGMA cache_size = -131072;")?;
    conn.execute("ATTACH DATABASE ?1 AS cat", [catalog.to_string_lossy()])?;
    let mut stmt = conn.prepare(
        "SELECT f.id, (SELECT path FROM cat.folders WHERE id = f.folder_id) || '\\' || f.name, f.mtime,
                f.rotation, CASE WHEN f.kind = 'video' THEN coalesce(a.poster_ms, 0) END
         FROM cat.files f JOIN cat.analysis a ON a.file_id = f.id
         LEFT JOIN entries t ON t.file_id = f.id AND t.height = ?1
         WHERE f.missing = 0 AND a.error IS NULL AND a.mtime = f.mtime
           AND (t.file_id IS NULL OR t.mtime != f.mtime OR t.rotation != f.rotation)
         ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF",
    )?;
    let rows = stmt.query_map([height], |r| {
        Ok(ThumbWork {
            id: r.get(0)?,
            path: r.get(1)?,
            source: Source { mtime: r.get(2)?, rotation: r.get(3)? },
            poster_ms: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

thread_local! {
    /// This thread's read connections, one per cache file.
    static READERS: RefCell<Vec<(PathBuf, Connection)>> = const { RefCell::new(Vec::new()) };
}

fn with_reader<R>(path: &Path, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
    READERS.with(|readers| {
        let mut readers = readers.borrow_mut();
        let i = match readers.iter().position(|(p, _)| p == path) {
            Some(i) => i,
            None => {
                let conn = Connection::open(path)?;
                conn.execute_batch(CONNECTION_PRAGMAS)?;
                conn.execute_batch("PRAGMA query_only = ON;")?;
                readers.push((path.to_path_buf(), conn));
                readers.len() - 1
            }
        };
        f(&readers[i].1)
    })
}

/// Entries shown, waiting to have their `last_used` written.
#[derive(Default)]
struct Touches {
    ids: BTreeSet<i64>,
    since: Option<Instant>,
}

impl Touches {
    fn due(&self) -> Option<Duration> {
        self.since.map(|t| TOUCH_FLUSH.saturating_sub(t.elapsed()))
    }

    /// Write them in `tx`; they stay queued until `clear` (after the commit).
    fn write(&self, tx: &Transaction) -> Result<()> {
        let mut touch = tx.prepare_cached("UPDATE entries SET last_used = unixepoch() WHERE id = ?1")?;
        for id in &self.ids {
            touch.execute([id])?;
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.ids.clear();
        self.since = None;
    }

    /// Write them in their own transaction. Best-effort: on failure (the file busy, the disk full) they stay
    /// queued, for another try a `TOUCH_FLUSH` later.
    fn flush(&mut self, conn: &mut Connection) -> Result<()> {
        if self.ids.is_empty() {
            return Ok(());
        }
        let written = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(anyhow::Error::from)
            .and_then(|tx| {
                self.write(&tx)?;
                Ok(tx.commit()?)
            });
        match &written {
            Ok(()) => self.clear(),
            Err(_) => self.since = Some(Instant::now()),
        }
        written
    }
}

fn write_loop(mut conn: Connection, inbox: Receiver<Op>, report: &Report) {
    let report_touches = |r: Result<()>| {
        if let Err(e) = r {
            report(&e.context("thumbnail cache: touches not written yet"));
        }
    };
    let mut touches = Touches::default();
    loop {
        let op = match touches.due() {
            Some(wait) => match inbox.recv_timeout(wait) {
                Ok(op) => Some(op),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match inbox.recv() {
                Ok(op) => Some(op),
                Err(_) => break,
            },
        };
        match op {
            None => report_touches(touches.flush(&mut conn)),
            Some(Op::Touch(id)) => {
                touches.ids.insert(id);
                touches.since.get_or_insert_with(Instant::now);
            }
            Some(Op::Put { thumbs, keep_height, reply }) => {
                let stored = put(&mut conn, &thumbs, keep_height, &mut touches);
                match reply {
                    Some(reply) => {
                        let _ = reply.send(stored);
                    }
                    None => {
                        if let Err(e) = stored {
                            report(&e.context("thumbnail cache: a thumbnail not stored"));
                        }
                    }
                }
            }
            Some(Op::Retag { file_id, old_mtime, new_mtime, reply }) => {
                let done = conn
                    .execute(
                        "UPDATE entries SET mtime = ?3 WHERE file_id = ?1 AND mtime = ?2",
                        params![file_id, old_mtime, new_mtime],
                    )
                    .map(drop)
                    .map_err(anyhow::Error::from);
                let _ = reply.send(done);
            }
            Some(Op::Heal(reply)) => {
                let _ = reply.send(heal(&mut conn));
            }
            Some(Op::Shrink(reply)) => {
                let _ = reply.send(shrink(&conn));
            }
            Some(Op::Flush(reply)) => {
                report_touches(touches.flush(&mut conn));
                let _ = reply.send(());
            }
        }
    }
    report_touches(touches.flush(&mut conn));
}

/// Store `thumbs` in one IMMEDIATE transaction: drop the entries they replace, read the total and the cap, evict
/// what's needed, then write. Reading and writing in one write transaction is what keeps two writers (the app and
/// `gallery thumbs`) from both seeing room for their batch, and from failing with SQLITE_BUSY_SNAPSHOT. Dropping
/// the replaced entries first keeps eviction from counting them twice (replaced, then evicted).
fn put(conn: &mut Connection, thumbs: &[Thumb], keep_height: Option<u32>, touches: &mut Touches) -> Result<bool> {
    let incoming: i64 = thumbs.iter().map(|t| t.data.len() as i64).sum();
    // Far over the cap: down to a slice of it first, a slice per transaction.
    loop {
        let (total, cap_bytes) = total_and_cap(conn)?;
        if total + incoming - cap_bytes <= EVICT_SLICE {
            break;
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        touches.write(&tx)?;
        let freed = evict(&tx, EVICT_SLICE, keep_height)?;
        tx.commit()?;
        touches.clear();
        if freed == 0 {
            return Ok(false);
        }
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    {
        let mut replaced = tx.prepare_cached("DELETE FROM entries WHERE file_id = ?1 AND height = ?2")?;
        for t in thumbs {
            replaced.execute(params![t.file_id, t.height])?;
        }
    }
    let (total, cap_bytes) = total_and_cap(&tx)?;
    let excess = total + incoming - cap_bytes;
    let mut touched = false;
    if excess > 0 {
        // What was shown lately must count before choosing what to evict.
        touches.write(&tx)?;
        touched = true;
        if evict(&tx, excess + (cap_bytes / 100).min(EVICT_SLACK_MAX), keep_height)? < excess {
            // No room: store nothing (the replaced and evicted entries come back), but write the touches. If that
            // fails they stay queued for the next flush, which reports it.
            drop(tx);
            let _ = touches.flush(conn);
            return Ok(false);
        }
    }
    {
        // RETURNING, not last_insert_rowid(): when the batch holds one (file, height) twice the upsert updates, and
        // last_insert_rowid() is then another row's id.
        let mut entry = tx.prepare_cached(
            "INSERT INTO entries(file_id, height, mtime, rotation, format, bytes, last_used)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch())
             ON CONFLICT(file_id, height) DO UPDATE SET mtime = excluded.mtime, rotation = excluded.rotation,
                 format = excluded.format, bytes = excluded.bytes, last_used = excluded.last_used
             RETURNING id",
        )?;
        let mut data = tx.prepare_cached(
            "INSERT INTO data(id, jpeg) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET jpeg = excluded.jpeg",
        )?;
        for Thumb { file_id, height, source, data: jpeg } in thumbs {
            let id: i64 = entry.query_row(
                params![file_id, height, source.mtime, source.rotation, FORMAT_JPEG, jpeg.len() as i64],
                |r| r.get(0),
            )?;
            data.execute(params![id, jpeg])?;
        }
    }
    tx.commit()?;
    if touched {
        touches.clear();
    }
    Ok(true)
}

fn total_and_cap(conn: &Connection) -> Result<(i64, i64)> {
    Ok(conn.query_row("SELECT t.bytes, l.cap FROM totals t, limits l", [], |r| Ok((r.get(0)?, r.get(1)?)))?)
}

/// Delete the least recently used entries, sparing `keep_height`, until `target` bytes are freed or none is left;
/// returns the bytes freed.
fn evict(tx: &Transaction, target: i64, keep_height: Option<u32>) -> Result<i64> {
    let mut victims = Vec::new();
    let mut freed = 0;
    {
        let mut lru = tx.prepare_cached("SELECT id, bytes FROM entries WHERE height != ?1 ORDER BY last_used")?;
        let mut rows = lru.query([keep_height.map_or(-1, i64::from)])?;
        while freed < target {
            let Some(r) = rows.next()? else { break };
            victims.push(r.get::<_, i64>(0)?);
            freed += r.get::<_, i64>(1)?;
        }
    }
    let mut delete = tx.prepare_cached("DELETE FROM entries WHERE id = ?1")?;
    for id in victims {
        delete.execute([id])?;
    }
    Ok(freed)
}

fn heal(conn: &mut Connection) -> Result<Option<Drift>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let kept: (i64, i64) = tx.query_row("SELECT bytes, count FROM totals", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let actual: (i64, i64) =
        tx.query_row("SELECT coalesce(sum(bytes), 0), count(*) FROM entries", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    if kept == actual {
        return Ok(None);
    }
    tx.execute("UPDATE totals SET bytes = ?1, count = ?2", params![actual.0, actual.1])?;
    tx.commit()?;
    Ok(Some((kept, actual)))
}

fn shrink(conn: &Connection) -> Result<()> {
    for _ in 0..SHRINK_STEPS {
        let free: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
        if free <= SHRINK_ABOVE_PAGES {
            break;
        }
        // It frees one page per step (a row each), so it is stepped to the end: `execute_batch` would free one.
        let mut step = conn.prepare(&format!("PRAGMA incremental_vacuum({SHRINK_STEP_PAGES})"))?;
        let mut rows = step.query([])?;
        while rows.next()?.is_some() {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder for one test, in `PG_TEST_DIR` (default `D:\PhotoGallery\scratch_thumbs`, never C:).
    fn scratch(name: &str) -> PathBuf {
        let base = std::env::var_os("PG_TEST_DIR")
            .map_or_else(|| PathBuf::from(r"D:\PhotoGallery\scratch_thumbs"), PathBuf::from);
        let dir = base.join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn thumb(file_id: i64, height: u32, mtime: i64, bytes: usize) -> Thumb {
        Thumb { file_id, height, source: Source { mtime, rotation: 0 }, data: vec![file_id as u8; bytes] }
    }

    /// A cache whose writer fails the test on any error it can't return.
    fn open(dir: &Path, cap_bytes: i64) -> ThumbCache {
        ThumbCache::open(dir, Some(cap_bytes), Box::new(|e| panic!("{e:#}"))).unwrap()
    }

    fn src(mtime: i64) -> Source {
        Source { mtime, rotation: 0 }
    }

    /// The totals as a second connection sees them, and the entries' actual sum.
    fn totals_and_sum(dir: &Path) -> ((i64, i64), (i64, i64)) {
        let c = Connection::open(cache_path(dir)).unwrap();
        let kept = c.query_row("SELECT bytes, count FROM totals", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        let actual =
            c.query_row("SELECT coalesce(sum(bytes), 0), count(*) FROM entries", [], |r| Ok((r.get(0)?, r.get(1)?)));
        let orphans: i64 = c
            .query_row("SELECT count(*) FROM data WHERE id NOT IN (SELECT id FROM entries)", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphans, 0, "JPEGs without an entry");
        (kept, actual.unwrap())
    }

    #[test]
    fn nearest_height_and_staleness() {
        let dir = scratch("nearest");
        let cache = open(&dir, 1 << 30);
        assert!(cache.put_batch(vec![thumb(1, 100, 5, 10), thumb(1, 300, 5, 30)], None).unwrap());
        // The smallest at least as high, else the largest below.
        assert_eq!(cache.get_nearest(1, 200, src(5)).unwrap().unwrap().0, 300);
        assert_eq!(cache.get_nearest(1, 90, src(5)).unwrap().unwrap().0, 100);
        assert_eq!(cache.get_nearest(1, 400, src(5)).unwrap().unwrap().0, 300);
        // Made from another version of the file, or another rotation: a miss.
        assert!(cache.get_nearest(1, 100, src(6)).unwrap().is_none());
        assert!(cache.get_nearest(1, 100, Source { mtime: 5, rotation: 1 }).unwrap().is_none());
        assert!(cache.get_nearest(2, 100, src(5)).unwrap().is_none());
        // An entry whose JPEG is gone (another process deleting it right now) is a miss too.
        let c = Connection::open(cache_path(&dir)).unwrap();
        c.execute("DELETE FROM data WHERE id = (SELECT id FROM entries WHERE height = 100)", []).unwrap();
        assert!(cache.get_nearest(1, 90, src(5)).unwrap().is_none());
        assert_eq!(cache.get_nearest(1, 300, src(5)).unwrap().unwrap().0, 300);
    }

    #[test]
    fn replacing_keeps_each_jpeg_with_its_file() {
        let dir = scratch("replace");
        let cache = open(&dir, 1 << 30);
        cache.put_batch(vec![thumb(1, 100, 5, 10)], None).unwrap();
        cache.put_batch(vec![thumb(2, 100, 5, 20)], None).unwrap();
        // Re-put file 1 after file 2 was inserted: an upsert that updates. With last_insert_rowid() this put file 1's
        // JPEG under file 2.
        cache.put_batch(vec![thumb(1, 100, 6, 15)], None).unwrap();
        assert_eq!(cache.get_nearest(1, 100, src(6)).unwrap().unwrap().1, vec![1u8; 15]);
        assert_eq!(cache.get_nearest(2, 100, src(5)).unwrap().unwrap().1, vec![2u8; 20]);
        assert_eq!(cache.totals().unwrap(), (35, 2));
        let (kept, actual) = totals_and_sum(&dir);
        assert_eq!(kept, actual);
    }

    #[test]
    fn eviction() {
        let dir = scratch("evict");
        // Room for 10 thumbnails of 100 bytes (the slack is 1% of the cap: 10 bytes).
        let cache = open(&dir, 1000);
        for i in 0..10 {
            assert!(cache.put_batch(vec![thumb(i, 50, 1, 100)], Some(50)).unwrap());
        }
        // The task can't evict its own height: full.
        assert!(!cache.put_batch(vec![thumb(10, 50, 1, 100)], Some(50)).unwrap());
        assert_eq!(cache.totals().unwrap(), (1000, 10));
        // At another height it may evict the 50 px ones, the least recently used first.
        let c = Connection::open(cache_path(&dir)).unwrap();
        c.execute("UPDATE entries SET last_used = last_used - 100 + file_id", []).unwrap();
        assert!(cache.put_batch(vec![thumb(20, 80, 1, 150)], Some(80)).unwrap());
        assert!(cache.get_nearest(0, 50, src(1)).unwrap().is_none());
        assert!(cache.get_nearest(1, 50, src(1)).unwrap().is_none());
        assert!(cache.get_nearest(2, 50, src(1)).unwrap().is_some());
        // On demand, any height goes; a batch bigger than the cap stores nothing.
        cache.put(thumb(21, 80, 1, 100));
        cache.flush();
        assert!(cache.get_nearest(21, 80, src(1)).unwrap().is_some());
        assert!(!cache.put_batch(vec![thumb(30, 80, 1, 2000)], None).unwrap());
        let (kept, actual) = totals_and_sum(&dir);
        assert_eq!(kept, actual);
        assert!(kept.0 <= 1000);
    }

    #[test]
    fn far_over_a_lowered_cap_comes_down() {
        let dir = scratch("lowered");
        let cache = open(&dir, 1 << 30);
        assert!(cache.put_batch((0..40).map(|i| thumb(i, 50, 1, 100)).collect(), None).unwrap());
        drop(cache);
        // 4000 bytes against a cap of 500: brought down in slices (1000 bytes in tests), then the put.
        let cache = open(&dir, 500);
        cache.put(thumb(100, 80, 1, 100));
        cache.flush();
        assert!(cache.get_nearest(100, 80, src(1)).unwrap().is_some());
        let (kept, actual) = totals_and_sum(&dir);
        assert_eq!(kept, actual);
        assert!(kept.0 <= 500, "{kept:?}");
        // The task's own height is spared: nothing else to evict, so no room.
        assert!(!cache.put_batch(vec![thumb(200, 80, 1, 2000)], Some(80)).unwrap());
    }

    #[test]
    fn one_cap_for_every_writer() {
        let dir = scratch("cap");
        assert_eq!(ThumbCache::read_usage(&dir).unwrap(), None);
        let first = ThumbCache::open(&dir, None, Box::new(|e| panic!("{e:#}"))).unwrap();
        assert_eq!(first.cap().unwrap(), DEFAULT_CAP_BYTES, "a new cache");
        assert!(first.put_batch((0..10).map(|i| thumb(i, 50, 1, 100)).collect(), None).unwrap());
        // Another process sets a lower cap: the first one keeps to it at its next write.
        let second = open(&dir, 500);
        assert_eq!(first.cap().unwrap(), 500);
        assert!(first.put_batch(vec![thumb(20, 80, 1, 100)], Some(80)).unwrap());
        assert!(first.totals().unwrap().0 <= 500);
        // Opened without one, the cap stays.
        drop(second);
        drop(ThumbCache::open(&dir, None, Box::new(|e| panic!("{e:#}"))).unwrap());
        let usage = ThumbCache::read_usage(&dir).unwrap().unwrap();
        assert_eq!((usage.cap, usage.bytes, usage.count), (500, first.totals().unwrap().0, first.totals().unwrap().1));
        assert!(ThumbCache::open(&dir, Some(-1), Box::new(|_| {})).is_err());
    }

    #[test]
    fn shrink_gives_pages_back() {
        let dir = scratch("shrink");
        let cache = open(&dir, 1 << 30);
        assert!(cache.put_batch((0..200).map(|i| thumb(i, 50, 1, 8000)).collect(), None).unwrap());
        let c = Connection::open(cache_path(&dir)).unwrap();
        c.execute("DELETE FROM entries", []).unwrap();
        let free = || -> i64 { c.query_row("PRAGMA freelist_count", [], |r| r.get(0)).unwrap() };
        let before = free();
        assert!(before > 300, "{before}");
        cache.shrink().unwrap();
        assert!(free() <= SHRINK_ABOVE_PAGES, "{before} -> {}", free());
    }

    #[test]
    fn touches_are_written_together() {
        let dir = scratch("touch");
        let cache = open(&dir, 1 << 30);
        cache.put_batch(vec![thumb(1, 100, 5, 10)], None).unwrap();
        let c = Connection::open(cache_path(&dir)).unwrap();
        let last_used = || -> i64 { c.query_row("SELECT last_used FROM entries", [], |r| r.get(0)).unwrap() };
        // Shown an hour ago or more: touched (when the writer flushes); recently: left alone.
        c.execute("UPDATE entries SET last_used = 1000", []).unwrap();
        cache.get_nearest(1, 100, src(5)).unwrap().unwrap();
        assert_eq!(last_used(), 1000, "a touch waits for the flush");
        cache.flush();
        let touched = last_used();
        assert!(touched > unix_now() - 5);
        c.execute("UPDATE entries SET last_used = ?1", [touched - 10]).unwrap();
        cache.get_nearest(1, 100, src(5)).unwrap().unwrap();
        cache.flush();
        assert_eq!(last_used(), touched - 10);
        // Closing writes what's waiting.
        c.execute("UPDATE entries SET last_used = 1000", []).unwrap();
        cache.get_nearest(1, 100, src(5)).unwrap().unwrap();
        cache.close();
        assert!(last_used() > 1000);
    }

    #[test]
    fn retag_follows_the_file() {
        let dir = scratch("retag");
        let cache = open(&dir, 1 << 30);
        cache.put_batch(vec![thumb(1, 100, 5, 10), thumb(1, 200, 5, 10), thumb(2, 100, 5, 10)], None).unwrap();
        cache.retag(1, 5, 9).unwrap();
        assert!(cache.get_nearest(1, 100, src(9)).unwrap().is_some());
        assert!(cache.get_nearest(1, 200, src(9)).unwrap().is_some());
        assert!(cache.get_nearest(2, 100, src(5)).unwrap().is_some());
        // Only from the mtime it had.
        cache.retag(2, 4, 9).unwrap();
        assert!(cache.get_nearest(2, 100, src(5)).unwrap().is_some());
    }

    #[test]
    fn reopened_it_keeps_what_it_held() {
        let dir = scratch("reopen");
        let cache = open(&dir, 1 << 30);
        cache.put_batch(vec![thumb(1, 100, 5, 10)], None).unwrap();
        drop(cache);
        let cache = open(&dir, 1 << 30);
        assert!(cache.get_nearest(1, 100, src(5)).unwrap().is_some());
        assert_eq!(cache.totals().unwrap(), (10, 1));
    }

    #[test]
    fn another_layout_is_refused() {
        let dir = scratch("layout");
        Connection::open(cache_path(&dir)).unwrap().execute_batch("PRAGMA user_version = 7;").unwrap();
        let e = ThumbCache::open(&dir, None, Box::new(|_| {})).err().expect("refused");
        assert!(format!("{e:#}").contains("layout version 7"), "{e:#}");
    }

    #[test]
    fn two_writers_share_one_file() {
        let dir = scratch("two");
        let a = open(&dir, 1 << 30);
        let b = open(&dir, 1 << 30);
        std::thread::scope(|s| {
            for (cache, base) in [(&a, 0), (&b, 1000)] {
                s.spawn(move || {
                    for i in 0..200 {
                        assert!(cache.put_batch(vec![thumb(base + i, 100, 1, 10 + (i as usize % 7))], None).unwrap());
                    }
                });
            }
        });
        let (kept, actual) = totals_and_sum(&dir);
        assert_eq!(kept, actual);
        assert_eq!(kept.1, 400);
    }

    #[test]
    fn totals_survive_any_sequence() {
        let dir = scratch("sequence");
        let cache = open(&dir, 5_000);
        // A fixed pseudo-random walk over puts (new and replacing), on-demand puts, evictions and retags.
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = |n: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % n
        };
        for _ in 0..600 {
            let (file, height, size) = (next(60) as i64, [64, 128, 256][next(3) as usize], 20 + next(400) as usize);
            match next(4) {
                0 => {
                    cache
                        .put_batch(vec![thumb(file, height, 1, size), thumb(file + 1, height, 1, size)], None)
                        .unwrap();
                }
                1 => {
                    cache.put_batch(vec![thumb(file, height, 1, size)], Some(height)).unwrap();
                }
                2 => cache.put(thumb(file, height, 1, size)),
                _ => cache.retag(file, 1, 1).unwrap(),
            }
        }
        cache.flush();
        let (kept, actual) = totals_and_sum(&dir);
        assert_eq!(kept, actual);
        assert!(kept.0 <= 5_000);
    }

    #[test]
    fn heal_fixes_drifted_totals() {
        let dir = scratch("heal");
        let cache = open(&dir, 1 << 30);
        cache.put_batch(vec![thumb(1, 100, 5, 10), thumb(2, 100, 5, 20)], None).unwrap();
        Connection::open(cache_path(&dir)).unwrap().execute("UPDATE totals SET bytes = 999, count = 7", []).unwrap();
        assert_eq!(cache.heal().unwrap(), Some(((999, 7), (30, 2))));
        assert_eq!(cache.totals().unwrap(), (30, 2));
        assert_eq!(cache.heal().unwrap(), None);
    }

    #[test]
    fn pending_against_a_catalog() {
        let dir = scratch("pending");
        let catalog = dir.join("catalog.sqlite");
        let c = crate::catalog::Catalog::open(&catalog).unwrap();
        c.conn
            .execute_batch(
                "INSERT INTO scans(id, root, started) VALUES (1, 'D:\\p', 0);
                 INSERT INTO folders(id, path) VALUES (1, 'D:\\p');",
            )
            .unwrap();
        use crate::catalog::fixture::{File, insert};
        insert(
            &c.conn,
            &[
                File { file_ref: 30, ..File::new(1, 1, "a.jpg", 1, 5) },
                File { file_ref: 10, ..File::new(2, 1, "b.jpg", 1, 5) },
                File { file_ref: 20, rotation: 1, ..File::new(3, 1, "c.jpg", 1, 5) },
                File { file_ref: 40, missing: true, ..File::new(4, 1, "d.jpg", 1, 5) },
            ],
        );
        c.conn
            .execute_batch(
                "INSERT INTO analysis(file_id, version, size, mtime, analyzed) SELECT id, 1, 1, 5, 0 FROM files;",
            )
            .unwrap();
        let cache = open(&dir, 1 << 30);
        // File 1 is current at 100 px; file 3 was turned since its thumbnail; file 4 is missing.
        cache.put_batch(vec![thumb(1, 100, 5, 10), thumb(3, 100, 5, 10)], None).unwrap();
        let ids: Vec<i64> = cache.pending(&catalog, 100).unwrap().iter().map(|w| w.id).collect();
        // In on-disk order (file_ref).
        assert_eq!(ids, vec![2, 3]);
        assert_eq!(cache.pending(&catalog, 200).unwrap().len(), 3);
    }

    #[test]
    fn disabled_keeps_nothing() {
        let cache = ThumbCache::disabled();
        cache.put(thumb(1, 100, 5, 10));
        assert!(!cache.put_batch(vec![thumb(1, 100, 5, 10)], None).unwrap());
        assert!(cache.get_nearest(1, 100, src(5)).unwrap().is_none());
        assert_eq!(cache.totals().unwrap(), (0, 0));
        assert!(cache.pending(Path::new("nowhere"), 100).unwrap().is_empty());
        cache.flush();
        cache.close();
    }
}
