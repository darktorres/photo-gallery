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
//!
//! The background task doesn't look at every file each round. The catalog numbers its changes (`files.seq`), and
//! a pass that went through every file at a height stores how far it looked (`checked`): the next pass at that
//! height reads only the files changed since. Evicting entries at a height forgets that, and so does a cache
//! started over: the next pass looks at every file again. A cache belongs to one catalog (`meta.catalog_id`, the
//! catalog's): opened for another one (rebuilt, so its file ids mean other files), it starts over.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::path::{FilePath, WorkList};

pub const FORMAT_JPEG: &str = "jpeg";

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
    /// In its list.
    pub path: FilePath,
    pub source: Source,
    /// For a video, the time of its poster frame (`video::poster_ms`).
    pub poster_ms: Option<i64>,
    /// The catalog's change number for it (`files.seq`).
    pub seq: i64,
}

/// The files a pass at `height` has to thumbnail, and what they were read against (`ThumbCache::checked`).
pub struct Pending {
    pub work: WorkList<ThumbWork>,
    pub snapshot: Snapshot,
}

/// Where a pass's work list was read: the catalog's change sequence and the height's evictions at that moment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Snapshot {
    pub height: u32,
    pub seq: i64,
    pub evictions: i64,
    /// Only the files changed since the last complete pass were read (`false`: every file).
    pub since_checked: bool,
}

/// The cache's layout version (`PRAGMA user_version`). Before the first release, layout changes edit `SCHEMA` in
/// place and it stays 1: an existing cache file is deleted by hand, like the catalog. A file of another version is
/// refused (the cache is then disabled), never migrated. A macro, so `SCHEMA` sets it as a constant.
macro_rules! cache_version {
    () => {
        1
    };
}
pub const CACHE_VERSION: i64 = cache_version!();

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
macro_rules! shrink_step_pages {
    () => {
        4096
    };
}
const SHRINK_STEPS: usize = 16;

/// A new cache file's tables, at [`CACHE_VERSION`].
const SCHEMA: &str = concat!(
    "
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
-- About the cache: `catalog_id`, the catalog's (its meta.catalog_id) whose file ids the entries use.
CREATE TABLE meta(
    key   TEXT PRIMARY KEY,
    value ANY NOT NULL);
-- Per height: the catalog's change number (files.seq) up to which every file was checked for a current entry
-- (NULL: not known), and how many times entries of that height were evicted.
CREATE TABLE checked(
    height    INTEGER PRIMARY KEY,
    seq       INTEGER,
    evictions INTEGER NOT NULL);
CREATE TRIGGER entries_added AFTER INSERT ON entries BEGIN
    UPDATE totals SET bytes = bytes + new.bytes, count = count + 1;
END;
CREATE TRIGGER entries_removed AFTER DELETE ON entries BEGIN
    UPDATE totals SET bytes = bytes - old.bytes, count = count - 1;
    DELETE FROM data WHERE id = old.id;
END;
CREATE TRIGGER entries_resized AFTER UPDATE OF bytes ON entries BEGIN
    UPDATE totals SET bytes = bytes - old.bytes + new.bytes;
END; PRAGMA user_version = ",
    cache_version!(),
    ";"
);

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
    /// A batch, given back with the answer: its buffers are the caller's to reuse.
    Put {
        thumbs: Vec<Thumb>,
        keep_height: Option<u32>,
        reply: Sender<(Result<bool>, Vec<Thumb>)>,
    },
    /// One made on demand, stored in the background.
    PutOne(Thumb),
    /// An entry was shown (its id).
    Touch(i64),
    Retag {
        file_id: i64,
        old_mtime: i64,
        new_mtime: i64,
        reply: Sender<Result<()>>,
    },
    /// A pass read from `snapshot` went through all of its work: every file is checked up to `through`.
    Checked {
        snapshot: Snapshot,
        through: i64,
        reply: Sender<Result<()>>,
    },
    /// Check the totals against the entries, and fix them.
    Heal(Sender<Result<Option<Drift>>>),
    Shrink(Sender<Result<()>>),
    /// Write what's waiting (touches), then answer.
    Flush(Sender<()>),
}

