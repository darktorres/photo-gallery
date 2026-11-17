//! The catalog's write transaction, which every module that writes begins: a leaf, below `core`.
//!
//! The tests of the crates around the catalog measure how long each write transaction holds the write lock
//! (`watch_holds`, through `crate::fixture::WriteHolds`): a walk must never hold it across a listing or a pause.

use std::ops::Deref;

use pg_perf::{Mark, Tally, Timer};
use rusqlite::{Connection, Transaction, TransactionBehavior};

/// The write transactions' time, process-wide, for the perf log: the wait for the write lock (`BEGIN IMMEDIATE`), the
/// hold from the begin to the end of the commit (or the rollback), and the commit alone. A pass's span takes the share it
/// caused as the difference of [`tx_mark`] (see [`attach_tx`]).
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

/// A transaction that writes: it takes the write lock when it begins, so a lock held by another connection is waited
/// for (`busy_timeout`) before anything is read. A deferred one that reads first and then writes can instead fail at
/// once when another connection has committed in between (SQLITE_BUSY_SNAPSHOT), which no wait fixes.
pub(crate) fn write_transaction(conn: &Connection) -> rusqlite::Result<WriteTx<'_>> {
    let wait = TX_WAIT.timer();
    let inner = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    drop(wait);
    Ok(WriteTx { inner, _hold: Hold::begin(), perf_hold: TX_HOLD.timer() })
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
