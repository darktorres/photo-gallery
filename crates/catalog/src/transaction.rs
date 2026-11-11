//! The catalog's write transaction, which every module that writes begins: a leaf, below `core`.
//!
//! The tests of the crates around the catalog measure how long each write transaction holds the write lock
//! (`watch_holds`, through `crate::fixture::WriteHolds`): a walk must never hold it across a listing or a pause.

use std::ops::Deref;

use rusqlite::{Connection, Transaction, TransactionBehavior};

/// A transaction that writes: it takes the write lock when it begins, so a lock held by another connection is waited
/// for (`busy_timeout`) before anything is read. A deferred one that reads first and then writes can instead fail at
/// once when another connection has committed in between (SQLITE_BUSY_SNAPSHOT), which no wait fixes.
pub(crate) fn write_transaction(conn: &Connection) -> rusqlite::Result<WriteTx<'_>> {
    let inner = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    Ok(WriteTx { inner, _hold: Hold::begin() })
}

/// A write transaction ([`write_transaction`]): a `rusqlite::Transaction` that also times its hold of the write lock,
/// from its begin to its commit or rollback. It is used as the transaction it derefs to; [`WriteTx::commit`] ends it.
pub(crate) struct WriteTx<'c> {
    inner: Transaction<'c>,
    /// Dropped after `inner` ends (fields drop in order, and `commit` drops it last).
    _hold: Hold,
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
        let WriteTx { inner, _hold } = self;
        inner.commit()
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