impl ThumbCache {
    /// Open (or create) the cache in the catalog folder `dir` for the catalog `catalog_id` (`Catalog::catalog_id`)
    /// and start its writer. A cache of another catalog is deleted and made anew (its cap kept); one another process
    /// has open can't be, and fails. With `cap_bytes`, that is the cache's cap from now on, for every process (one
    /// far over it comes down at the next write); without, the cap stays as it is (`DEFAULT_CAP_BYTES` for a new
    /// cache). The totals are checked against the entries once here (`heal`), a fix reported.
    pub fn open(dir: &Path, catalog_id: &str, cap_bytes: Option<i64>, report: Report) -> Result<ThumbCache> {
        if let Some(cap) = cap_bytes
            && cap < 0
        {
            bail!("a thumbnail cache cap of {cap} bytes");
        }
        let path = cache_path(dir);
        let mut conn = open_writer(&path, catalog_id, cap_bytes)?;
        if let Some((kept, actual)) = heal(&mut conn)? {
            report(&anyhow!("thumbnail cache: totals were {kept:?}, the entries hold {actual:?}; fixed"));
        }
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
    /// one at least that high, else the largest below it. Its bytes are read into `jpeg` (replacing what it held),
    /// a buffer the caller keeps from one thumbnail to the next; returns its height.
    pub fn get_nearest_into(
        &self,
        file_id: i64,
        height: u32,
        source: Source,
        jpeg: &mut Vec<u8>,
    ) -> Result<Option<u32>> {
        let Some(store) = &self.store else { return Ok(None) };
        let found = with_reader(&store.path, |conn| {
            Ok(conn
                .prepare_cached(
                    "SELECT e.id, e.height, e.last_used, (SELECT jpeg FROM data WHERE id = e.id) FROM entries e
                     WHERE e.file_id = ?1 AND e.mtime = ?3 AND e.rotation = ?4
                     ORDER BY e.height < ?2, abs(e.height - ?2) LIMIT 1",
                )?
                .query_row(params![file_id, height, source.mtime, source.rotation], |r| {
                    // An entry without its JPEG (another process deleting it now) is a miss.
                    let Some(bytes) = r.get_ref(3)?.as_blob_or_null()? else { return Ok(None) };
                    jpeg.clear();
                    jpeg.extend_from_slice(bytes);
                    Ok(Some((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?, r.get::<_, i64>(2)?)))
                })
                .optional()?
                .flatten())
        })?;
        let Some((id, h, last_used)) = found else { return Ok(None) };
        if last_used < unix_now() - TOUCH_AGE_S {
            // Best-effort: a cache closing doesn't fail the read.
            let _ = self.send(Op::Touch(id));
        }
        Ok(Some(h))
    }

    /// [`ThumbCache::get_nearest_into`] into a new buffer: its height and bytes.
    #[cfg(test)]
    pub fn get_nearest(&self, file_id: i64, height: u32, source: Source) -> Result<Option<(u32, Vec<u8>)>> {
        let mut jpeg = Vec::new();
        Ok(self.get_nearest_into(file_id, height, source, &mut jpeg)?.map(|h| (h, jpeg)))
    }

    /// Store a thumbnail made on demand, in the background, evicting the least recently used entries of any
    /// height if needed. A failure goes to the `Report`: the thumbnail was shown anyway.
    pub fn put(&self, thumb: Thumb) {
        if self.store.is_some() {
            let _ = self.send(Op::PutOne(thumb));
        }
    }

    /// Store `thumbs` in one transaction, waiting for it. With `keep_height`, eviction spares that height (the
    /// background task filling the cache for the current grid); returns false, storing nothing, once it can't make
    /// room. A disabled cache stores nothing either (false: the task stops). The batch comes back as it went, so
    /// that its `Vec` and its thumbnails' buffers are reused rather than made anew for the next one.
    pub fn put_batch(&self, thumbs: &mut Vec<Thumb>, keep_height: Option<u32>) -> Result<bool> {
        if self.store.is_none() {
            return Ok(false);
        }
        let (reply, answer) = mpsc::channel();
        self.send(Op::Put { thumbs: std::mem::take(thumbs), keep_height, reply })?;
        let (stored, back) = answer.recv().map_err(|_| anyhow!("thumbnail cache writer stopped"))?;
        *thumbs = back;
        stored
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
        // Read-write, as the other connections are: a read-only one can't merge the WAL when it closes last, and
        // would leave the WAL and its index behind.
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
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

    /// Analyzed files without a current thumbnail at `height`, in on-disk order like the import: those changed
    /// since the last complete pass at that height, or every one (`pending_in`). Reads on its own connection, with
    /// the catalog (`catalog`) attached; it never waits for the writer, nor the writer for it.
    pub fn pending(&self, catalog: &Path, height: u32) -> Result<Pending> {
        let Some(store) = &self.store else {
            let snapshot = Snapshot { height, seq: 0, evictions: 0, since_checked: false };
            return Ok(Pending { work: WorkList::default(), snapshot });
        };
        pending_in(&store.path, catalog, height)
    }

    /// A pass went through all the work `pending` gave it: files changed after its snapshot are the next pass's
    /// work. `first_failed`, the smallest `seq` among the files it couldn't thumbnail (a file changed or gone,
    /// a drive offline), keeps those in it. Not stored if entries of that height were evicted meanwhile (they
    /// may be among the files counted as done). Waits for it.
    pub fn checked(&self, snapshot: &Snapshot, first_failed: Option<i64>) -> Result<()> {
        if self.store.is_none() {
            return Ok(());
        }
        let through = first_failed.map_or(snapshot.seq, |seq| snapshot.seq.min(seq - 1));
        let (reply, answer) = mpsc::channel();
        self.send(Op::Checked { snapshot: *snapshot, through, reply })?;
        answer.recv().map_err(|_| anyhow!("thumbnail cache writer stopped"))?
    }

    /// Check the totals against the entries, and fix them if they differ (only a writer that bypassed the
    /// triggers could cause that); returns what was fixed. Cheap: the entries are small. `open` does it.
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
    /// The calling thread's read connections close too: the last connection to a file to close merges its WAL, and
    /// a read after this one opens a new connection.
    pub fn close(&self) {
        if let Some(store) = &self.store {
            drop(store.ops.lock().unwrap().take());
            if let Some(writer) = store.writer.lock().unwrap().take() {
                let _ = writer.join();
            }
        }
        READERS.with(|readers| readers.borrow_mut().clear());
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

/// The writer's connection to a cache of catalog `catalog_id`, with the cap set if one is given. A file it can't use,
/// another catalog's or another layout's (pre-release the layout changes in place), is deleted with its WAL and made
/// anew, keeping its cap where it has one; nothing is deleted entry by entry.
fn open_writer(path: &Path, catalog_id: &str, cap_bytes: Option<i64>) -> Result<Connection> {
    let conn = open_file(path, catalog_id, cap_bytes)?;
    if usable_for(&conn, catalog_id) {
        return Ok(conn);
    }
    let cap: Option<i64> = conn.query_row("SELECT cap FROM limits", [], |r| r.get(0)).ok();
    // The last connection closing checkpoints and deletes the WAL; another process's would keep the files open.
    drop(conn);
    for suffix in ["", "-wal", "-shm"] {
        let file = PathBuf::from(format!("{}{suffix}", path.display()));
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => bail!(
                "{}: made for another catalog or layout, and can't be started over while another program has it \
                 open: {e}",
                file.display()
            ),
        }
    }
    let conn = open_file(path, catalog_id, cap_bytes.or(cap))?;
    if !usable_for(&conn, catalog_id) {
        bail!("{}: not usable even when made anew", path.display());
    }
    Ok(conn)
}

/// Whether the cache file has the current layout and holds catalog `catalog_id`'s thumbnails.
fn usable_for(conn: &Connection, catalog_id: &str) -> bool {
    let layout = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).is_ok_and(|v| v == CACHE_VERSION)
        && ["entries", "data", "totals", "limits", "meta", "checked"].iter().all(|t| {
            conn.query_row("SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1", [t], |r| {
                r.get::<_, i64>(0)
            })
            .is_ok_and(|n| n == 1)
        });
    layout
        && conn
            .query_row("SELECT value FROM meta WHERE key = 'catalog_id'", [], |r| r.get::<_, String>(0))
            .is_ok_and(|owner| owner == catalog_id)
}

/// A connection to the cache file, with the layout created in a new file (for catalog `catalog_id`), and the cap
/// set if one is given and the file has the table for it.
fn open_file(path: &Path, catalog_id: &str, cap_bytes: Option<i64>) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    // auto_vacuum must be set before the first table exists.
    conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
    conn.execute_batch(CONNECTION_PRAGMAS)?;
    let version = |c: &Connection| -> Result<i64> { Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?) };
    if version(&conn)? == 0 {
        // Another process may be creating it too: decide inside the write lock.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if version(&tx)? == 0 {
            tx.execute_batch(SCHEMA)?;
            tx.execute("INSERT INTO limits VALUES (1, ?1)", [cap_bytes.unwrap_or(DEFAULT_CAP_BYTES)])?;
            tx.execute("INSERT INTO meta VALUES ('catalog_id', ?1)", [catalog_id])?;
        }
        tx.commit()?;
    }
    if let Some(cap) = cap_bytes {
        // A file of another layout may lack it: open_writer starts that one over.
        let _ = conn.execute("UPDATE limits SET cap = ?1", [cap]);
    }
    Ok(conn)
}

