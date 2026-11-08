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
//! are one row kept by triggers, so nothing ever sums the entries. Opening reads the totals, not the entries: they
//! are checked against the entries at most once a day (`HEAL_INTERVAL`, a scan of the entries' sizes), so opening
//! costs the same at any size except on that day's first open. The totals are right for every connection and
//! process that writes the file (the app, `gallery thumbs`).
//!
//! Reads take no lock: each thread reads through its own connection (WAL readers never wait for the writer).
//! Writes go through one writer per cache (a `Service`), in IMMEDIATE transactions: an on-demand thumbnail is handed
//! to it and shown at once, whatever the write does. What fails there has no caller to return to: it is published as
//! an [`Event`] on the bus the cache was opened with.
//!
//! The background task doesn't look at every file each round. The catalog numbers its changes (`files.seq`), and
//! a pass that went through every file at a height stores how far it looked (`checked`): the next pass at that
//! height reads only the files changed since. Evicting entries at a height forgets that, and so does a cache
//! started over: the next pass looks at every file again. A cache belongs to one catalog (`meta.catalog_id`, the
//! catalog's): opened for another one (rebuilt, so its file ids mean other files), it starts over.
// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

use pg_model::ids::FileId;
use pg_model::turns::Turns;
use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use pg_jobs::events::Bus;
use pg_jobs::service::{Service, Waker, Woke};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params};

use pg_model::path::{FilePath, WorkList};
use pg_platform::sqlite::{attach_bytes, sqlite_path};
use pg_schema::Schema;

pub const FORMAT_JPEG: &str = "jpeg";

/// What a thumbnail was made from: the file's mtime and its catalog rotation (`files.rotation`). A thumbnail is
/// current while both still match the file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Source {
    pub mtime: i64,
    pub rotation: Turns,
}

pub struct Thumb {
    pub file_id: FileId,
    /// The requested grid height (the image may be smaller).
    pub height: u32,
    pub source: Source,
    pub data: Vec<u8>,
}

/// A file the background task still has to thumbnail at the current height.
pub struct ThumbWork {
    pub id: FileId,
    /// In its list.
    pub path: FilePath,
    pub source: Source,
    /// The file's size when the work was listed: a thumbnail is kept only if the file still has it.
    pub size: i64,
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

/// The cache's layout version (`PRAGMA user_version`). Before the first release, layout changes edit `SCHEMA` in place
/// and it stays 1. A cache of another version, or whose schema differs from this build's ([`judge`]), is started over,
/// never migrated: it holds derived data only. A macro, so `SCHEMA` sets it as a constant.
macro_rules! cache_version {
    () => {
        1
    };
}
pub const CACHE_VERSION: i64 = cache_version!();

/// The largest cap a new cache gets when none is set (`ThumbCache::open`), and the cap where the volume's size can't be
/// read.
pub const MAX_DEFAULT_CAP_BYTES: i64 = 50_000_000_000;
/// The smallest cap a new cache gets by default.
const MIN_DEFAULT_CAP_BYTES: i64 = 1_000_000_000;

/// The cap of a new cache in `dir` when none is set: a twentieth (5%) of the volume `dir` is on, between 1 GB
/// (`MIN_DEFAULT_CAP_BYTES`) and 50 GB (`MAX_DEFAULT_CAP_BYTES`). A cache on a small disk can't fill it; one on a large
/// disk gets the 50 GB it always had.
pub fn default_cap_bytes(dir: &Path) -> i64 {
    pg_platform::volume::total_bytes(dir)
        .and_then(|total| i64::try_from(total / 20).ok())
        .map_or(MAX_DEFAULT_CAP_BYTES, |share| share.clamp(MIN_DEFAULT_CAP_BYTES, MAX_DEFAULT_CAP_BYTES))
}

/// Parse a cache size limit in GB, as the command line's `--cap-gb` and the app's `PG_THUMB_CAP_GB` give it,
/// into bytes. Zero, negative, NaN, infinite and absurd sizes are refused: `as` would turn them into a cap of 0
/// (which evicts every thumbnail) or a saturated i64.
pub fn parse_cap_gb(text: &str) -> Result<i64, String> {
    let gb: f64 = text.trim().parse().map_err(|_| format!("{text} isn't a number of GB"))?;
    let bytes = gb * 1e9;
    if !(1.0..MAX_CAP_BYTES).contains(&bytes) {
        return Err(format!("{text} GB isn't a size a cache can have (a number of GB above zero)"));
    }
    Ok(whole_bytes(bytes))
}

/// The largest cap `parse_cap_gb` takes, in bytes: under `i64::MAX` (about 9.22e18).
const MAX_CAP_BYTES: f64 = 9.0e18;

/// `bytes` as a whole number of them (the fraction dropped), held to 0..=`MAX_CAP_BYTES` (NaN reads as 0).
fn whole_bytes(bytes: f64) -> i64 {
    pg_model::num::f64_to_i64(bytes.clamp(0.0, MAX_CAP_BYTES))
}

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

/// The `application_id` header field of a cache file (`PGTC`), set when the file is made: with [`SEEDS`] and the schema,
/// what tells a cache from another SQLite file.
const APPLICATION_ID: i64 = 0x5047_5443;

/// The rows that creation seeds and that a cache must have the keys of: `limits` and `totals`. Their values are not
/// compared: the cap is the user's. Not `meta`: the cache writes more keys there while it runs (`healed`), and its
/// `catalog_id` is checked by value on its own.
const SEEDS: &[(&str, &str)] = &[("limits", "one"), ("totals", "one")];

/// Settings for every connection to a cache file. Recursive triggers make a REPLACE fire the delete trigger
/// (this code never replaces entries; anything else writing the file might).
const CONNECTION_PRAGMAS: &str =
    "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA recursive_triggers = ON;";

/// How long a statement waits for another connection's lock before it fails (`busy_timeout`, on every connection to
/// the cache): the writer's transactions and the readers' pass both wait for each other, rather than failing at once.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// How often `open` checks the totals against the entries, which is a scan of them: at most once in this interval.
const HEAL_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// What the cache's writer reports. Nothing waits for these (the thumbnail was shown anyway, the totals were fixed),
/// so they are events on a bus, not results: a subscriber prints them, or counts them. `error` is the error chain as
/// `{:#}` prints it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The totals were off when the cache was opened, and were fixed: what they were, and what the entries hold.
    TotalsFixed { kept: (i64, i64), actual: (i64, i64) },
    /// The times entries were shown could not be written; they stay queued for the next try.
    TouchesNotWritten { error: String },
    /// An on-demand thumbnail could not be stored (it was shown anyway).
    NotStored { error: String },
    /// The WAL could not be merged when the writer closed.
    WalNotMerged { error: String },
    /// The cache file was started over when it was opened, for `reason`: because this build can't use it (made for
    /// another catalog, or by another build), or because it was damaged or not a database. `path` is where the old
    /// file is now for a damaged one, which is moved aside (`thumbs.sqlite.bad`), and the cache file itself, made
    /// anew, for one this build can't use, which is deleted. `detail` is the technical account (the first difference,
    /// SQLite's error), for a log: the words a person reads come from `reason` and `path`. Its thumbnails are made
    /// again; the cap it had is kept, except a damaged file's, which can't be read (the cap given, or the default).
    StartedOver { reason: StartOverReason, path: PathBuf, detail: String },
}

/// Why a cache file was started over ([`Event::StartedOver`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartOverReason {
    /// SQLite found the file's pages don't hold together (`DatabaseCorrupt`): moved aside.
    Damaged,
    /// The file is not a database at all (`NotADatabase`): moved aside.
    NotADatabase,
    /// A cache made for another catalog: deleted, its cap kept.
    OtherCatalog,
    /// A cache made by another build (another layout version, schema, application id or vacuum setting): deleted, its
    /// cap kept.
    OtherBuild,
}

/// Why a file is started over, inside the cache: the reason, and the technical account of it ([`Event::StartedOver`]).
struct Why {
    reason: StartOverReason,
    detail: String,
}

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
    inbox: Arc<Inbox>,
    writer: Service,
    /// The shared hold on the file (`pg_schema::Live`), for as long as the cache is open.
    _live: pg_schema::Live,
}

/// The writer's queue: the ops not yet done, in order, and whether more are taken. The writer is woken after each push.
struct Inbox {
    state: Mutex<Queue>,
}

struct Queue {
    ops: VecDeque<Op>,
    /// False once the cache is closed or its writer has ended: nothing more is queued.
    open: bool,
}

