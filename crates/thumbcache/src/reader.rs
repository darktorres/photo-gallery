//! The read side: the pending query (the files a pass still has to thumbnail) and this thread's read connections,
//! which serve lookups without waiting for the writer.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use anyhow::Result;
use pg_model::ids::FileId;
use pg_model::path::WorkList;
use pg_model::size::ThumbHeight;
use pg_model::turns::Turns;
use pg_platform::sqlite::attach_bytes;
use rusqlite::{Connection, OptionalExtension, params};

use crate::schema::{BUSY_TIMEOUT, CONNECTION_PRAGMAS};
use crate::types::{Pending, Snapshot, Source, ThumbWork};

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
    // The folder ids' table is sized up front. The items are not: counting them first would scan the view.
    let folders: i64 = tx.query_row("SELECT max_id FROM cat.thumb_folders", [], |r| r.get(0))?;
    let mut work = WorkList::with_capacity(0, folders);
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
                source: Source { mtime: r.get(3)?, rotation: Turns::new(r.get(4)?) },
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
    let mut stmt = tx.prepare(
        "WITH RECURSIVE v(volume) AS (
             SELECT min(volume) FROM cat.files WHERE volume IS NOT NULL
             UNION ALL
             SELECT (SELECT min(volume) FROM cat.files WHERE volume > v.volume) FROM v WHERE v.volume IS NOT NULL
         )
         SELECT volume FROM v WHERE volume IS NOT NULL",
    )?;
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
        let dir = pg_testkit::run_folder("scratch_thumbs", "checkpoint-plan").unwrap();
        let catalog = dir.join("catalog.sqlite");
        let versions = pg_catalog::Versions {
            import: pg_analysis::analyze::IMPORT_VERSION,
            borders: pg_analysis::borders::BORDERS_VERSION,
            thumbs: 1,
            pairing: pg_model::pdq::PAIRING_VERSION,
        };
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
        "SELECT f.id, f.folder_id, f.name, f.mtime, f.rotation, f.poster_ms, f.seq, f.folder, f.size
         FROM cat.thumb_sources f
         LEFT JOIN entries t ON t.file_id = f.id AND t.height = ?1
         WHERE {changed}{volume} IN ({}) AND (t.file_id IS NULL OR t.mtime != f.mtime OR t.rotation != f.rotation)
           AND NOT EXISTS (SELECT 1 FROM cat.file_failures x WHERE x.file_id = f.id AND x.pass = 'thumbs'
                           AND x.retry = 0 AND x.mtime = f.mtime AND x.size = f.size AND x.rotation = f.rotation)
         ORDER BY {volume}, f.file_ref & 0xFFFFFFFFFFFF",
        list.join(",")
    )
}

thread_local! {
    /// This thread's read connections, one per cache file.
    pub(crate) static READERS: RefCell<Vec<(PathBuf, Connection)>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn with_reader<R>(path: &Path, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
    READERS.with(|readers| {
        let mut readers = readers.borrow_mut();
        if let Some((_, conn)) = readers.iter().find(|(p, _)| p == path) {
            return f(conn);
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.execute_batch(CONNECTION_PRAGMAS)?;
        conn.execute_batch("PRAGMA query_only = ON;")?;
        let result = f(&conn);
        readers.push((path.to_path_buf(), conn));
        result
    })
}