/// `ThumbCache::pending` on the cache file `cache` (a free function: the bench calls it without a writer).
///
/// One read transaction over both files: the catalog's change number, the height's `checked` row and the work
/// list are one moment's. With a `checked` number no later than the catalog's (a catalog restored from a copy
/// would be behind), only the files changed since are read, through `files_seq`; otherwise every file.
pub fn pending_in(cache: &Path, catalog: &Path, height: u32) -> Result<Pending> {
    let conn = Connection::open(cache)?;
    // The whole entries table fits in the page cache (~40 bytes an entry); the sort stays in memory.
    conn.execute_batch("PRAGMA query_only = ON; PRAGMA temp_store = MEMORY; PRAGMA cache_size = -131072;")?;
    conn.execute("ATTACH DATABASE ?1 AS cat", [catalog.to_string_lossy()])?;
    let tx = conn.unchecked_transaction()?;
    let seq: i64 = tx.query_row("SELECT value FROM cat.meta WHERE key = 'seq'", [], |r| r.get(0))?;
    let (checked, evictions): (Option<i64>, i64) = tx
        .query_row("SELECT seq, evictions FROM checked WHERE height = ?1", [height], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?
        .unwrap_or((None, 0));
    let since = checked.filter(|&c| c <= seq);
    let values = match since {
        Some(c) => vec![height as i64, c],
        None => vec![height as i64],
    };
    let mut work = WorkList::default();
    {
        let mut stmt = tx.prepare(pending_sql(since.is_some()))?;
        let mut rows = stmt.query(rusqlite::params_from_iter(values))?;
        while let Some(r) = rows.next()? {
            let name = r.get_ref(2)?.as_str()?;
            let path =
                work.file(r.get(1)?, name, |arena| Ok::<_, rusqlite::Error>(arena.push(r.get_ref(7)?.as_str()?)))?;
            work.items.push(ThumbWork {
                id: r.get(0)?,
                path,
                source: Source { mtime: r.get(3)?, rotation: r.get(4)? },
                poster_ms: r.get(5)?,
                seq: r.get(6)?,
            });
        }
    }
    tx.commit()?;
    Ok(Pending { work, snapshot: Snapshot { height, seq, evictions, since_checked: since.is_some() } })
}

/// `pending_in`'s query, with the catalog attached as `cat`: ?1 the height, and with `since_checked` ?2 the
/// change number checked up to.
fn pending_sql(since_checked: bool) -> &'static str {
    macro_rules! pending {
        ($changed:literal) => {
            concat!(
                "SELECT f.id, f.folder_id, f.name, f.mtime, f.rotation, CASE WHEN f.kind = 'video' THEN coalesce(f.poster_ms, 0) END, f.seq, f.folder
                 FROM cat.thumb_sources f
                 LEFT JOIN entries t ON t.file_id = f.id AND t.height = ?1
                 WHERE ",
                $changed,
                "f.missing = 0 AND f.error IS NULL AND f.analyzed_mtime = f.mtime
                   AND (t.file_id IS NULL OR t.mtime != f.mtime OR t.rotation != f.rotation)
                 ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF"
            )
        };
    }
    if since_checked { pending!("f.seq > ?2 AND ") } else { pending!("") }
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
                let _ = reply.send((stored, thumbs));
            }
            Some(Op::PutOne(thumb)) => {
                if let Err(e) = put(&mut conn, std::slice::from_ref(&thumb), None, &mut touches) {
                    report(&e.context("thumbnail cache: a thumbnail not stored"));
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
            Some(Op::Checked { snapshot, through, reply }) => {
                // Evicted meanwhile (the counter moved on): not stored.
                let done = conn
                    .execute(
                        "INSERT INTO checked(height, seq, evictions) VALUES (?1, ?2, ?3)
                         ON CONFLICT(height) DO UPDATE SET seq = excluded.seq
                         WHERE checked.evictions = excluded.evictions",
                        params![snapshot.height, through, snapshot.evictions],
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
    // The writer's last act: the WAL merged into the file, so the file is whole even if the process ends before the
    // readers close (the last connection to close deletes the WAL, and a reader may still be open).
    if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)") {
        report(&anyhow::Error::from(e).context("thumbnail cache: the WAL was not merged at close"));
    }
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
/// returns the bytes freed. The heights evicted from forget how far they were checked (`checked`), and count the
/// eviction, so a pass that read its work before it doesn't store its watermark.
fn evict(tx: &Transaction, target: i64, keep_height: Option<u32>) -> Result<i64> {
    let mut victims = Vec::new();
    let mut heights = BTreeSet::new();
    let mut freed = 0;
    {
        let mut lru =
            tx.prepare_cached("SELECT id, bytes, height FROM entries WHERE height != ?1 ORDER BY last_used")?;
        let mut rows = lru.query([keep_height.map_or(-1, i64::from)])?;
        while freed < target {
            let Some(r) = rows.next()? else { break };
            victims.push(r.get::<_, i64>(0)?);
            freed += r.get::<_, i64>(1)?;
            heights.insert(r.get::<_, i64>(2)?);
        }
    }
    let mut delete = tx.prepare_cached("DELETE FROM entries WHERE id = ?1")?;
    for id in victims {
        delete.execute([id])?;
    }
    let mut forget = tx.prepare_cached(
        "INSERT INTO checked(height, seq, evictions) VALUES (?1, NULL, 1)
         ON CONFLICT(height) DO UPDATE SET seq = NULL, evictions = evictions + 1",
    )?;
    for height in heights {
        forget.execute([height])?;
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
        let mut step = conn.prepare(concat!("PRAGMA incremental_vacuum(", shrink_step_pages!(), ")"))?;
        let mut rows = step.query([])?;
        while rows.next()?.is_some() {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{Decoder, MinSize};
    use crate::maker::Maker;
    use pg_testkit::test_folder;
    use turbojpeg::{PixelFormat, Subsamp};

    /// Where this module's tests write (`PG_TEST_DIR` when it is set).
    const SCRATCH: &str = r"D:\PhotoGallery\scratch_thumbs";

    /// Closing the cache leaves no WAL or index file: the writer merges the WAL, and this thread's reader (opened by
    /// the read before) closes with it, so the last connection deletes the files. A reader left open keeps them.
    #[test]
    fn close_merges_the_wal_and_leaves_no_files() {
        let dir = test_folder(SCRATCH, "close-wal");
        let cache = open(&dir, 1 << 20);
        cache.put(thumb(3, 100, 5, 30));
        cache.flush();
        assert_eq!(cache.totals().unwrap(), (30, 1));
        cache.close();
        assert!(!dir.join("thumbs.sqlite-wal").exists(), "the WAL is left");
        assert!(!dir.join("thumbs.sqlite-shm").exists(), "the index is left");
        assert_eq!(totals_and_sum(&dir).0, (30, 1));
    }

    fn thumb(file_id: i64, height: u32, mtime: i64, bytes: usize) -> Thumb {
        Thumb { file_id, height, source: Source { mtime, rotation: 0 }, data: vec![file_id as u8; bytes] }
    }

    /// The catalog the test caches belong to.
    const CATALOG_ID: &str = "test-catalog";

    /// A cache whose writer fails the test on any error it can't return.
    fn open(dir: &Path, cap_bytes: i64) -> ThumbCache {
        ThumbCache::open(dir, CATALOG_ID, Some(cap_bytes), Box::new(|e| panic!("{e:#}"))).unwrap()
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

    /// A JPEG of a pattern, `w` × `h`.
    fn jpeg(w: usize, h: usize, seed: usize) -> Vec<u8> {
        let pixels: Vec<u8> =
            (0..w * h * 3).map(|i| ((i / 3 % w) * 7 + (i / 3 / w) * 3 + (i % 3) * 50 + seed) as u8).collect();
        let image =
            turbojpeg::Image { pixels: &pixels[..], width: w, pitch: w * 3, height: h, format: PixelFormat::RGB };
        turbojpeg::compress(image, 90, Subsamp::Sub2x2).unwrap().to_vec()
    }

    /// Files read into the maker make the thumbnails and decodes their bytes make, file after file (larger, smaller,
    /// missing, not an image), and a missing file fails as `std::fs::read` does.
    #[test]
    fn files_read_into_the_maker_are_their_bytes() {
        let dir = test_folder(SCRATCH, "read-file");
        let files: Vec<(PathBuf, Vec<u8>)> = [(640, 480, 1), (64, 48, 2), (1200, 800, 3), (33, 77, 4)]
            .into_iter()
            .enumerate()
            .map(|(i, (w, h, seed))| (dir.join(format!("{i}.jpg")), jpeg(w, h, seed)))
            .chain([(dir.join("text.jpg"), b"not an image".to_vec())])
            .collect();
        for (path, bytes) in &files {
            std::fs::write(path, bytes).unwrap();
        }
        let missing = dir.join("missing.jpg");
        let mut maker = Maker::new().unwrap();
        for _ in 0..2 {
            for (path, bytes) in &files {
                maker.read_file(path).unwrap();
                let made = maker.make_from_file(120, 1).map_err(|e| format!("{e:#}"));
                let expected = Maker::new().unwrap().make(bytes, 120, 1).map_err(|e| format!("{e:#}"));
                assert_eq!(made, expected, "{}", path.display());
                maker.read_file(path).unwrap();
                let decoded = maker.decode_file(MinSize::Fit(300, 300), 3).map(|d| (d.image.width, d.image.pixels));
                let expected = crate::display::decode(&mut Decoder::new().unwrap(), bytes, MinSize::Fit(300, 300), 3);
                let expected = expected.map(|d| (d.image.width, d.image.pixels));
                assert_eq!(decoded.map_err(|e| format!("{e:#}")), expected.map_err(|e| format!("{e:#}")));
            }
            let err = maker.read_file(&missing).unwrap_err();
            assert_eq!(err.to_string(), std::fs::read(&missing).unwrap_err().to_string());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nearest_height_and_staleness() {
        let dir = test_folder(SCRATCH, "nearest");
        let cache = open(&dir, 1 << 30);
        assert!(cache.put_batch(&mut vec![thumb(1, 100, 5, 10), thumb(1, 300, 5, 30)], None).unwrap());
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

    /// A batch comes back as it went, its thumbnails' buffers with it, for the next batch; one put alone is stored
    /// as a batch of one is. A hit is read into the caller's buffer, replacing what it held.
    #[test]
    fn batches_come_back_and_hits_are_read_into_the_callers_buffer() {
        let dir = test_folder(SCRATCH, "back");
        let cache = open(&dir, 1 << 30);
        let mut batch = vec![thumb(1, 100, 5, 10), thumb(2, 100, 5, 20)];
        assert!(cache.put_batch(&mut batch, None).unwrap());
        assert_eq!(batch.iter().map(|t| (t.file_id, t.data.len())).collect::<Vec<_>>(), [(1, 10), (2, 20)]);
        cache.put(thumb(3, 100, 5, 30));
        cache.flush();
        let mut jpeg = vec![9; 100];
        assert_eq!(cache.get_nearest_into(3, 100, src(5), &mut jpeg).unwrap(), Some(100));
        assert_eq!(jpeg, vec![3u8; 30]);
        // A miss leaves it as it was.
        assert_eq!(cache.get_nearest_into(4, 100, src(5), &mut jpeg).unwrap(), None);
        assert_eq!(jpeg, vec![3u8; 30]);
        assert_eq!(cache.totals().unwrap(), (60, 3));
    }

    #[test]
    fn replacing_keeps_each_jpeg_with_its_file() {
        let dir = test_folder(SCRATCH, "replace");
        let cache = open(&dir, 1 << 30);
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap();
        cache.put_batch(&mut vec![thumb(2, 100, 5, 20)], None).unwrap();
        // Re-put file 1 after file 2 was inserted: an upsert that updates. With last_insert_rowid() this put file 1's
        // JPEG under file 2.
        cache.put_batch(&mut vec![thumb(1, 100, 6, 15)], None).unwrap();
        assert_eq!(cache.get_nearest(1, 100, src(6)).unwrap().unwrap().1, vec![1u8; 15]);
        assert_eq!(cache.get_nearest(2, 100, src(5)).unwrap().unwrap().1, vec![2u8; 20]);
        assert_eq!(cache.totals().unwrap(), (35, 2));
        let (kept, actual) = totals_and_sum(&dir);
        assert_eq!(kept, actual);
    }

    #[test]
    fn eviction() {
        let dir = test_folder(SCRATCH, "evict");
        // Room for 10 thumbnails of 100 bytes (the slack is 1% of the cap: 10 bytes).
        let cache = open(&dir, 1000);
        for i in 0..10 {
            assert!(cache.put_batch(&mut vec![thumb(i, 50, 1, 100)], Some(50)).unwrap());
        }
        // The task can't evict its own height: full.
        assert!(!cache.put_batch(&mut vec![thumb(10, 50, 1, 100)], Some(50)).unwrap());
        assert_eq!(cache.totals().unwrap(), (1000, 10));
        // At another height it may evict the 50 px ones, the least recently used first.
        let c = Connection::open(cache_path(&dir)).unwrap();
        c.execute("UPDATE entries SET last_used = last_used - 100 + file_id", []).unwrap();
        assert!(cache.put_batch(&mut vec![thumb(20, 80, 1, 150)], Some(80)).unwrap());
        assert!(cache.get_nearest(0, 50, src(1)).unwrap().is_none());
        assert!(cache.get_nearest(1, 50, src(1)).unwrap().is_none());
        assert!(cache.get_nearest(2, 50, src(1)).unwrap().is_some());
        // On demand, any height goes; a batch bigger than the cap stores nothing.
        cache.put(thumb(21, 80, 1, 100));
        cache.flush();
        assert!(cache.get_nearest(21, 80, src(1)).unwrap().is_some());
        assert!(!cache.put_batch(&mut vec![thumb(30, 80, 1, 2000)], None).unwrap());
        let (kept, actual) = totals_and_sum(&dir);
        assert_eq!(kept, actual);
        assert!(kept.0 <= 1000);
    }

    #[test]
    fn far_over_a_lowered_cap_comes_down() {
        let dir = test_folder(SCRATCH, "lowered");
        let cache = open(&dir, 1 << 30);
        assert!(cache.put_batch(&mut (0..40).map(|i| thumb(i, 50, 1, 100)).collect(), None).unwrap());
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
        assert!(!cache.put_batch(&mut vec![thumb(200, 80, 1, 2000)], Some(80)).unwrap());
    }

    #[test]
    fn one_cap_for_every_writer() {
        let dir = test_folder(SCRATCH, "cap");
        assert_eq!(ThumbCache::read_usage(&dir).unwrap(), None);
        let first = ThumbCache::open(&dir, CATALOG_ID, None, Box::new(|e| panic!("{e:#}"))).unwrap();
        assert_eq!(first.cap().unwrap(), DEFAULT_CAP_BYTES, "a new cache");
        assert!(first.put_batch(&mut (0..10).map(|i| thumb(i, 50, 1, 100)).collect(), None).unwrap());
        // Another process sets a lower cap: the first one keeps to it at its next write.
        let second = open(&dir, 500);
        assert_eq!(first.cap().unwrap(), 500);
        assert!(first.put_batch(&mut vec![thumb(20, 80, 1, 100)], Some(80)).unwrap());
        assert!(first.totals().unwrap().0 <= 500);
        // Opened without one, the cap stays.
        drop(second);
        drop(ThumbCache::open(&dir, CATALOG_ID, None, Box::new(|e| panic!("{e:#}"))).unwrap());
        let usage = ThumbCache::read_usage(&dir).unwrap().unwrap();
        assert_eq!((usage.cap, usage.bytes, usage.count), (500, first.totals().unwrap().0, first.totals().unwrap().1));
        assert!(ThumbCache::open(&dir, CATALOG_ID, Some(-1), Box::new(|_| {})).is_err());
    }

    #[test]
    fn shrink_gives_pages_back() {
        let dir = test_folder(SCRATCH, "shrink");
        let cache = open(&dir, 1 << 30);
        assert!(cache.put_batch(&mut (0..200).map(|i| thumb(i, 50, 1, 8000)).collect(), None).unwrap());
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
        let dir = test_folder(SCRATCH, "touch");
        let cache = open(&dir, 1 << 30);
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap();
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
        let dir = test_folder(SCRATCH, "retag");
        let cache = open(&dir, 1 << 30);
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10), thumb(1, 200, 5, 10), thumb(2, 100, 5, 10)], None).unwrap();
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
        let dir = test_folder(SCRATCH, "reopen");
        let cache = open(&dir, 1 << 30);
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap();
        drop(cache);
        let cache = open(&dir, 1 << 30);
        assert!(cache.get_nearest(1, 100, src(5)).unwrap().is_some());
        assert_eq!(cache.totals().unwrap(), (10, 1));
    }

    #[test]
    fn another_layout_version_starts_over() {
        let dir = test_folder(SCRATCH, "layout");
        Connection::open(cache_path(&dir)).unwrap().execute_batch("PRAGMA user_version = 7;").unwrap();
        let cache = ThumbCache::open(&dir, CATALOG_ID, None, Box::new(|e| panic!("{e:#}"))).unwrap();
        assert_eq!((cache.totals().unwrap(), cache.cap().unwrap()), ((0, 0), DEFAULT_CAP_BYTES));
    }

    #[test]
    fn two_writers_share_one_file() {
        let dir = test_folder(SCRATCH, "two");
        let a = open(&dir, 1 << 30);
        let b = open(&dir, 1 << 30);
        std::thread::scope(|s| {
            for (cache, base) in [(&a, 0), (&b, 1000)] {
                s.spawn(move || {
                    for i in 0..200 {
                        assert!(
                            cache.put_batch(&mut vec![thumb(base + i, 100, 1, 10 + (i as usize % 7))], None).unwrap()
                        );
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
        let dir = test_folder(SCRATCH, "sequence");
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
                        .put_batch(&mut vec![thumb(file, height, 1, size), thumb(file + 1, height, 1, size)], None)
                        .unwrap();
                }
                1 => {
                    cache.put_batch(&mut vec![thumb(file, height, 1, size)], Some(height)).unwrap();
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
        let dir = test_folder(SCRATCH, "heal");
        let cache = open(&dir, 1 << 30);
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10), thumb(2, 100, 5, 20)], None).unwrap();
        Connection::open(cache_path(&dir)).unwrap().execute("UPDATE totals SET bytes = 999, count = 7", []).unwrap();
        assert_eq!(cache.heal().unwrap(), Some(((999, 7), (30, 2))));
        assert_eq!(cache.totals().unwrap(), (30, 2));
        assert_eq!(cache.heal().unwrap(), None);
    }

    /// A catalog in `dir` with four analyzed files: 1–3 present (3 turned once in the catalog), 4 missing.
    fn catalog_of_four(dir: &Path) -> (PathBuf, crate::catalog::Catalog) {
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
                "INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed)
                 SELECT id, 1, size, mtime, rotation, kind, 0 FROM files;",
            )
            .unwrap();
        (catalog, c)
    }

    fn ids(p: &Pending) -> Vec<i64> {
        p.work.items.iter().map(|w| w.id).collect()
    }

    /// Thumbnails of `bytes` bytes for the work `which` picks, as a pass makes them.
    fn made(p: &Pending, which: impl Fn(&ThumbWork) -> bool, bytes: usize) -> Vec<Thumb> {
        let height = p.snapshot.height;
        p.work
            .items
            .iter()
            .filter(|w| which(w))
            .map(|w| Thumb { file_id: w.id, height, source: w.source, data: vec![0; bytes] })
            .collect()
    }

    #[test]
    fn pending_against_a_catalog() {
        let dir = test_folder(SCRATCH, "pending");
        let (catalog, _c) = catalog_of_four(&dir);
        let cache = open(&dir, 1 << 30);
        // File 1 is current at 100 px; file 3 was turned since its thumbnail; file 4 is missing.
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10), thumb(3, 100, 5, 10)], None).unwrap();
        let p = cache.pending(&catalog, 100).unwrap();
        // In on-disk order (file_ref).
        assert_eq!(ids(&p), vec![2, 3]);
        assert!(!p.snapshot.since_checked);
        assert_eq!(cache.pending(&catalog, 200).unwrap().work.len(), 3);
    }

    #[test]
    fn a_complete_pass_is_where_the_next_one_starts() {
        let dir = test_folder(SCRATCH, "watermark");
        let (catalog, c) = catalog_of_four(&dir);
        let cache = open(&dir, 1 << 30);
        let first = cache.pending(&catalog, 100).unwrap();
        assert_eq!(ids(&first), vec![2, 3, 1]);
        // Done, but file 2 couldn't be made: it stays in the work.
        assert!(cache.put_batch(&mut made(&first, |w| w.id != 2, 10), Some(100)).unwrap());
        let failed = first.work.items.iter().find(|w| w.id == 2).unwrap().seq;
        cache.checked(&first.snapshot, Some(failed)).unwrap();
        let second = cache.pending(&catalog, 100).unwrap();
        assert!(second.snapshot.since_checked);
        assert_eq!(ids(&second), vec![2]);
        cache.checked(&second.snapshot, None).unwrap();
        let third = cache.pending(&catalog, 100).unwrap();
        assert!(third.snapshot.since_checked && third.work.is_empty(), "{:?}", ids(&third));
        // A change in the catalog after it: only that file. (The analysis follows, as a metadata write's does.)
        c.conn
            .execute_batch("UPDATE files SET mtime = 6 WHERE id = 1; UPDATE analysis SET mtime = 6 WHERE file_id = 1;")
            .unwrap();
        assert_eq!(ids(&cache.pending(&catalog, 100).unwrap()), vec![1]);
        // A catalog behind the cache (restored from a copy): every file again.
        c.conn.execute("UPDATE meta SET value = 1 WHERE key = 'seq'", []).unwrap();
        let behind = cache.pending(&catalog, 100).unwrap();
        assert!(!behind.snapshot.since_checked);
        assert_eq!(ids(&behind), vec![2, 1]);
    }

    #[test]
    fn a_pass_after_a_complete_one_reads_the_changed_files() {
        let dir = test_folder(SCRATCH, "seq-plan");
        let (catalog, _c) = catalog_of_four(&dir);
        drop(open(&dir, 1 << 30));
        let conn = Connection::open(cache_path(&dir)).unwrap();
        conn.execute("ATTACH DATABASE ?1 AS cat", [catalog.to_string_lossy()]).unwrap();
        let plan: Vec<String> = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {}", pending_sql(true)))
            .unwrap()
            .query_map(params![100, 0], |r| r.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let plan = plan.join("\n");
        assert!(plan.contains("USING INDEX files_seq"), "{plan}");
    }

    #[test]
    fn eviction_forgets_how_far_a_height_was_checked() {
        let dir = test_folder(SCRATCH, "forget");
        let (catalog, _c) = catalog_of_four(&dir);
        // Room for three of these.
        let cache = open(&dir, 300);
        let pass = cache.pending(&catalog, 100).unwrap();
        assert!(cache.put_batch(&mut made(&pass, |_| true, 95), Some(100)).unwrap());
        cache.checked(&pass.snapshot, None).unwrap();
        assert!(cache.pending(&catalog, 100).unwrap().snapshot.since_checked);
        // Another height's pass evicts from this one: it is looked at whole again.
        assert!(cache.put_batch(&mut vec![thumb(1, 200, 5, 95)], Some(200)).unwrap());
        let after = cache.pending(&catalog, 100).unwrap();
        assert!(!after.snapshot.since_checked);
        assert_eq!(after.work.len(), 1, "the evicted file");
        // Evicted while a pass ran: that pass's watermark isn't stored.
        let during = cache.pending(&catalog, 100).unwrap();
        cache.put(thumb(9, 300, 5, 290));
        cache.flush();
        cache.checked(&during.snapshot, None).unwrap();
        assert!(!cache.pending(&catalog, 100).unwrap().snapshot.since_checked);
    }

    #[test]
    fn a_cache_of_another_catalog_starts_over() {
        let dir = test_folder(SCRATCH, "other-catalog");
        let cache = open(&dir, 12345);
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap();
        drop(cache);
        let reopened = ThumbCache::open(&dir, "rebuilt", None, Box::new(|e| panic!("{e:#}"))).unwrap();
        assert!(reopened.get_nearest(1, 100, src(5)).unwrap().is_none(), "file 1 is another file now");
        assert_eq!((reopened.totals().unwrap(), reopened.cap().unwrap()), ((0, 0), 12345));
        // Open elsewhere: it can't be deleted, so it isn't used.
        let e = ThumbCache::open(&dir, CATALOG_ID, None, Box::new(|_| {})).err().expect("refused");
        assert!(format!("{e:#}").contains("another catalog"), "{e:#}");
    }

    #[test]
    fn a_cache_of_an_earlier_layout_starts_over() {
        let dir = test_folder(SCRATCH, "earlier-layout");
        // Before the catalog id and the watermarks: no `meta`, no `checked`.
        let old = Connection::open(dir.join("thumbs.sqlite")).unwrap();
        old.execute_batch(
            "CREATE TABLE entries(file_id INTEGER, height INTEGER); INSERT INTO entries VALUES (1, 100);
             CREATE TABLE limits(id INTEGER PRIMARY KEY, cap INTEGER NOT NULL); INSERT INTO limits VALUES (1, 777);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        drop(old);
        let cache = ThumbCache::open(&dir, CATALOG_ID, None, Box::new(|e| panic!("{e:#}"))).unwrap();
        assert_eq!((cache.totals().unwrap(), cache.cap().unwrap()), ((0, 0), 777), "empty, its cap kept");
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap();
        assert!(cache.get_nearest(1, 100, src(5)).unwrap().is_some());
    }

    #[test]
    fn disabled_keeps_nothing() {
        let cache = ThumbCache::disabled();
        cache.put(thumb(1, 100, 5, 10));
        assert!(!cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap());
        assert!(cache.get_nearest(1, 100, src(5)).unwrap().is_none());
        assert_eq!(cache.totals().unwrap(), (0, 0));
        assert!(cache.pending(Path::new("nowhere"), 100).unwrap().work.is_empty());
        cache.flush();
        cache.close();
    }
}
