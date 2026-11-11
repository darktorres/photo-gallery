//! The catalog's write transaction, which every module that writes begins: a leaf, below `core`.

use rusqlite::{Connection, Transaction, TransactionBehavior};

/// A transaction that writes: it takes the write lock when it begins, so a lock held by another connection is waited
/// for (`busy_timeout`) before anything is read. A deferred one that reads first and then writes can instead fail at
/// once when another connection has committed in between (SQLITE_BUSY_SNAPSHOT), which no wait fixes.
pub(crate) fn write_transaction(conn: &Connection) -> rusqlite::Result<Transaction<'_>> {
    Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
}
