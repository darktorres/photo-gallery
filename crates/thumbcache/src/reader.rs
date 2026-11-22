//! The read side: the pending query (the files a pass still has to thumbnail) and the cache file's read connections
//! ([`ReaderPool`]), which serve lookups without waiting for the writer.

use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

use anyhow::Result;
use pg_model::ids::FileId;
use pg_model::path::WorkList;
use pg_model::size::ThumbHeight;
use pg_model::turns::Turns;
use pg_platform::sqlite::attach_bytes;
use rusqlite::{Connection, OptionalExtension, params};

use crate::schema::{BUSY_TIMEOUT, CONNECTION_PRAGMAS};
use crate::types::{Pending, Snapshot, Source, ThumbWork};

/// The files whose thumbnails the cache at `cache` holds and the catalog at `catalog` no longer does (`ThumbCache::
/// sweep_orphans`). The catalog's id high-water mark (`thumb_high_water`: the ids are never reused) is read first. An
/// entry at or below it whose file is absent was deleted for good; a file inserted meanwhile has a higher id, and is not
/// among them. Read on one connection with `query_only`, so no write of the cache waits for it. The catalog is read
/// through its views only (`thumb_high_water`, `thumb_file_ids`, `thumb_volumes`, `thumb_clock`, `thumb_sources`).
pub(crate) fn orphans_in(cache: &Path, catalog: &Path) -> Result<Vec<FileId>> {
    // ATTACH makes an empty file at a path with none, so a missing catalog is an error here, not a new catalog beside
    // the cache (the sweep's caller logs it).
    if !catalog.exists() {
        anyhow::bail!("catalog {}: not there, so its orphans are not looked for", catalog.display());
    }
    let conn = Connection::open(cache)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.execute_batch("PRAGMA query_only = ON; PRAGMA temp_store = MEMORY;")?;
    conn.execute("ATTACH DATABASE ?1 AS cat", [attach_bytes(catalog)?])?;
    let high: i64 = conn.query_row("SELECT high FROM cat.thumb_high_water", [], |r| r.get(0))?;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT file_id FROM main.entries WHERE file_id <= ?1 AND file_id NOT IN (SELECT id FROM cat.thumb_file_ids) \
         ORDER BY file_id",
    )?;
    let ids: Vec<i64> = stmt.query_map([high], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(ids.into_iter().map(FileId::new).collect())
}

/// `ThumbCache::pending` on the cache file `cache` (a free function: the bench calls it without a writer), over every
/// volume of the catalog. See `pending_for`.
pub fn pending_in(cache: &Path, catalog: &Path, height: ThumbHeight) -> Result<Pending> {
    pending_for(cache, catalog, height, None)
}

/// [`pending_in`] over the given catalog volumes only: a lane reads and checks its own volumes' work, and the others' are
/// another lane's to check.
pub(crate) fn pending_on(cache: &Path, catalog: &Path, height: ThumbHeight, volumes: &[i64]) -> Result<Pending> {
    pending_for(cache, catalog, height, Some(volumes))
}

