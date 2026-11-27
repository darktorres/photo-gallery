//! The catalog's write transaction, which every module that writes begins: a leaf, below `core`.
//!
//! The tests of the crates around the catalog measure how long each write transaction holds the write lock
//! (`watch_holds`, through `crate::fixture::WriteHolds`): a walk must never hold it across a listing or a pause.

use std::cell::RefCell;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pg_perf::{Mark, Tally, Timer};
use rusqlite::{Connection, ErrorCode, Transaction, TransactionBehavior};

use crate::error::{Error, Result};

/// The write transactions' time, process-wide, for the perf log: the wait for the write lock (`BEGIN IMMEDIATE`, retries
/// included), the hold from the begin to the end of the commit (or the rollback), and the commit alone. A pass's span takes
/// the share it caused as the difference of [`tx_mark`] (see [`attach_tx`]).
static TX_WAIT: Tally = Tally::new("catalog.tx.wait");
static TX_HOLD: Tally = Tally::new("catalog.tx.hold");
static TX_COMMIT: Tally = Tally::new("catalog.tx.commit");

/// The write transactions' totals now (see [`attach_tx`]).
#[derive(Clone, Copy, Default)]
pub struct TxMark([Mark; 3]);

/// The write transactions' totals now, for [`attach_tx`].
pub fn tx_mark() -> TxMark {
    TxMark([TX_WAIT.mark(), TX_HOLD.mark(), TX_COMMIT.mark()])
}

/// Puts the write transactions' time since `since` (a [`tx_mark`]) on `span`: the lock waits, holds and commits the
/// span's work caused. Process-wide, so a pass that runs alongside another writer also counts that writer's share.
pub fn attach_tx(span: &mut pg_perf::Span, since: TxMark) {
    span.attach_since(&TX_WAIT, since.0[0]);
    span.attach_since(&TX_HOLD, since.0[1]);
    span.attach_since(&TX_COMMIT, since.0[2]);
}

/// How long a write waits between its tries to begin while another connection holds the write lock.
const RETRY_EVERY: Duration = Duration::from_millis(50);

/// How long this thread's write transactions keep trying to begin while another connection holds the write lock, once
/// the connection's own busy timeout (`crate::schema::BUSY_TIMEOUT_MS`, 5 s) has run out: until `until`, and not once `stop` is set. A thread
/// that sets no patience keeps the timeout alone.
#[derive(Clone)]
pub struct Patience {
    /// Set when the wait should stop.
    pub stop: Arc<AtomicBool>,
    /// The time the wait ends at.
    pub until: Instant,
}

thread_local! {
    static PATIENCE: RefCell<Option<Patience>> = const { RefCell::new(None) };
}

/// Sets this thread's [`Patience`] until the guard is dropped, when the one it replaced comes back.
pub fn wait_for_writers(patience: Patience) -> PatienceGuard {
    let earlier = PATIENCE.with_borrow_mut(|p| p.replace(patience));
    PatienceGuard { earlier }
}

/// The patience set by [`wait_for_writers`], restored when dropped.
pub struct PatienceGuard {
    earlier: Option<Patience>,
}

impl Drop for PatienceGuard {
    fn drop(&mut self) {
        let earlier = self.earlier.take();
        PATIENCE.with_borrow_mut(|p| *p = earlier);
    }
}

/// Whether this thread tries to begin once more: its patience holds, and it sleeps a step first.
fn keep_waiting() -> bool {
    let patient = PATIENCE
        .with_borrow(|p| p.as_ref().is_some_and(|p| !p.stop.load(Ordering::SeqCst) && Instant::now() < p.until));
    if patient {
        std::thread::sleep(RETRY_EVERY);
    }
    patient
}

/// Merges the whole write-ahead log of `conn` into its file and empties the log (`TRUNCATE`). SQLite reports a merge that
/// a reader's pages kept from finishing as a result row, not an error, so the row is read: a merge that did not finish is
/// [`Error::WalNotMerged`], and the file is whole regardless (its changes stay in the log).
pub(crate) fn checkpoint_truncate(conn: &Connection) -> Result<()> {
    let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
    if busy != 0 {
        let path = conn.path().unwrap_or_default().to_string();
        return Err(Error::WalNotMerged { path });
    }
    Ok(())
}

/// A transaction that writes: it takes the write lock when it begins, so a lock held by another connection is waited
/// for (`busy_timeout`, then this thread's [`Patience`]) before anything is read. A deferred one that reads first and
/// then writes can instead fail at once when another connection has committed in between (SQLITE_BUSY_SNAPSHOT), which
/// no wait fixes.
pub(crate) fn write_transaction(conn: &Connection) -> rusqlite::Result<WriteTx<'_>> {
    // The wait for the lock, the retries included: timed whether or not the transaction begins.
    let wait = TX_WAIT.timer();
    loop {
        match Transaction::new_unchecked(conn, TransactionBehavior::Immediate) {
            Ok(inner) => {
                drop(wait);
                return Ok(WriteTx { inner, _hold: Hold::begin(), perf_hold: TX_HOLD.timer() });
            }
            Err(e) if e.sqlite_error_code() == Some(ErrorCode::DatabaseBusy) && keep_waiting() => {}
            Err(e) => return Err(e),
        }
    }
}

