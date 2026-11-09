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

mod config;
mod maintenance;
mod reader;
mod schema;
mod types;
mod writer;

pub use config::{CACHE_VERSION, MAX_DEFAULT_CAP_BYTES, cache_path, default_cap_bytes, parse_cap_gb};
pub use maintenance::Drift;
pub use reader::pending_in;
pub use types::{Event, FORMAT_JPEG, Pending, Snapshot, Source, StartOverReason, Thumb, ThumbWork, Usage};

use std::path::Path;
use std::sync::{Arc, mpsc};

use anyhow::{Context, Result, anyhow, bail};
use pg_jobs::events::Bus;
use pg_jobs::service::Service;
use pg_model::ids::FileId;
use pg_model::path::WorkList;
use pg_platform::sqlite::sqlite_path;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::config::TOUCH_AGE_S;
use crate::maintenance::unix_now;
use crate::reader::{READERS, with_reader};
use crate::schema::{BUSY_TIMEOUT, Writing, hold_shared, open_writer};
use crate::writer::{Inbox, Op, Store, write_loop};

#[cfg(test)]
use pg_model::turns::Turns;
#[cfg(test)]
use pg_platform::sqlite::attach_bytes;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use crate::maintenance::SHRINK_ABOVE_PAGES;
#[cfg(test)]
use crate::reader::pending_sql;
#[cfg(test)]
use crate::schema::APPLICATION_ID;

/// The thumbnail cache. Without a file (it couldn't be opened, `disabled`), nothing is kept: thumbnails are made
/// every time they are shown.
pub struct ThumbCache {
    store: Option<Store>,
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
    /// found here; damage SQLite finds in the entries' pages only at the day's check of the totals, so up to a day
    /// late. No check reads a JPEG's bytes: a thumbnail that doesn't decode is found by the reader (see
    /// `crates/README.md`).
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

    /// Drop the thumbnail of `file_id` at `height`, for one that doesn't decode, and forget how far that height was
    /// checked (as an eviction does): the next pass at that height looks at every file again, and makes it anew from
    /// the original. Waits.
    pub fn forget_height(&self, file_id: FileId, height: u32) -> Result<()> {
        if self.store.is_none() {
            return Ok(());
        }
        let (reply, answer) = mpsc::channel();
        self.send(Op::ForgetHeight { file_id, height, reply })?;
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

    /// The same damage found on an open that passes a cap: the cap is written only after the day's check, so the
    /// damaged file is moved aside with the bytes it had.
    #[test]
    fn damage_found_on_an_open_with_a_cap_is_moved_aside_whole() {
        let dir = test_folder(SCRATCH, "heal-damage-cap").unwrap();
        let (mut bytes, page) = closed_cache_due_a_check(&dir);
        let keep = 16 * page;
        assert!(bytes.len() > 4 * keep, "the cache is too small to damage past its roots: {} bytes", bytes.len());
        for b in &mut bytes[keep..] {
            *b = 0xEE;
        }
        std::fs::write(cache_path(&dir), &bytes).unwrap();
        let (cache, reason, path, why) = open_started_over(&dir, CATALOG_ID, Some(12345));
        assert!(why.contains("damaged"), "{why}");
        assert_eq!((reason, path), (StartOverReason::Damaged, bad_path(&dir)));
        assert_eq!(std::fs::read(bad_path(&dir)).unwrap(), bytes, "the old bytes are kept");
        assert_eq!(cache.cap().unwrap(), 12345);
        drop(cache);
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
        let c = pg_catalog::Catalog::prepare(&catalog, pg_catalog::Versions { import: pg_analysis::analyze::IMPORT_VERSION, borders: pg_analysis::borders::BORDERS_VERSION }, rules).unwrap();
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

    /// A height's entry dropped on its own (a thumbnail that doesn't decode): its file is pending at that height again,
    /// with the height looked at whole, and the other files' entries stay.
    #[test]
    fn forgetting_a_height_makes_its_file_pending_again() {
        let dir = test_folder(SCRATCH, "forget-height").unwrap();
        let (catalog, _c) = catalog_of_four(&dir);
        let cache = open(&dir, 1_000_000);
        let pass = cache.pending(&catalog, 100).unwrap();
        assert!(cache.put_batch(&mut made(&pass, |_| true, 95), Some(100)).unwrap());
        cache.checked(&pass.snapshot, None).unwrap();
        assert!(cache.pending(&catalog, 100).unwrap().work.is_empty());
        let file = pass.work.items[0].id;
        cache.forget_height(file, 100).unwrap();
        let after = cache.pending(&catalog, 100).unwrap();
        assert!(!after.snapshot.since_checked, "the height is looked at whole");
        assert_eq!(ids(&after), vec![file.get()]);
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

    /// An earlier layout whose `limits` has no `cap` column is started over with the default cap: there is no cap to
    /// keep, and reading the missing one is not an error (that would keep the cache from opening at all).
    #[test]
    fn an_earlier_limits_without_a_cap_starts_over_with_the_default() {
        let dir = test_folder(SCRATCH, "earlier-limits").unwrap();
        let old = Connection::open(dir.join("thumbs.sqlite")).unwrap();
        old.execute_batch(
            "CREATE TABLE entries(file_id INTEGER, height INTEGER);
             CREATE TABLE limits(id INTEGER PRIMARY KEY); INSERT INTO limits VALUES (1);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        drop(old);
        let (cache, reason, _, _) = open_started_over(&dir, CATALOG_ID, None);
        assert_eq!(reason, StartOverReason::OtherBuild);
        assert_eq!((cache.totals().unwrap(), cache.cap().unwrap()), ((0, 0), default_cap_bytes(&dir)));
        drop(cache);
        std::fs::remove_dir_all(&dir).unwrap();
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