/// One read transaction over both files: the catalog's change number, the volumes' checkpoints and the height's
/// evictions are one moment's. The checkpoints give the change number every one of the volumes was checked up to (the
/// least of them), when each has one. With that number no later than the catalog's (a catalog restored from a copy
/// would be behind), only the files changed since are read, through `files_seq`; otherwise every file of the volumes.
fn pending_for(cache: &Path, catalog: &Path, height: ThumbHeight, volumes: Option<&[i64]>) -> Result<Pending> {
    let conn = Connection::open(cache)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    // The whole entries table fits in the page cache (~40 bytes an entry); the sort stays in memory.
    conn.execute_batch("PRAGMA query_only = ON; PRAGMA temp_store = MEMORY; PRAGMA cache_size = -131072;")?;
    conn.execute("ATTACH DATABASE ?1 AS cat", [attach_bytes(catalog)?])?;
    let tx = conn.unchecked_transaction()?;
    let seq: i64 = tx.query_row("SELECT seq FROM cat.thumb_clock", [], |r| r.get(0))?;
    let evictions: i64 = tx
        .query_row("SELECT evictions FROM checked WHERE height = ?1", [height.get()], |r| r.get(0))
        .optional()?
        .unwrap_or(0);
    let volumes = match volumes {
        Some(volumes) => volumes.to_vec(),
        None => catalog_volumes(&tx)?,
    };
    let since = checkpoint_since(&tx, i64::from(height.get()), &volumes)?.filter(|&c| c <= seq);
    let values = match since {
        Some(c) => vec![i64::from(height.get()), c],
        None => vec![i64::from(height.get())],
    };
    // The items are not sized up front: counting them first would scan the view.
    let mut work = WorkList::with_capacity(0);
    {
        let mut stmt = tx.prepare(&pending_sql(since.is_some(), &volumes))?;
        let mut rows = stmt.query(rusqlite::params_from_iter(values))?;
        while let Some(r) = rows.next()? {
            let name = r.get_ref(2)?.as_blob()?;
            let path =
                work.file(r.get(1)?, name, |arena| Ok::<_, rusqlite::Error>(arena.push(r.get_ref(7)?.as_blob()?)))?;
            work.items.push(ThumbWork {
                id: FileId::new(r.get(0)?),
                path,
                source: Source { mtime: r.get(3)?, turns: Turns::new(r.get(4)?) },
                size: r.get(8)?,
                poster_ms: r.get(5)?,
                seq: r.get(6)?,
            });
        }
    }
    tx.commit()?;
    Ok(Pending { work, snapshot: Snapshot { height, seq, evictions, since_checked: since.is_some(), volumes } })
}

/// The change number every one of `volumes` was checked up to at `height`: the least of their checkpoints, or `None`
/// when one has none (the height is looked at whole), or there are no volumes.
fn checkpoint_since(tx: &Connection, height: i64, volumes: &[i64]) -> Result<Option<i64>> {
    if volumes.is_empty() {
        return Ok(None);
    }
    let mut stmt = tx.prepare("SELECT seq FROM checkpoints WHERE height = ?1 AND volume = ?2")?;
    let mut least: Option<i64> = None;
    for &volume in volumes {
        let Some(seq) = stmt.query_row(params![height, volume], |r| r.get::<_, i64>(0)).optional()? else {
            return Ok(None);
        };
        least = Some(least.map_or(seq, |l| l.min(seq)));
    }
    Ok(least)
}