/// A write transaction ([`write_transaction`]): a `rusqlite::Transaction` that also times its hold of the write lock,
/// from its begin to its commit or rollback. It is used as the transaction it derefs to; [`WriteTx::commit`] ends it.
pub(crate) struct WriteTx<'c> {
    inner: Transaction<'c>,
    /// Dropped after `inner` ends (fields drop in order, and `commit` drops it last).
    _hold: Hold,
    /// The hold for the perf log (the same interval, timed whether or not the tests watch it).
    perf_hold: Timer<'static>,
}

impl<'c> Deref for WriteTx<'c> {
    type Target = Transaction<'c>;

    fn deref(&self) -> &Transaction<'c> {
        &self.inner
    }
}

impl WriteTx<'_> {
    /// Commit, and end the hold: `_hold` is dropped at the end of the body, after the commit it times.
    pub(crate) fn commit(self) -> rusqlite::Result<()> {
        let WriteTx { inner, _hold, perf_hold } = self;
        let commit = TX_COMMIT.timer();
        let result = inner.commit();
        drop(commit);
        drop(perf_hold);
        result
    }
}

#[cfg(any(test, feature = "fixture"))]
mod holds {
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    thread_local! {
        /// The holds measured on this thread while a watch is on (`watch_holds`), in the order they ended.
        static WATCHED: RefCell<Option<Vec<Duration>>> = const { RefCell::new(None) };
    }

    /// One write transaction's hold of the lock, timed when a watch is on.
    pub(crate) struct Hold(Option<Instant>);

    impl Hold {
        pub(crate) fn begin() -> Hold {
            Hold(WATCHED.with_borrow(Option::is_some).then(Instant::now))
        }
    }

    impl Drop for Hold {
        fn drop(&mut self) {
            if let Some(began) = self.0 {
                let held = began.elapsed();
                WATCHED.with_borrow_mut(|w| {
                    if let Some(holds) = w.as_mut() {
                        holds.push(held);
                    }
                });
            }
        }
    }

    /// Start timing this thread's write transactions (any earlier holds measured are dropped).
    pub(crate) fn watch_holds() {
        WATCHED.with_borrow_mut(|w| *w = Some(Vec::new()));
    }

    /// The holds measured on this thread since the watch began, taken: the watch goes on, empty.
    pub(crate) fn take_holds() -> Vec<Duration> {
        WATCHED.with_borrow_mut(|w| w.as_mut().map(std::mem::take).unwrap_or_default())
    }

    /// Stop timing this thread's write transactions.
    pub(crate) fn unwatch_holds() {
        WATCHED.with_borrow_mut(|w| *w = None);
    }
}

#[cfg(any(test, feature = "fixture"))]
pub(crate) use holds::{Hold, take_holds, unwatch_holds, watch_holds};

/// Without the test fixture, nothing is timed.
#[cfg(not(any(test, feature = "fixture")))]
pub(crate) struct Hold;

#[cfg(not(any(test, feature = "fixture")))]
impl Hold {
    fn begin() -> Hold {
        Hold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A catalog file with a table, and a connection that doesn't wait on its own (`busy_timeout` 0), so only the
    /// patience can make a write wait.
    fn two_connections(name: &str) -> (Connection, Connection, Connection) {
        let dir = pg_testkit::run_folder("scratch_catalog", name).unwrap();
        let path = dir.join("catalog.sqlite");
        let open = || {
            let conn = Connection::open(&path).unwrap();
            conn.busy_timeout(Duration::ZERO).unwrap();
            conn
        };
        let setup = open();
        setup.execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE t (x INTEGER);").unwrap();
        (setup, open(), open())
    }

    #[test]
    fn a_busy_begin_fails_at_once_without_patience() {
        let (_setup, holder, mine) = two_connections("write-no-patience");
        holder.execute_batch("BEGIN IMMEDIATE").unwrap();
        // No busy timeout and no patience: the error comes at once, with no wait for a timer to measure.
        assert!(write_transaction(&mine).is_err());
        holder.execute_batch("COMMIT").unwrap();
    }

    #[test]
    fn a_write_waits_for_a_lock_released_within_its_patience() {
        let (_setup, holder, mine) = two_connections("write-patience");
        holder.execute_batch("BEGIN IMMEDIATE").unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            holder.execute_batch("COMMIT").unwrap();
        });
        let _patience = wait_for_writers(Patience {
            stop: Arc::new(AtomicBool::new(false)),
            until: Instant::now() + Duration::from_secs(5),
        });
        let tx = write_transaction(&mine).unwrap();
        tx.execute("INSERT INTO t (x) VALUES (1)", []).unwrap();
        tx.commit().unwrap();
        releaser.join().unwrap();
    }

    #[test]
    fn a_stop_ends_the_wait_and_the_patience_is_restored_after() {
        let (_setup, holder, mine) = two_connections("write-stop");
        holder.execute_batch("BEGIN IMMEDIATE").unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        {
            let _patience =
                wait_for_writers(Patience { stop: stop.clone(), until: Instant::now() + Duration::from_secs(60) });
            stop.store(true, Ordering::SeqCst);
            let began = Instant::now();
            assert!(write_transaction(&mine).is_err(), "a stopped wait gives up");
            // Well short of the 60 s patience, which the stop must cut short; the margin is wide so load can't fail it.
            assert!(began.elapsed() < Duration::from_secs(30));
        }
        assert!(PATIENCE.with_borrow(Option::is_none), "the guard restores the earlier patience");
        holder.execute_batch("COMMIT").unwrap();
    }
}
