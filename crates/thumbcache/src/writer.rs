//! The write side: the writer's queue and its loop, and every change the cache makes to its file: stores, touches,
//! forgets, the checks it is asked for, and eviction.

use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::Result;
use pg_jobs::events::Bus;
use pg_jobs::service::{Service, Waker, Woke};
use pg_model::ids::FileId;
use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use crate::config::TOUCH_FLUSH;
use crate::maintenance::{Drift, heal, shrink};
use crate::types::{Event, FORMAT_JPEG, Snapshot, Thumb};

/// Eviction frees this much more than it must (1% of the cap, at most this), so a full cache doesn't evict on
/// every batch.
const EVICT_SLACK_MAX: i64 = 64 << 20;

/// No transaction evicts much more than this: a cache far over its cap (the cap lowered) is brought down in slices,
/// each its own transaction, so the file is never locked for long (another process waits 5 s at most).
const EVICT_SLICE: i64 = if cfg!(test) { 1000 } else { 256 << 20 };

pub(crate) struct Store {
    pub(crate) path: PathBuf,
    pub(crate) inbox: Arc<Inbox>,
    pub(crate) writer: Service,
    /// The shared hold on the file (`pg_dbfile::Live`), for as long as the cache is open.
    pub(crate) _live: pg_dbfile::Live,
}

/// The writer's queue: the ops not yet done, in order, and whether more are taken. The writer is woken after each push.
pub(crate) struct Inbox {
    state: Mutex<Queue>,
}

struct Queue {
    ops: VecDeque<Op>,
    /// False once the cache is closed or its writer has ended: nothing more is queued.
    open: bool,
}

impl Inbox {
    pub(crate) fn new() -> Inbox {
        Inbox { state: Mutex::new(Queue { ops: VecDeque::new(), open: true }) }
    }

    /// Queues `op`; false, queuing nothing, once the queue is closed.
    pub(crate) fn push(&self, op: Op) -> bool {
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
    pub(crate) fn close(&self) {
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
pub(crate) enum Op {
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
    /// One height's thumbnail of the file is dropped, and that height's watermark with it.
    ForgetHeight {
        file_id: FileId,
        height: u32,
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
pub(crate) fn write_loop(mut conn: Connection, inbox: &Inbox, events: &Bus<Event>, waker: &Waker<'_>) {
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
        Op::ForgetHeight { file_id, height, reply } => {
            let done = forget_height_entry(conn, file_id, height);
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

/// Drop the entry of `file_id` at `height`, and that height's watermark (`checked`), as an eviction does
/// ([`crate::ThumbCache::forget_height`]). The entry's JPEG goes with it (the trigger on `entries`).
fn forget_height_entry(conn: &mut Connection, file_id: FileId, height: u32) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM entries WHERE file_id = ?1 AND height = ?2", params![file_id.get(), height])?;
    tx.execute(
        "INSERT INTO checked(height, seq, evictions) VALUES (?1, NULL, 1)
         ON CONFLICT(height) DO UPDATE SET seq = NULL, evictions = evictions + 1",
        [height],
    )?;
    tx.commit()?;
    Ok(())
}
