//! The read side: the pending query (the files a pass still has to thumbnail) and this thread's read connections,
//! which serve lookups without waiting for the writer.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use anyhow::Result;
use pg_model::ids::FileId;
use pg_model::path::WorkList;
use pg_model::turns::Turns;
use pg_platform::sqlite::{attach_bytes, sqlite_path};
use rusqlite::{Connection, OptionalExtension};

use crate::schema::{BUSY_TIMEOUT, CONNECTION_PRAGMAS};
use crate::types::{Pending, Snapshot, Source, ThumbWork};

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
pub(crate) fn pending_sql(since_checked: bool) -> &'static str {
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
    pub(crate) static READERS: RefCell<Vec<(PathBuf, Connection)>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn with_reader<R>(path: &Path, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
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