/// The catalog's volumes, by a loose index scan over `files`'s `(volume, file_ref)` index: a step per volume, not per
/// file. A displaced row (no volume) is none.
fn catalog_volumes(tx: &Connection) -> Result<Vec<i64>> {
    let mut stmt = tx.prepare(pg_query::volumes_sql!("cat.thumb_volumes"))?;
    Ok(stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod checkpoint_tests {
    use super::*;

    fn with_files() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE checkpoints(height INTEGER, volume INTEGER, seq INTEGER, PRIMARY KEY (height, volume));
             ATTACH DATABASE ':memory:' AS cat;
             CREATE TABLE cat.files(volume INTEGER, file_ref INTEGER);
             CREATE INDEX cat.files_volume ON files(volume, file_ref);
             CREATE VIEW cat.thumb_volumes AS SELECT volume FROM files WHERE volume IS NOT NULL;
             INSERT INTO cat.files VALUES (1, 1), (1, 2), (3, 1), (3, 2), (7, 9), (NULL, 4);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn the_volumes_are_found_one_step_each_and_a_displaced_row_is_none() {
        assert_eq!(catalog_volumes(&with_files()).unwrap(), vec![1, 3, 7]);
    }

    #[test]
    fn a_height_is_checked_up_to_the_least_of_its_volumes_checkpoints() {
        let conn = with_files();
        conn.execute_batch("INSERT INTO checkpoints VALUES (100, 1, 40), (100, 3, 25);").unwrap();
        assert_eq!(checkpoint_since(&conn, 100, &[1, 3]).unwrap(), Some(25), "the least");
        assert_eq!(checkpoint_since(&conn, 100, &[1]).unwrap(), Some(40), "one volume: its own");
        assert_eq!(checkpoint_since(&conn, 100, &[1, 7]).unwrap(), None, "a volume with none is looked at whole");
        assert_eq!(checkpoint_since(&conn, 100, &[]).unwrap(), None);
        assert_eq!(checkpoint_since(&conn, 200, &[1]).unwrap(), None, "another height has none");
    }

    /// The pass since a checkpoint, over a volume filter, still reads the change index: `+f.volume` keeps the planner on
    /// `files_seq`, where a plain `f.volume IN` would switch it to the volume index and scan every file of the lane. A
    /// real catalog, so the view is the one the pass reads.
    #[test]
    fn the_pass_since_a_checkpoint_with_a_volume_filter_reads_the_change_index() {
        use pg_model::ids::FolderId;
        let dir = pg_testkit::run_folder("scratch_thumbcache", "checkpoint-plan").unwrap();
        let catalog = dir.join("catalog.sqlite");
        let versions = pg_catalog::Versions::current();
        let c = pg_catalog::Catalog::prepare(&catalog, versions, pg_model::rules::PathRules::Windows).unwrap();
        pg_catalog::fixture::state::add_scan(&c, 1, "D:\\p").unwrap();
        pg_catalog::fixture::state::add_folder(&c, 1, "D:\\p").unwrap();
        let a = pg_catalog::fixture::File::new(FileId::new(1), FolderId::new(1), "a.jpg", 1, 5);
        pg_catalog::fixture::insert(&c, &[a]).unwrap();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("ATTACH DATABASE ?1 AS cat", [attach_bytes(&catalog).unwrap()]).unwrap();
        conn.execute_batch(
            "CREATE TABLE entries(file_id INTEGER, height INTEGER, mtime INTEGER, rotation INTEGER,
                                  PRIMARY KEY (file_id, height));",
        )
        .unwrap();
        let plan: Vec<String> = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {}", pending_sql(true, &[1])))
            .unwrap()
            .query_map(params![100, 0], |r| r.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let plan = plan.join("\n");
        assert!(plan.contains("USING INDEX files_seq"), "{plan}");
        assert!(!plan.contains("sqlite_autoindex_files_1"), "the volume index is not the change index: {plan}");
    }
}

/// `pending_in`'s query over `volumes`, with the catalog attached as `cat`: ?1 the height, and with `since_checked` ?2
/// the change number checked up to. Which files can have a thumbnail is the catalog's view's to say (`thumb_sources`):
/// this only compares its rows with the cache's entries. A pass since a change number reads the few changed files by
/// `files_seq`, and the volumes are filtered as `+f.volume IN (…)` so the planner keeps that index: a plain
/// `f.volume IN` makes it prefer the `(volume, file_ref)` index and scan every file of the volumes.
pub(crate) fn pending_sql(since_checked: bool, volumes: &[i64]) -> String {
    let list: Vec<String> = volumes.iter().map(i64::to_string).collect();
    let (changed, volume) = if since_checked { ("f.seq > ?2 AND ", "+f.volume") } else { ("", "f.volume") };
    format!(
        "SELECT f.id, f.folder_id, f.name, f.mtime, f.turns, f.poster_ms, f.seq, f.folder, f.size
         FROM cat.thumb_sources f
         LEFT JOIN entries t ON t.file_id = f.id AND t.height = ?1
         WHERE {changed}{volume} IN ({}) AND (t.file_id IS NULL OR t.mtime != f.mtime OR t.rotation != f.turns)
           AND NOT EXISTS (SELECT 1 FROM cat.thumb_failures x WHERE x.file_id = f.id AND x.pass = 'thumbs'
                           AND x.retry = 0 AND x.mtime = f.mtime AND x.size = f.size AND x.rotation = f.turns)
         ORDER BY {volume}, f.file_ref & 0xFFFFFFFFFFFF",
        list.join(",")
    )
}

/// The read connections of one cache file, shared by every reader of the cache (on any thread): the idle ones, and how many
/// readers hold one now. A read takes an idle connection (or opens one), runs, and gives it back. An idle connection holds
/// no transaction, so the writer's checkpoint is never held up by one.
pub(crate) struct ReaderPool {
    path: PathBuf,
    state: Mutex<PoolState>,
    /// Signalled when a reader gives its connection back, for [`ReaderPool::shut`].
    returned: Condvar,
}

#[derive(Default)]
struct PoolState {
    idle: Vec<Connection>,
    /// The readers holding a connection now.
    out: usize,
    /// Set by [`ReaderPool::shut`]: no connection is kept after it. A read then opens its own, closed when it returns.
    closed: bool,
}

impl ReaderPool {
    pub(crate) fn new(path: &Path) -> ReaderPool {
        ReaderPool { path: path.to_path_buf(), state: Mutex::new(PoolState::default()), returned: Condvar::new() }
    }

    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `f` on a read connection of the cache file.
    pub(crate) fn with<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        let checkout = self.checkout()?;
        f(checkout.connection())
    }

    fn checkout(&self) -> Result<Checkout<'_>> {
        let (idle, pooled) = {
            let mut state = self.lock();
            if state.closed {
                (None, false)
            } else {
                state.out += 1;
                (state.idle.pop(), true)
            }
        };
        let mut checkout = Checkout { pool: self, conn: idle, pooled };
        if checkout.conn.is_none() {
            // A failure here still counts the reader out: the checkout is dropped with the error.
            checkout.conn = Some(open_reader(&self.path)?);
        }
        Ok(checkout)
    }

    /// Stops keeping connections, and waits until the readers holding one have given it back. `ThumbCache::close` calls
    /// it first, so no read is mid-way when the writer stops; [`ReaderPool::drain`] comes after the writer.
    pub(crate) fn shut(&self) {
        let mut state = self.lock();
        state.closed = true;
        let _state = self.returned.wait_while(state, |s| s.out > 0).unwrap_or_else(PoisonError::into_inner);
    }

    /// Closes the idle connections: the last connection to the file to close merges its WAL and deletes its index.
    pub(crate) fn drain(&self) {
        let idle = std::mem::take(&mut self.lock().idle);
        drop(idle);
    }
}