impl Inbox {
    fn new() -> Inbox {
        Inbox { state: Mutex::new(Queue { ops: VecDeque::new(), open: true }) }
    }

    /// Queues `op`; false, queuing nothing, once the queue is closed.
    fn push(&self, op: Op) -> bool {
        let mut queue = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !queue.open {
            return false;
        }
        queue.ops.push_back(op);
        true
    }

    fn next(&self) -> Option<Op> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).ops.pop_front()
    }

    /// Stops taking ops; those queued are still done.
    fn close(&self) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).open = false;
    }
}

/// Closes the queue when the writer's body is left, by a return or a panic. What is still queued is dropped: a caller
/// waiting for an answer to one of those gets none (its `recv` fails).
struct Closing<'a>(&'a Inbox);

impl Drop for Closing<'_> {
    fn drop(&mut self) {
        let mut queue = self.0.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.open = false;
        queue.ops.clear();
    }
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
        file_id: FileId,
        old_mtime: i64,
        new_mtime: i64,
        reply: Sender<Result<()>>,
    },
    /// The file's thumbnails are dropped: its content was replaced (S-17.7 decision 7).
    Forget {
        file_id: FileId,
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
    /// has open can't be, and fails (as does a writer thread that can't be spawned). With `cap_bytes`, that is the
    /// cache's cap from now on, for every process (one far over it comes down at the next write); without, the cap
    /// stays as it is ([`default_cap_bytes`] for a new cache). The totals are checked against the entries once here
    /// (`heal`, if the last check was `HEAL_INTERVAL` ago or more), a fix published on `events` before this returns:
    /// subscribe first. A file SQLite finds damaged, or that is not a database at all, is moved aside to
    /// `thumbs.sqlite.bad` and made anew (an [`Event::StartedOver`] says so): damage in the file's header or layout is
    /// found here, damage among the entries only at the day's check of the totals, so up to a day late.
    pub fn open(dir: &Path, catalog_id: &str, cap_bytes: Option<i64>, events: Arc<Bus<Event>>) -> Result<ThumbCache> {
        if let Some(cap) = cap_bytes
            && cap < 0
        {
            bail!("a thumbnail cache cap of {cap} bytes");
        }
        let path = cache_path(dir);
        let Writing { conn, started_over, drift, live } = open_writer(&path, catalog_id, cap_bytes)?;
        if let Some((why, old)) = started_over {
            events.publish(Event::StartedOver { reason: why.reason, path: old, detail: why.detail });
        }
        if let Some((kept, actual)) = drift {
            events.publish(Event::TotalsFixed { kept, actual });
        }
        let inbox = Arc::new(Inbox::new());
        let body = inbox.clone();
        let writer = Service::start("thumbnail cache", move |waker| write_loop(conn, &body, &events, waker))
            .context("start the thumbnail cache writer")?;
        Ok(ThumbCache { store: Some(Store { path, inbox, writer, _live: live }) })
    }

    /// A cache that keeps nothing, for when the file can't be opened: a cache must never keep the app from
    /// starting.
    pub fn disabled() -> ThumbCache {
        ThumbCache { store: None }
    }

    fn send(&self, op: Op) -> Result<()> {
        let store = self.store.as_ref().ok_or_else(|| anyhow!("no thumbnail cache"))?;
        if !store.inbox.push(op) {
            bail!("thumbnail cache closed");
        }
        store.writer.wake();
        Ok(())
    }

    /// The cached thumbnail closest to `height` that was made from the file as it is now (`source`): the smallest
    /// one at least that high, else the largest below it. Its bytes are read into `jpeg` (replacing what it held),
    /// a buffer the caller keeps from one thumbnail to the next; returns its height.
    pub fn get_nearest_into(
        &self,
        file_id: FileId,
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
                .query_row(params![file_id.get(), height, source.mtime, source.rotation.get()], |r| {
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
    pub fn get_nearest(&self, file_id: FileId, height: u32, source: Source) -> Result<Option<(u32, Vec<u8>)>> {
        let mut jpeg = Vec::new();
        Ok(self.get_nearest_into(file_id, height, source, &mut jpeg)?.map(|h| (h, jpeg)))
    }

    /// Store a thumbnail made on demand, in the background, evicting the least recently used entries of any
    /// height if needed. A failure is published as an [`Event::NotStored`]: the thumbnail was shown anyway.
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
    pub fn retag(&self, file_id: FileId, old_mtime: i64, new_mtime: i64) -> Result<()> {
        if self.store.is_none() {
            return Ok(());
        }
        let (reply, answer) = mpsc::channel();
        self.send(Op::Retag { file_id, old_mtime, new_mtime, reply })?;
        answer.recv().map_err(|_| anyhow!("thumbnail cache writer stopped"))?
    }

    /// Drop the thumbnails of a file whose content was replaced under its id (the catalog's pairing says which). Waits.
    pub fn forget(&self, file_id: FileId) -> Result<()> {
        if self.store.is_none() {
            return Ok(());
        }
        let (reply, answer) = mpsc::channel();
        self.send(Op::Forget { file_id, reply })?;
        answer.recv().map_err(|_| anyhow!("thumbnail cache writer stopped"))?
    }

    /// What the cache in the catalog folder `dir` holds and may hold, read without opening it for writing (nothing
    /// created, nothing deleted); none if there is no cache file.
    pub fn read_usage(dir: &Path) -> Result<Option<Usage>> {
        let path = cache_path(dir);
        if !path.exists() {
            return Ok(None);
        }
        // Held shared for the read: a start over elsewhere waits for it, as it waits for the cache's own connections.
        let _live = hold_shared(&path)?;
        // Read-write, as the other connections are: a read-only one can't merge the WAL when it closes last, and
        // would leave the WAL and its index behind.
        let conn = Connection::open_with_flags(sqlite_path(&path)?, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
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
    /// triggers could cause that); returns what was fixed. A scan of the entries: `open` does it at most once a day.
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
            store.inbox.close();
            store.writer.stop();
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
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// What a cache file is, for a writer of catalog `catalog_id`, before anything is written to it.
enum Found {
    /// This build's cache, of this catalog: kept.
    Usable,
    /// No file, or an empty one: made.
    New,
    /// A cache this build can't use (of another catalog, another layout, or an earlier build's): started over, with
    /// the reason and the cap it had.
    Stale { why: Why, cap: Option<i64> },
    /// Not a database, or one SQLite finds damaged (`NotADatabase`, `DatabaseCorrupt`): moved aside to
    /// `<name>.bad` and made anew. Nothing in it can be trusted, its cap included, and it held derived data only.
    Damaged { why: Why },
    /// A database that is not a thumbnail cache: refused, and left as it is.
    Foreign { why: String },
}

/// Why `e` says the file is damaged, if its code is one of SQLite's for that: not a database at all
/// (`NotADatabase`), or a database whose pages don't hold together (`DatabaseCorrupt`). Any other failure (busy, no
/// permission, a full disk) is not the file's fault, and stays an error.
fn damage(e: &rusqlite::Error) -> Option<Why> {
    match e.sqlite_error_code() {
        Some(rusqlite::ErrorCode::NotADatabase) => {
            Some(Why { reason: StartOverReason::NotADatabase, detail: "it is not a database".to_string() })
        }
        Some(rusqlite::ErrorCode::DatabaseCorrupt) => {
            Some(Why { reason: StartOverReason::Damaged, detail: format!("it is damaged ({e})") })
        }
        _ => None,
    }
}

/// [`damage`] for the first SQLite error in `e`'s chain.
fn damage_in(e: &anyhow::Error) -> Option<Why> {
    e.chain().filter_map(|cause| cause.downcast_ref::<rusqlite::Error>()).find_map(damage)
}

/// The cache's schema as this build makes it, for the comparison in [`judge`]. Made in memory, once per process: the
/// catalog's id is not part of the schema (only the keys of its rows are compared, so any id will do here).
fn expected() -> Result<&'static Schema> {
    static EXPECTED: OnceLock<Schema> = OnceLock::new();
    if let Some(schema) = EXPECTED.get() {
        return Ok(schema);
    }
    let conn = Connection::open_in_memory()?;
    make_layout(&conn, "", MAX_DEFAULT_CAP_BYTES)?;
    let schema = Schema::read(&conn, SEEDS)?;
    Ok(EXPECTED.get_or_init(|| schema))
}

/// Make the cache's layout in `conn`, an empty database: its tables ([`SCHEMA`]), its application id, the cap row and the
/// catalog's id. Creating a file and making the expected schema both do this, so they can't disagree.
fn make_layout(conn: &Connection, catalog_id: &str, cap_bytes: i64) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    conn.pragma_update(None, "application_id", APPLICATION_ID)?;
    conn.execute("INSERT INTO limits VALUES (1, ?1)", [cap_bytes])?;
    conn.execute("INSERT INTO meta VALUES ('catalog_id', ?1)", [catalog_id])?;
    Ok(())
}

/// What the file at `path` is, for a writer of catalog `catalog_id`. It reads only: a file that is not ours gets no
/// pragma and no write, so it is left as it was. A read SQLite fails as damage ([`damage`]) is [`Found::Damaged`].
fn judge(path: &Path, catalog_id: &str) -> Result<Found> {
    if !path.exists() {
        return Ok(Found::New);
    }
    let expected = expected()?;
    let conn = Connection::open(sqlite_path(path)?)?;
    match read_found(&conn, catalog_id, expected) {
        Ok(found) => Ok(found),
        Err(e) => match damage(&e) {
            Some(why) => Ok(Found::Damaged { why }),
            None => Err(e.into()),
        },
    }
}

/// [`judge`]'s reads, on the file's connection `conn`: its schema compared with this build's (`expected`), its header
/// fields and its owner.
fn read_found(conn: &Connection, catalog_id: &str, expected: &Schema) -> rusqlite::Result<Found> {
    let schema = Schema::read(conn, SEEDS)?;
    if schema.is_empty() {
        return Ok(Found::New);
    }
    let id = pg_schema::application_id(conn)?;
    // A cache of an earlier build has no application id, but it has the two tables every cache has.
    let ours = id == APPLICATION_ID || (id == 0 && schema.has_table("entries") && schema.has_table("limits"));
    if !ours {
        return Ok(Found::Foreign { why: "it is not a thumbnail cache".to_string() });
    }
    let cap = match conn.query_row("SELECT cap FROM limits", [], |r| r.get::<_, i64>(0)) {
        Ok(cap) => Some(cap),
        // An earlier layout may have another `limits`: no cap to keep. Damage is not that.
        Err(e) if damage(&e).is_some() => return Err(e),
        Err(_) => None,
    };
    let version = pg_schema::user_version(conn)?;
    let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
    // The owner is read last: an earlier layout may have no `meta` table at all.
    let other_build = |detail: String| Some(Why { reason: StartOverReason::OtherBuild, detail });
    let difference = if version != CACHE_VERSION {
        other_build(format!("layout version {version}, this build's is {CACHE_VERSION}"))
    } else if let Some(difference) = schema.difference(expected) {
        other_build(difference.to_string())
    } else if id != APPLICATION_ID {
        other_build(format!("application id {id}, not a cache's"))
    } else if auto_vacuum != 2 {
        other_build("auto_vacuum is not incremental, so its free pages can't be given back".to_string())
    } else {
        let owner: Option<String> =
            conn.query_row("SELECT value FROM meta WHERE key = 'catalog_id'", [], |r| r.get(0)).optional()?;
        (owner.as_deref() != Some(catalog_id))
            .then(|| Why { reason: StartOverReason::OtherCatalog, detail: "made for another catalog".to_string() })
    };
    Ok(match difference {
        None => Found::Usable,
        Some(why) => Found::Stale { why, cap },
    })
}

/// What [`open_writer`] gives the cache: the writer's connection, why the file was started over and the path of
/// [`Event::StartedOver`] (`None`: it was kept, or it is new), what the day's check of the totals fixed
/// (`heal_if_due`), and the shared hold on the file (`pg_schema::Live`), kept for as long as the connection is.
struct Writing {
    conn: Connection,
    started_over: Option<(Why, PathBuf)>,
    drift: Option<Drift>,
    live: pg_schema::Live,
}

/// One try at the file ([`open_once`]): opened, or found damaged (the reason), with nothing of it held.
enum Attempt {
    Opened(Writing),
    Damaged(Why),
}

/// The writer's connection to a cache of catalog `catalog_id`, with the cap set if one is given ([`Writing`]). A file
/// this build can't use (of another catalog or layout) is deleted with its sidecars and made anew, keeping its cap:
/// that needs the exclusive hold, so no other program or connection has the file open (a start over is refused
/// otherwise, and nothing is deleted). A file that is not a database, or one SQLite finds damaged when it is judged,
/// opened or has its totals checked (`heal_if_due`), is moved aside to `<name>.bad` with its sidecars
/// (`pg_schema::move_aside`, which overwrites nothing) and made anew once; it holds derived data only. A database
/// that is not a thumbnail cache is refused and left alone, and any other failure (busy, no permission, a full disk)
/// stays an error. The lock beside the file is held while it is judged, checked and changed, so another process
/// opening it finds the result.
fn open_writer(path: &Path, catalog_id: &str, cap_bytes: Option<i64>) -> Result<Writing> {
    let _lock = pg_schema::Lock::take(path)
        .with_context(|| format!("{}: its lock file", path.display()))?
        .ok_or_else(|| anyhow!("{}: another process is changing it", path.display()))?;
    let why = match open_once(path, catalog_id, cap_bytes)? {
        Attempt::Opened(writing) => return Ok(writing),
        Attempt::Damaged(why) => why,
    };
    let kept = move_damaged(path, &why.detail)?;
    match open_once(path, catalog_id, cap_bytes)? {
        Attempt::Opened(mut writing) => {
            let detail = format!("{}; the old file is kept as {}", why.detail, kept.display());
            writing.started_over = Some((Why { reason: why.reason, detail }, kept));
            Ok(writing)
        }
        Attempt::Damaged(again) => bail!("{}: {}, even when made anew", path.display(), again.detail),
    }
}

/// One try of [`open_writer`], under its lock: the file judged, started over if this build can't use it, opened, and
/// its totals checked. Damage found on the way is [`Attempt::Damaged`], with every connection to the file closed and
/// every hold on it let go, so the file can be moved aside.
fn open_once(path: &Path, catalog_id: &str, cap_bytes: Option<i64>) -> Result<Attempt> {
    let (usable, why, cap) = match judge(path, catalog_id)? {
        Found::Usable => (true, None, None),
        Found::New => (false, None, None),
        Found::Stale { why, cap } => (false, Some(why), cap),
        Found::Damaged { why } => return Ok(Attempt::Damaged(why)),
        Found::Foreign { why } => bail!("{}: {why}, so it is left alone", path.display()),
    };
    // Taken before anything is removed, and kept until the new file is made: the exclusive hold says nothing else has
    // the file open. Without it, the file is not touched.
    let exclusive = if why.is_some() {
        let held = pg_schema::Live::exclusive(path)
            .with_context(|| format!("{}: its open marker", path.display()))?
            .ok_or_else(|| {
                anyhow!(
                    "{}: made for another catalog or layout, and can't be started over while another program has it \
                     open",
                    path.display()
                )
            })?;
        Some(held)
    } else {
        None
    };
    if why.is_some() {
        // Sidecars first: a `-wal` left beside a new file would be replayed into it.
        for suffix in ["-wal", "-shm", "-journal", ""] {
            // The sidecar's name is the file's own units with the suffix (not its display form, which is lossy).
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            let file = PathBuf::from(name);
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
    }
    let (conn, drift) = match ready(path, catalog_id, cap_bytes.or(cap), !usable) {
        Ok(ready) => ready,
        // The connection is closed (`ready` dropped it) and the exclusive hold goes with this return.
        Err(e) => return damage_in(&e).map(Attempt::Damaged).ok_or(e),
    };
    // The file is made: the exclusive hold gives way to the shared one the connection keeps.
    drop(exclusive);
    let live = hold_shared(path)?;
    let started_over = why.map(|why| (why, path.to_path_buf()));
    Ok(Attempt::Opened(Writing { conn, started_over, drift, live }))
}

/// The writer's connection to the judged file ([`open_file`]), and what the day's check of its totals fixed. A file
/// made here (`made`) is judged again first, and must be this build's.
fn ready(path: &Path, catalog_id: &str, cap_bytes: Option<i64>, made: bool) -> Result<(Connection, Option<Drift>)> {
    let mut conn = open_file(path, catalog_id, cap_bytes)?;
    if made && !matches!(judge(path, catalog_id)?, Found::Usable) {
        bail!("{}: not usable even when made anew", path.display());
    }
    let drift = heal_if_due(&mut conn)?;
    Ok((conn, drift))
}

/// Move the damaged cache at `path` aside, with its sidecars, to `<name>.bad` (or `.bad.1`, and so on): its new path.
/// Only with the exclusive hold, so no other program or connection has it open; refused otherwise, with nothing moved.
fn move_damaged(path: &Path, why: &str) -> Result<PathBuf> {
    let _exclusive =
        pg_schema::Live::exclusive(path).with_context(|| format!("{}: its open marker", path.display()))?.ok_or_else(
            || anyhow!("{}: {why}, and can't be moved aside while another program has it open", path.display()),
        )?;
    pg_schema::move_aside(path, ".bad").with_context(|| format!("{}: {why}, and can't be moved aside", path.display()))
}

/// The shared hold a cache keeps while it is open (`pg_schema::Live`): it waits for a start over in progress to finish.
fn hold_shared(path: &Path) -> Result<pg_schema::Live> {
    pg_schema::Live::shared(path)
        .with_context(|| format!("{}: its open marker", path.display()))?
        .ok_or_else(|| anyhow!("{}: another process is starting it over", path.display()))
}

/// A connection to the cache file, made with its layout if it is new (for catalog `catalog_id`), and the cap set if one
/// is given and the file has the table for it. The file was judged by [`judge`] first, so only a new one is made here.
fn open_file(path: &Path, catalog_id: &str, cap_bytes: Option<i64>) -> Result<Connection> {
    let mut conn = Connection::open(sqlite_path(path)?)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    // auto_vacuum must be set before the first table exists. Setting it writes page 1 even when the file already has
    // it, so it is set only when it differs: a cache judged usable has it, and its open must write nothing before the
    // day's check (which may find the file damaged and move it aside as it was).
    let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
    if auto_vacuum != 2 {
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
    }
    conn.execute_batch(CONNECTION_PRAGMAS)?;
    // The writer's pages: a 200-thumbnail batch fits in them, so its transaction doesn't go to the file for each.
    conn.execute_batch("PRAGMA cache_size = -16384;")?;
    let empty = |c: &Connection| -> Result<bool> { Ok(Schema::read(c, &[])?.is_empty()) };
    if empty(&conn)? {
        // The setting takes effect only in a file SQLite has not written: an empty file that was written (a
        // user_version) needs a VACUUM to take it.
        let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
        if auto_vacuum != 2 {
            conn.execute_batch("VACUUM;")?;
        }
        // Another process may be creating it too: decide inside the write lock.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if empty(&tx)? {
            let dir = path.parent().unwrap_or(path);
            make_layout(&tx, catalog_id, cap_bytes.unwrap_or_else(|| default_cap_bytes(dir)))?;
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
    let conn = Connection::open(sqlite_path(cache)?)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    // The whole entries table fits in the page cache (~40 bytes an entry); the sort stays in memory.
    conn.execute_batch("PRAGMA query_only = ON; PRAGMA temp_store = MEMORY; PRAGMA cache_size = -131072;")?;
    conn.execute("ATTACH DATABASE ?1 AS cat", [attach_bytes(catalog)?])?;
    let tx = conn.unchecked_transaction()?;
    let seq: i64 = tx.query_row("SELECT seq FROM cat.thumb_clock", [], |r| r.get(0))?;
    let (checked, evictions): (Option<i64>, i64) = tx
        .query_row("SELECT seq, evictions FROM checked WHERE height = ?1", [height], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?
        .unwrap_or((None, 0));
    let since = checked.filter(|&c| c <= seq);
    let values = match since {
        Some(c) => vec![i64::from(height), c],
        None => vec![i64::from(height)],
    };
    // The folder ids' table is sized up front. The items are not: counting them first would scan the view.
    let mut work = WorkList::with_capacity(0, pg_catalog::max_folder_id(&tx, "cat.")?);
    {
        let mut stmt = tx.prepare(pending_sql(since.is_some()))?;
        let mut rows = stmt.query(rusqlite::params_from_iter(values))?;
        while let Some(r) = rows.next()? {
            let name = r.get_ref(2)?.as_blob()?;
            let path =
                work.file(r.get(1)?, name, |arena| Ok::<_, rusqlite::Error>(arena.push(r.get_ref(7)?.as_blob()?)))?;
            work.items.push(ThumbWork {
                id: FileId::new(r.get(0)?),
                path,
                source: Source { mtime: r.get(3)?, rotation: Turns::new(r.get(4)?) },
                size: r.get(8)?,
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
                "SELECT f.id, f.folder_id, f.name, f.mtime, f.rotation, CASE WHEN f.kind = 'video' THEN coalesce(f.poster_ms, 0) END, f.seq, f.folder, f.size
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
        if let Some((_, conn)) = readers.iter().find(|(p, _)| p == path) {
            return f(conn);
        }
        let conn = Connection::open(sqlite_path(path)?)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.execute_batch(CONNECTION_PRAGMAS)?;
        conn.execute_batch("PRAGMA query_only = ON;")?;
        let result = f(&conn);
        readers.push((path.to_path_buf(), conn));
        result
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

/// The writer's body: each queued op in order, and the touches flushed when they are due, until the queue is closed
/// and empty. Then the touches are flushed and the WAL merged.
fn write_loop(mut conn: Connection, inbox: &Inbox, events: &Bus<Event>, waker: &Waker<'_>) {
    // However the loop ends, the queue closes: no caller is left waiting for an answer that won't come.
    let _closing = Closing(inbox);
    let mut touches = Touches::default();
    loop {
        // Read before the queue: `close` closes the queue before it quits, so a quit seen here means nothing more
        // will be queued, and an empty queue after it is the end.
        let quitting = waker.quitting();
        match inbox.next() {
            Some(op) => apply(op, &mut conn, &mut touches, events),
            None if quitting => break,
            None => {
                if waker.wait(touches.due()) == Woke::Idle {
                    touches_failure(touches.flush(&mut conn), events);
                }
            }
        }
    }
    touches_failure(touches.flush(&mut conn), events);
    // The writer's last act: the WAL merged into the file, so the file is whole even if the process ends before the
    // readers close (the last connection to close deletes the WAL, and a reader may still be open).
    if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)") {
        events.publish(Event::WalNotMerged { error: format!("{e:#}") });
    }
}

/// Does one op of the writer's queue.
fn apply(op: Op, conn: &mut Connection, touches: &mut Touches, events: &Bus<Event>) {
    match op {
        Op::Touch(id) => {
            touches.ids.insert(id);
            touches.since.get_or_insert_with(Instant::now);
        }
        Op::Put { thumbs, keep_height, reply } => {
            let stored = put(conn, &thumbs, keep_height, touches);
            let _ = reply.send((stored, thumbs));
        }
        Op::PutOne(thumb) => {
            if let Err(e) = put(conn, std::slice::from_ref(&thumb), None, touches) {
                events.publish(Event::NotStored { error: format!("{e:#}") });
            }
        }
        Op::Retag { file_id, old_mtime, new_mtime, reply } => {
            let done = conn
                .execute(
                    "UPDATE entries SET mtime = ?3 WHERE file_id = ?1 AND mtime = ?2",
                    params![file_id.get(), old_mtime, new_mtime],
                )
                .map(drop)
                .map_err(anyhow::Error::from);
            let _ = reply.send(done);
        }
        Op::Forget { file_id, reply } => {
            let done = conn
                .execute("DELETE FROM entries WHERE file_id = ?1", params![file_id.get()])
                .map(drop)
                .map_err(anyhow::Error::from);
            let _ = reply.send(done);
        }
        Op::Checked { snapshot, through, reply } => {
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
        Op::Heal(reply) => {
            let _ = reply.send(heal(conn));
        }
        Op::Shrink(reply) => {
            let _ = reply.send(shrink(conn));
        }
        Op::Flush(reply) => {
            touches_failure(touches.flush(conn), events);
            let _ = reply.send(());
        }
    }
}

/// Touches that could not be written are an event; they stay queued for the next try.
fn touches_failure(written: Result<()>, events: &Bus<Event>) {
    if let Err(e) = written {
        events.publish(Event::TouchesNotWritten { error: format!("{e:#}") });
    }
}

/// Store `thumbs` in one IMMEDIATE transaction: drop the entries they replace, read the total and the cap, evict
/// what's needed, then write. Reading and writing in one write transaction is what keeps two writers (the app and
/// `gallery thumbs`) from both seeing room for their batch, and from failing with SQLITE_BUSY_SNAPSHOT. Dropping
/// the replaced entries first keeps eviction from counting them twice (replaced, then evicted).
fn put(conn: &mut Connection, thumbs: &[Thumb], keep_height: Option<u32>, touches: &mut Touches) -> Result<bool> {
    let incoming: i64 = thumbs.iter().map(|t| i64::try_from(t.data.len()).unwrap_or(i64::MAX)).sum();
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
            replaced.execute(params![t.file_id.get(), t.height])?;
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
                params![
                    file_id.get(),
                    height,
                    source.mtime,
                    source.rotation.get(),
                    FORMAT_JPEG,
                    i64::try_from(jpeg.len()).unwrap_or(i64::MAX)
                ],
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

/// `heal`, unless it was done within `HEAL_INTERVAL` (`meta.healed`, the Unix second it last was): the open's check.
fn heal_if_due(conn: &mut Connection) -> Result<Option<Drift>> {
    let now = unix_now();
    let last: i64 =
        conn.query_row("SELECT coalesce((SELECT value FROM meta WHERE key = 'healed'), 0)", [], |r| r.get(0))?;
    if now.saturating_sub(last) < i64::try_from(HEAL_INTERVAL.as_secs()).unwrap_or(i64::MAX) {
        return Ok(None);
    }
    let fixed = heal(conn)?;
    conn.execute("INSERT OR REPLACE INTO meta(key, value) VALUES ('healed', ?1)", [now])?;
    Ok(fixed)
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
    use pg_catalog::fixture::state;
    use pg_model::ids::FolderId;

    #[test]
    fn the_cap_parser_refuses_what_would_be_no_cache() {
        assert_eq!(parse_cap_gb("2"), Ok(2_000_000_000));
        assert_eq!(parse_cap_gb(" 0.5 "), Ok(500_000_000));
        for text in ["0", "-1", "NaN", "inf", "1e-12", "1e12", "lots"] {
            assert!(parse_cap_gb(text).is_err(), "{text} is refused");
        }
    }
    use pg_testkit::events::Quiet;
    use pg_testkit::test_folder;

    /// Where this module's tests write (`PG_TEST_DIR` when it is set).
    const SCRATCH: &str = "scratch_thumbs";

    /// Closing the cache leaves no WAL or index file: the writer merges the WAL, and this thread's reader (opened by
    /// the read before) closes with it, so the last connection deletes the files. A reader left open keeps them.
    #[test]
    fn close_merges_the_wal_and_leaves_no_files() {
        let dir = test_folder(SCRATCH, "close-wal").unwrap();
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
        Thumb {
            file_id: FileId::new(file_id),
            height,
            source: Source { mtime, rotation: Turns::new(0) },
            data: vec![file_id.to_le_bytes()[0]; bytes],
        }
    }

    /// The catalog the test caches belong to.
    const CATALOG_ID: &str = "test-catalog";

    /// A cache for a test, with its events watched: an event the test doesn't expect fails it. The cache is declared
    /// before the watch, so it closes first.
    struct Opened {
        cache: ThumbCache,
        _events: Quiet<Event>,
    }

    impl std::ops::Deref for Opened {
        type Target = ThumbCache;
        fn deref(&self) -> &ThumbCache {
            &self.cache
        }
    }

    fn open_quiet(dir: &Path, catalog_id: &str, cap_bytes: Option<i64>) -> Opened {
        let (events, watch) = Quiet::bus();
        let cache = ThumbCache::open(dir, catalog_id, cap_bytes, events).unwrap();
        Opened { cache, _events: watch }
    }

    /// Opens a cache that starts over: returns it with the reason, the path and the detail of the one event the open
    /// publishes.
    fn open_started_over(
        dir: &Path,
        catalog_id: &str,
        cap_bytes: Option<i64>,
    ) -> (ThumbCache, StartOverReason, PathBuf, String) {
        let bus = Arc::new(Bus::new(8));
        let events = bus.subscribe();
        let cache = ThumbCache::open(dir, catalog_id, cap_bytes, bus).unwrap();
        let mut taken = events.take().items.into_iter();
        let first = taken.next();
        assert!(taken.next().is_none(), "one event expected");
        match first {
            Some(Event::StartedOver { reason, path, detail }) => (cache, reason, path, detail),
            other => panic!("a start-over expected, got {other:?}"),
        }
    }

    fn open(dir: &Path, cap_bytes: i64) -> Opened {
        open_quiet(dir, CATALOG_ID, Some(cap_bytes))
    }

    fn src(mtime: i64) -> Source {
        Source { mtime, rotation: Turns::new(0) }
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
        let dir = test_folder(SCRATCH, "nearest").unwrap();
        let cache = open(&dir, 1 << 30);
        assert!(cache.put_batch(&mut vec![thumb(1, 100, 5, 10), thumb(1, 300, 5, 30)], None).unwrap());
        // The smallest at least as high, else the largest below.
        assert_eq!(cache.get_nearest(FileId::new(1), 200, src(5)).unwrap().unwrap().0, 300);
        assert_eq!(cache.get_nearest(FileId::new(1), 90, src(5)).unwrap().unwrap().0, 100);
        assert_eq!(cache.get_nearest(FileId::new(1), 400, src(5)).unwrap().unwrap().0, 300);
        // Made from another version of the file, or another rotation: a miss.
        assert!(cache.get_nearest(FileId::new(1), 100, src(6)).unwrap().is_none());
        assert!(
            cache.get_nearest(FileId::new(1), 100, Source { mtime: 5, rotation: Turns::new(1) }).unwrap().is_none()
        );
        assert!(cache.get_nearest(FileId::new(2), 100, src(5)).unwrap().is_none());
        // An entry whose JPEG is gone (another process deleting it right now) is a miss too.
        let c = Connection::open(cache_path(&dir)).unwrap();
        c.execute("DELETE FROM data WHERE id = (SELECT id FROM entries WHERE height = 100)", []).unwrap();
        assert!(cache.get_nearest(FileId::new(1), 90, src(5)).unwrap().is_none());
        assert_eq!(cache.get_nearest(FileId::new(1), 300, src(5)).unwrap().unwrap().0, 300);
    }

    /// A batch comes back as it went, its thumbnails' buffers with it, for the next batch; one put alone is stored
    /// as a batch of one is. A hit is read into the caller's buffer, replacing what it held.
    #[test]
    fn batches_come_back_and_hits_are_read_into_the_callers_buffer() {
        let dir = test_folder(SCRATCH, "back").unwrap();
        let cache = open(&dir, 1 << 30);
        let mut batch = vec![thumb(1, 100, 5, 10), thumb(2, 100, 5, 20)];
        assert!(cache.put_batch(&mut batch, None).unwrap());
        assert_eq!(
            batch.iter().map(|t| (t.file_id, t.data.len())).collect::<Vec<_>>(),
            [(FileId::new(1), 10), (FileId::new(2), 20)]
        );
        cache.put(thumb(3, 100, 5, 30));
        cache.flush();
        let mut jpeg = vec![9; 100];
        assert_eq!(cache.get_nearest_into(FileId::new(3), 100, src(5), &mut jpeg).unwrap(), Some(100));
        assert_eq!(jpeg, vec![3u8; 30]);
        // A miss leaves it as it was.
        assert_eq!(cache.get_nearest_into(FileId::new(4), 100, src(5), &mut jpeg).unwrap(), None);
        assert_eq!(jpeg, vec![3u8; 30]);
        assert_eq!(cache.totals().unwrap(), (60, 3));
    }

    #[test]
    fn replacing_keeps_each_jpeg_with_its_file() {
        let dir = test_folder(SCRATCH, "replace").unwrap();
        let cache = open(&dir, 1 << 30);
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap();
        cache.put_batch(&mut vec![thumb(2, 100, 5, 20)], None).unwrap();
        // Re-put file 1 after file 2 was inserted: an upsert that updates. With last_insert_rowid() this put file 1's
        // JPEG under file 2.
        cache.put_batch(&mut vec![thumb(1, 100, 6, 15)], None).unwrap();
        assert_eq!(cache.get_nearest(FileId::new(1), 100, src(6)).unwrap().unwrap().1, vec![1u8; 15]);
        assert_eq!(cache.get_nearest(FileId::new(2), 100, src(5)).unwrap().unwrap().1, vec![2u8; 20]);
        assert_eq!(cache.totals().unwrap(), (35, 2));
        let (kept, actual) = totals_and_sum(&dir);
        assert_eq!(kept, actual);
    }

    #[test]
    fn eviction() {
        let dir = test_folder(SCRATCH, "evict").unwrap();
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
        assert!(cache.get_nearest(FileId::new(0), 50, src(1)).unwrap().is_none());
        assert!(cache.get_nearest(FileId::new(1), 50, src(1)).unwrap().is_none());
        assert!(cache.get_nearest(FileId::new(2), 50, src(1)).unwrap().is_some());
        // On demand, any height goes; a batch bigger than the cap stores nothing.
        cache.put(thumb(21, 80, 1, 100));
        cache.flush();
        assert!(cache.get_nearest(FileId::new(21), 80, src(1)).unwrap().is_some());
        assert!(!cache.put_batch(&mut vec![thumb(30, 80, 1, 2000)], None).unwrap());
        let (kept, actual) = totals_and_sum(&dir);
        assert_eq!(kept, actual);
        assert!(kept.0 <= 1000);
    }

    #[test]
    fn far_over_a_lowered_cap_comes_down() {
        let dir = test_folder(SCRATCH, "lowered").unwrap();
        let cache = open(&dir, 1 << 30);
        assert!(cache.put_batch(&mut (0..40).map(|i| thumb(i, 50, 1, 100)).collect(), None).unwrap());
        drop(cache);
        // 4000 bytes against a cap of 500: brought down in slices (1000 bytes in tests), then the put.
        let cache = open(&dir, 500);
        cache.put(thumb(100, 80, 1, 100));
        cache.flush();
        assert!(cache.get_nearest(FileId::new(100), 80, src(1)).unwrap().is_some());
        let (kept, actual) = totals_and_sum(&dir);
        assert_eq!(kept, actual);
        assert!(kept.0 <= 500, "{kept:?}");
        // The task's own height is spared: nothing else to evict, so no room.
        assert!(!cache.put_batch(&mut vec![thumb(200, 80, 1, 2000)], Some(80)).unwrap());
    }

    #[test]
    fn one_cap_for_every_writer() {
        let dir = test_folder(SCRATCH, "cap").unwrap();
        assert_eq!(ThumbCache::read_usage(&dir).unwrap(), None);
        let first = open_quiet(&dir, CATALOG_ID, None);
        assert_eq!(first.cap().unwrap(), default_cap_bytes(&dir), "a new cache");
        assert!(first.put_batch(&mut (0..10).map(|i| thumb(i, 50, 1, 100)).collect(), None).unwrap());
        // Another process sets a lower cap: the first one keeps to it at its next write.
        let second = open(&dir, 500);
        assert_eq!(first.cap().unwrap(), 500);
        assert!(first.put_batch(&mut vec![thumb(20, 80, 1, 100)], Some(80)).unwrap());
        assert!(first.totals().unwrap().0 <= 500);
        // Opened without one, the cap stays.
        drop(second);
        drop(open_quiet(&dir, CATALOG_ID, None));
        let usage = ThumbCache::read_usage(&dir).unwrap().unwrap();
        assert_eq!((usage.cap, usage.bytes, usage.count), (500, first.totals().unwrap().0, first.totals().unwrap().1));
        assert!(ThumbCache::open(&dir, CATALOG_ID, Some(-1), Arc::new(Bus::new(1))).is_err());
    }

    #[test]
    fn shrink_gives_pages_back() {
        let dir = test_folder(SCRATCH, "shrink").unwrap();
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
        let dir = test_folder(SCRATCH, "touch").unwrap();
        let cache = open(&dir, 1 << 30);
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap();
        let c = Connection::open(cache_path(&dir)).unwrap();
        let last_used = || -> i64 { c.query_row("SELECT last_used FROM entries", [], |r| r.get(0)).unwrap() };
        // Shown an hour ago or more: touched (when the writer flushes); recently: left alone.
        c.execute("UPDATE entries SET last_used = 1000", []).unwrap();
        cache.get_nearest(FileId::new(1), 100, src(5)).unwrap().unwrap();
        assert_eq!(last_used(), 1000, "a touch waits for the flush");
        cache.flush();
        let touched = last_used();
        assert!(touched > unix_now() - 5);
        c.execute("UPDATE entries SET last_used = ?1", [touched - 10]).unwrap();
        cache.get_nearest(FileId::new(1), 100, src(5)).unwrap().unwrap();
        cache.flush();
        assert_eq!(last_used(), touched - 10);
        // Closing writes what's waiting.
        c.execute("UPDATE entries SET last_used = 1000", []).unwrap();
        cache.get_nearest(FileId::new(1), 100, src(5)).unwrap().unwrap();
        cache.close();
        assert!(last_used() > 1000);
    }

    #[test]
    fn retag_follows_the_file() {
        let dir = test_folder(SCRATCH, "retag").unwrap();
        let cache = open(&dir, 1 << 30);
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10), thumb(1, 200, 5, 10), thumb(2, 100, 5, 10)], None).unwrap();
        cache.retag(FileId::new(1), 5, 9).unwrap();
        assert!(cache.get_nearest(FileId::new(1), 100, src(9)).unwrap().is_some());
        assert!(cache.get_nearest(FileId::new(1), 200, src(9)).unwrap().is_some());
        assert!(cache.get_nearest(FileId::new(2), 100, src(5)).unwrap().is_some());
        // Only from the mtime it had.
        cache.retag(FileId::new(2), 4, 9).unwrap();
        assert!(cache.get_nearest(FileId::new(2), 100, src(5)).unwrap().is_some());
    }

    #[test]
    fn reopened_it_keeps_what_it_held() {
        let dir = test_folder(SCRATCH, "reopen").unwrap();
        let cache = open(&dir, 1 << 30);
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap();
        drop(cache);
        let cache = open(&dir, 1 << 30);
        assert!(cache.get_nearest(FileId::new(1), 100, src(5)).unwrap().is_some());
        assert_eq!(cache.totals().unwrap(), (10, 1));
    }

    #[test]
    fn another_layout_version_starts_over() {
        let dir = test_folder(SCRATCH, "layout").unwrap();
        Connection::open(cache_path(&dir)).unwrap().execute_batch("PRAGMA user_version = 7;").unwrap();
        let cache = open_quiet(&dir, CATALOG_ID, None);
        assert_eq!((cache.totals().unwrap(), cache.cap().unwrap()), ((0, 0), default_cap_bytes(&dir)));
    }

    #[test]
    fn a_new_cache_has_its_application_id_and_incremental_auto_vacuum() {
        let dir = test_folder(SCRATCH, "fresh-file").unwrap();
        drop(open_quiet(&dir, CATALOG_ID, None));
        let conn = Connection::open(cache_path(&dir)).unwrap();
        assert_eq!(pg_schema::application_id(&conn).unwrap(), APPLICATION_ID);
        assert_eq!(conn.query_row("PRAGMA auto_vacuum", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        drop(conn);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_edited_table_starts_over_with_its_cap_kept() {
        let dir = test_folder(SCRATCH, "edited-cache").unwrap();
        drop(open_quiet(&dir, CATALOG_ID, Some(12345)));
        Connection::open(cache_path(&dir))
            .unwrap()
            .execute_batch("ALTER TABLE entries ADD COLUMN extra INTEGER;")
            .unwrap();
        let (cache, reason, path, why) = open_started_over(&dir, CATALOG_ID, None);
        assert!(why.contains("entries"), "{why}");
        assert_eq!((reason, path), (StartOverReason::OtherBuild, cache_path(&dir)));
        assert_eq!((cache.totals().unwrap(), cache.cap().unwrap()), ((0, 0), 12345));
        drop(cache);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A file with this cache's tables but another application id is not ours to delete: it is refused, as it is.
    #[test]
    fn another_applications_file_is_refused_and_left_alone() {
        let dir = test_folder(SCRATCH, "app-id-cache").unwrap();
        drop(open_quiet(&dir, CATALOG_ID, Some(999)));
        Connection::open(cache_path(&dir)).unwrap().pragma_update(None, "application_id", 9).unwrap();
        let e = ThumbCache::open(&dir, CATALOG_ID, None, Arc::new(Bus::new(1))).err().expect("refused");
        assert!(format!("{e:#}").contains("left alone"), "{e:#}");
        assert_eq!(pg_schema::application_id(&Connection::open(cache_path(&dir)).unwrap()).unwrap(), 9);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A database that is not a cache (tables of its own, no cache's application id) is refused and left as it was.
    #[test]
    fn a_database_that_is_not_a_cache_is_refused_and_left_as_it_was() {
        let dir = test_folder(SCRATCH, "foreign-db").unwrap();
        Connection::open(cache_path(&dir)).unwrap().execute_batch("CREATE TABLE notes(text TEXT);").unwrap();
        let before = std::fs::read(cache_path(&dir)).unwrap();
        let e = ThumbCache::open(&dir, CATALOG_ID, None, Arc::new(Bus::new(1))).err().expect("refused");
        assert!(format!("{e:#}").contains("left alone"), "{e:#}");
        assert_eq!(std::fs::read(cache_path(&dir)).unwrap(), before);
        assert!(!bad_path(&dir).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Where a damaged cache in `dir` is moved aside to.
    fn bad_path(dir: &Path) -> PathBuf {
        dir.join("thumbs.sqlite.bad")
    }

    /// Opens the cache in `dir`, whose file holds `damaged`: it is moved aside to `.bad` whole, a new cache takes the
    /// name, and the one event says why and where the old file went: the reason and the detail.
    fn opens_over_damage(dir: &Path, damaged: &[u8]) -> (StartOverReason, String) {
        let (cache, reason, path, why) = open_started_over(dir, CATALOG_ID, None);
        assert!(why.contains("thumbs.sqlite.bad"), "{why}");
        assert_eq!(path, bad_path(dir), "the event names where the old file went");
        assert!(matches!(reason, StartOverReason::Damaged | StartOverReason::NotADatabase), "{reason:?}");
        assert_eq!(std::fs::read(bad_path(dir)).unwrap(), damaged, "the old bytes are kept");
        assert_eq!(cache.totals().unwrap(), (0, 0));
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap();
        assert!(cache.get_nearest(FileId::new(1), 100, src(5)).unwrap().is_some());
        drop(cache);
        (reason, why)
    }

    /// A file that is not a database at all is moved aside and made anew: the cache holds derived data only.
    #[test]
    fn a_file_that_is_not_a_database_is_moved_aside() {
        let dir = test_folder(SCRATCH, "foreign-cache").unwrap();
        std::fs::write(cache_path(&dir), vec![b'x'; 8192]).unwrap();
        let (reason, why) = opens_over_damage(&dir, &[b'x'; 8192]);
        assert!(why.contains("not a database"), "{why}");
        assert_eq!(reason, StartOverReason::NotADatabase);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A cache of a few hundred entries, closed (its WAL merged), with the day's check of its totals due at the next
    /// open: its bytes and page size.
    fn closed_cache_due_a_check(dir: &Path) -> (Vec<u8>, usize) {
        let cache = open_quiet(dir, CATALOG_ID, None);
        for i in 0..20 {
            let mut batch: Vec<Thumb> = (0..20).map(|j| thumb(i * 20 + j, 100, 5, 2000)).collect();
            assert!(cache.put_batch(&mut batch, None).unwrap());
        }
        drop(cache);
        let conn = Connection::open(cache_path(dir)).unwrap();
        conn.execute("UPDATE meta SET value = 0 WHERE key = 'healed'", []).unwrap();
        let page: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap();
        drop(conn);
        assert!(!dir.join("thumbs.sqlite-wal").exists(), "the WAL is merged");
        (std::fs::read(cache_path(dir)).unwrap(), usize::try_from(page).unwrap())
    }

    /// A real cache whose body is garbage after its header: moved aside and made anew.
    #[test]
    fn a_cache_with_a_garbage_body_is_moved_aside() {
        let dir = test_folder(SCRATCH, "garbage-body").unwrap();
        let (mut bytes, _) = closed_cache_due_a_check(&dir);
        for (i, b) in bytes.iter_mut().enumerate().skip(100) {
            *b = u8::try_from(i % 251).unwrap() ^ 0xA5;
        }
        std::fs::write(cache_path(&dir), &bytes).unwrap();
        opens_over_damage(&dir, &bytes);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A real cache cut to half its pages (a disk full, a copy cut short): moved aside and made anew.
    #[test]
    fn a_truncated_cache_is_moved_aside() {
        let dir = test_folder(SCRATCH, "truncated").unwrap();
        let (bytes, page) = closed_cache_due_a_check(&dir);
        let cut = &bytes[..bytes.len() / page / 2 * page];
        std::fs::write(cache_path(&dir), cut).unwrap();
        opens_over_damage(&dir, cut);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Damage among the entries, past the pages the open reads, is found by the day's check of the totals
    /// (`meta.healed` put back, or the open doesn't check): that takes the same path, under the same lock.
    #[test]
    fn damage_found_by_the_days_check_is_moved_aside() {
        let dir = test_folder(SCRATCH, "heal-damage").unwrap();
        let (mut bytes, page) = closed_cache_due_a_check(&dir);
        // The first pages hold the header, the schema and the tables' roots (the open reads those); the entries' leaves
        // and the JPEGs are past them.
        let keep = 16 * page;
        assert!(bytes.len() > 4 * keep, "the cache is too small to damage past its roots: {} bytes", bytes.len());
        for b in &mut bytes[keep..] {
            *b = 0xEE;
        }
        std::fs::write(cache_path(&dir), &bytes).unwrap();
        let (reason, why) = opens_over_damage(&dir, &bytes);
        assert!(why.contains("damaged"), "{why}");
        assert_eq!(reason, StartOverReason::Damaged);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A damaged cache that another program has open is not moved: the open fails, and the file stays as it was.
    #[test]
    fn a_damaged_cache_another_program_holds_is_not_moved() {
        let dir = test_folder(SCRATCH, "damaged-held").unwrap();
        std::fs::write(cache_path(&dir), vec![b'x'; 8192]).unwrap();
        let held = pg_schema::Live::shared(&cache_path(&dir)).unwrap().unwrap();
        let e = ThumbCache::open(&dir, CATALOG_ID, None, Arc::new(Bus::new(1))).err().expect("refused");
        assert!(format!("{e:#}").contains("another program has it open"), "{e:#}");
        assert_eq!(std::fs::read(cache_path(&dir)).unwrap(), vec![b'x'; 8192]);
        assert!(!bad_path(&dir).exists());
        drop(held);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn two_writers_share_one_file() {
        let dir = test_folder(SCRATCH, "two").unwrap();
        let a = open(&dir, 1 << 30);
        let b = open(&dir, 1 << 30);
        std::thread::scope(|s| {
            for (cache, base) in [(&a, 0), (&b, 1000)] {
                s.spawn(move || {
                    for i in 0..200 {
                        assert!(
                            cache
                                .put_batch(
                                    &mut vec![thumb(base + i, 100, 1, 10 + usize::try_from(i % 7).unwrap())],
                                    None
                                )
                                .unwrap()
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
        let dir = test_folder(SCRATCH, "sequence").unwrap();
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
            let (file, height, size) = (
                i64::try_from(next(60)).unwrap(),
                [64, 128, 256][usize::try_from(next(3)).unwrap()],
                20 + usize::try_from(next(400)).unwrap(),
            );
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
                _ => cache.retag(FileId::new(file), 1, 1).unwrap(),
            }
        }
        cache.flush();
        let (kept, actual) = totals_and_sum(&dir);
        assert_eq!(kept, actual);
        assert!(kept.0 <= 5_000);
    }

    #[test]
    fn heal_fixes_drifted_totals() {
        let dir = test_folder(SCRATCH, "heal").unwrap();
        let cache = open(&dir, 1 << 30);
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10), thumb(2, 100, 5, 20)], None).unwrap();
        Connection::open(cache_path(&dir)).unwrap().execute("UPDATE totals SET bytes = 999, count = 7", []).unwrap();
        assert_eq!(cache.heal().unwrap(), Some(((999, 7), (30, 2))));
        assert_eq!(cache.totals().unwrap(), (30, 2));
        assert_eq!(cache.heal().unwrap(), None);
    }

    #[test]
    fn a_closed_cache_takes_no_more_writes_and_still_reads() {
        let dir = test_folder(SCRATCH, "closed").unwrap();
        let cache = open(&dir, 1 << 20);
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap();
        cache.close();
        let refused = cache.put_batch(&mut vec![thumb(2, 100, 5, 20)], None).unwrap_err();
        assert!(refused.to_string().contains("closed"), "{refused:#}");
        assert_eq!(cache.totals().unwrap(), (10, 1));
    }

    #[test]
    fn a_fix_made_at_open_is_published_before_open_returns() {
        let dir = test_folder(SCRATCH, "fix-event").unwrap();
        drop(open(&dir, 1 << 30));
        // A day on (the last check is put back a day), so this open checks the totals.
        let file = Connection::open(cache_path(&dir)).unwrap();
        file.execute("UPDATE totals SET bytes = 999, count = 7", []).unwrap();
        file.execute("UPDATE meta SET value = 0 WHERE key = 'healed'", []).unwrap();
        let bus = Arc::new(Bus::new(4));
        let events = bus.subscribe();
        let cache = ThumbCache::open(&dir, CATALOG_ID, Some(1 << 30), bus).unwrap();
        assert_eq!(events.take().items, vec![Event::TotalsFixed { kept: (999, 7), actual: (0, 0) }]);
        drop(cache);
    }

    #[test]
    fn the_totals_are_checked_at_most_once_a_day() {
        let dir = test_folder(SCRATCH, "heal-daily").unwrap();
        drop(open(&dir, 1 << 30));
        Connection::open(cache_path(&dir)).unwrap().execute("UPDATE totals SET bytes = 999, count = 7", []).unwrap();
        // Within the day of the last check, an open doesn't check: the drift stays until the next day's open.
        let bus = Arc::new(Bus::new(4));
        let events = bus.subscribe();
        drop(ThumbCache::open(&dir, CATALOG_ID, Some(1 << 30), bus).unwrap());
        assert!(events.take().items.is_empty());
        let file = Connection::open(cache_path(&dir)).unwrap();
        assert_eq!(file.query_row("SELECT count FROM totals", [], |r| r.get::<_, i64>(0)).unwrap(), 7);
    }

    /// A catalog in `dir` with four analyzed files: 1–3 present (3 turned once in the catalog), 4 missing.
    fn catalog_of_four(dir: &Path) -> (PathBuf, pg_catalog::Catalog) {
        let catalog = dir.join("catalog.sqlite");
        let rules = pg_model::rules::PathRules::Windows;
        let c = pg_catalog::Catalog::prepare(&catalog, pg_catalog::fixture::versions(), rules).unwrap();
        state::add_scan(&c, 1, "D:\\p").unwrap();
        state::add_folder(&c, 1, "D:\\p").unwrap();
        use pg_catalog::fixture::{File, insert};
        insert(
            &c,
            &[
                File { file_ref: 30, ..File::new(FileId::new(1), FolderId::new(1), "a.jpg", 1, 5) },
                File { file_ref: 10, ..File::new(FileId::new(2), FolderId::new(1), "b.jpg", 1, 5) },
                File {
                    file_ref: 20,
                    rotation: Turns::new(1),
                    ..File::new(FileId::new(3), FolderId::new(1), "c.jpg", 1, 5)
                },
                File { file_ref: 40, missing: true, ..File::new(FileId::new(4), FolderId::new(1), "d.jpg", 1, 5) },
            ],
        )
        .unwrap();
        state::stub_analysis_of_all(&c).unwrap();
        (catalog, c)
    }

    fn ids(p: &Pending) -> Vec<i64> {
        p.work.items.iter().map(|w| w.id.get()).collect()
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
        let dir = test_folder(SCRATCH, "pending").unwrap();
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
        let dir = test_folder(SCRATCH, "watermark").unwrap();
        let (catalog, c) = catalog_of_four(&dir);
        let cache = open(&dir, 1 << 30);
        let first = cache.pending(&catalog, 100).unwrap();
        assert_eq!(ids(&first), vec![2, 3, 1]);
        // Done, but file 2 couldn't be made: it stays in the work.
        assert!(cache.put_batch(&mut made(&first, |w| w.id != FileId::new(2), 10), Some(100)).unwrap());
        let failed = first.work.items.iter().find(|w| w.id == FileId::new(2)).unwrap().seq;
        cache.checked(&first.snapshot, Some(failed)).unwrap();
        let second = cache.pending(&catalog, 100).unwrap();
        assert!(second.snapshot.since_checked);
        assert_eq!(ids(&second), vec![2]);
        cache.checked(&second.snapshot, None).unwrap();
        let third = cache.pending(&catalog, 100).unwrap();
        assert!(third.snapshot.since_checked && third.work.is_empty(), "{:?}", ids(&third));
        // A change in the catalog after it: only that file. (The analysis follows, as a metadata write's does.)
        state::change_mtime(&c, FileId::new(1), 6).unwrap();
        assert_eq!(ids(&cache.pending(&catalog, 100).unwrap()), vec![1]);
        // A catalog behind the cache (restored from a copy): every file again.
        state::set_meta_seq(&c, 1).unwrap();
        let behind = cache.pending(&catalog, 100).unwrap();
        assert!(!behind.snapshot.since_checked);
        assert_eq!(ids(&behind), vec![2, 1]);
    }

    /// On Windows, a cache in a folder whose name has no UTF-8 form is made, and opened again through the folder's alias
    /// (`pg_platform::sqlite`): the file is in the folder itself.
    #[cfg(windows)]
    #[test]
    fn a_cache_in_a_folder_without_a_utf8_name_is_opened_through_an_alias() {
        let dir = test_folder(SCRATCH, "odd-folder").unwrap();
        let folder = dir.join(pg_platform::names::path_buf(b"cache\xED\xA0\x80"));
        std::fs::create_dir_all(&folder).unwrap();
        drop(open(&folder, 1 << 30));
        assert!(cache_path(&folder).exists(), "the cache file is in the folder");
        drop(open(&folder, 1 << 30));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_pass_after_a_complete_one_reads_the_changed_files() {
        let dir = test_folder(SCRATCH, "seq-plan").unwrap();
        let (catalog, _c) = catalog_of_four(&dir);
        drop(open(&dir, 1 << 30));
        let conn = Connection::open(cache_path(&dir)).unwrap();
        conn.execute("ATTACH DATABASE ?1 AS cat", [attach_bytes(&catalog).unwrap()]).unwrap();
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
        let dir = test_folder(SCRATCH, "forget").unwrap();
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

    /// The usage of a cache another process holds for a start over is not read (the read waits for the hold, and gives
    /// up): a read holds the cache shared while it reads, as the cache's own connections do.
    #[test]
    fn read_usage_is_refused_while_a_start_over_holds_the_cache() {
        let dir = test_folder(SCRATCH, "usage-held").unwrap();
        drop(open(&dir, 1 << 30));
        let held = pg_schema::Live::exclusive(&cache_path(&dir)).unwrap().expect("free");
        assert!(ThumbCache::read_usage(&dir).is_err(), "held for a start over: not read");
        drop(held);
        assert!(ThumbCache::read_usage(&dir).unwrap().is_some(), "read once free");
    }

    #[test]
    fn a_cache_of_another_catalog_starts_over() {
        let dir = test_folder(SCRATCH, "other-catalog").unwrap();
        let cache = open(&dir, 12345);
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap();
        drop(cache);
        let (reopened, reason, path, why) = open_started_over(&dir, "rebuilt", None);
        assert!(why.contains("another catalog"), "{why}");
        assert_eq!((reason, path), (StartOverReason::OtherCatalog, cache_path(&dir)));
        assert!(reopened.get_nearest(FileId::new(1), 100, src(5)).unwrap().is_none(), "file 1 is another file now");
        assert_eq!((reopened.totals().unwrap(), reopened.cap().unwrap()), ((0, 0), 12345));
        // Open elsewhere: it can't be deleted, so it isn't used.
        let e = ThumbCache::open(&dir, CATALOG_ID, None, Arc::new(Bus::new(1))).err().expect("refused");
        assert!(format!("{e:#}").contains("another catalog"), "{e:#}");
    }

    #[test]
    fn a_cache_of_an_earlier_layout_starts_over() {
        let dir = test_folder(SCRATCH, "earlier-layout").unwrap();
        // Before the catalog id and the watermarks: no `meta`, no `checked`.
        let old = Connection::open(dir.join("thumbs.sqlite")).unwrap();
        old.execute_batch(
            "CREATE TABLE entries(file_id INTEGER, height INTEGER); INSERT INTO entries VALUES (1, 100);
             CREATE TABLE limits(id INTEGER PRIMARY KEY, cap INTEGER NOT NULL); INSERT INTO limits VALUES (1, 777);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        drop(old);
        let (cache, reason, _, why) = open_started_over(&dir, CATALOG_ID, None);
        assert!(why.contains("entries"), "{why}");
        assert_eq!(reason, StartOverReason::OtherBuild);
        assert_eq!((cache.totals().unwrap(), cache.cap().unwrap()), ((0, 0), 777), "empty, its cap kept");
        cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap();
        assert!(cache.get_nearest(FileId::new(1), 100, src(5)).unwrap().is_some());
    }

    #[test]
    fn disabled_keeps_nothing() {
        let cache = ThumbCache::disabled();
        cache.put(thumb(1, 100, 5, 10));
        assert!(!cache.put_batch(&mut vec![thumb(1, 100, 5, 10)], None).unwrap());
        assert!(cache.get_nearest(FileId::new(1), 100, src(5)).unwrap().is_none());
        assert_eq!(cache.totals().unwrap(), (0, 0));
        assert!(cache.pending(Path::new("nowhere"), 100).unwrap().work.is_empty());
        cache.flush();
        cache.close();
    }
}