/// A read connection checked out of a [`ReaderPool`]. It is given back when dropped, which is also when a read that
/// panicked lets go of it: the pool counts the reader out either way.
struct Checkout<'a> {
    pool: &'a ReaderPool,
    conn: Option<Connection>,
    /// Whether the reader is counted out: it took its connection from a pool that was open.
    pooled: bool,
}

impl Checkout<'_> {
    fn connection(&self) -> &Connection {
        match &self.conn {
            Some(conn) => conn,
            None => unreachable!("a checkout holds its connection until it is dropped"),
        }
    }
}

impl Drop for Checkout<'_> {
    fn drop(&mut self) {
        let mut state = self.pool.lock();
        if self.pooled {
            state.out -= 1;
        }
        let conn = self.conn.take();
        let back = match conn {
            Some(conn) if self.pooled && !state.closed => {
                state.idle.push(conn);
                None
            }
            other => other,
        };
        drop(state);
        self.pool.returned.notify_all();
        // Dropped outside the lock: closing the last connection to a file merges its WAL.
        drop(back);
    }
}

/// A read connection to the cache file at `path`: busy-timed, with the connection's pragmas, and read-only.
fn open_reader(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.execute_batch(CONNECTION_PRAGMAS)?;
    conn.execute_batch("PRAGMA query_only = ON;")?;
    Ok(conn)
}

#[cfg(test)]
mod orphan_tests {
    use super::orphans_in;
    use pg_testkit::run_folder;

    /// A catalog that is not there is an error, and no catalog is made at its path: the attach would have made an empty
    /// file that the catalog's own open (and a rename over it) then trips on.
    #[test]
    fn a_missing_catalog_is_an_error_and_no_file_is_made() {
        let dir = run_folder("scratch_thumbcache", "orphans-missing-catalog").unwrap();
        let catalog = dir.join("catalog.sqlite");
        let cache = dir.join("thumbs.sqlite");
        assert!(orphans_in(&cache, &catalog).is_err());
        assert!(!catalog.exists(), "no catalog was made beside the cache");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
