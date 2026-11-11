//! The lock that orders the processes changing a database file's layout, and the holds a process keeps on a file it has
//! open. Both are files beside the database, which stay after they are let go.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;
use std::time::Duration;

use pg_platform::file::beside;

/// How long [`Lock::take`] waits for another holder: about five seconds, as the connections' `busy_timeout`.
const LOCK_TRIES: u32 = 100;
const LOCK_WAIT: Duration = Duration::from_millis(50);

/// A lock on a database file, held by a process that is about to change the file's layout, so that two such processes
/// go one after the other and the second finds the first's result. The lock is a file beside the database
/// (`<database>.lock`), which stays after the lock is released. It holds between processes of one operating system; a
/// folder shared between two (Windows and WSL) needs the database's own transactions to be the guard.
#[derive(Debug)]
pub struct Lock {
    _held: File,
}

impl Lock {
    /// The lock on the database at `path`, waiting up to `LOCK_TRIES` tries for another holder to let go. `None` when
    /// another still holds it after that.
    pub fn take(path: &Path) -> io::Result<Option<Lock>> {
        let file = OpenOptions::new().create(true).write(true).truncate(false).open(beside(path, ".lock"))?;
        for attempt in 0..LOCK_TRIES {
            match file.try_lock() {
                Ok(()) => return Ok(Some(Lock { _held: file })),
                Err(TryLockError::Error(e)) => return Err(e),
                Err(TryLockError::WouldBlock) if attempt + 1 < LOCK_TRIES => std::thread::sleep(LOCK_WAIT),
                Err(TryLockError::WouldBlock) => {}
            }
        }
        Ok(None)
    }
}

/// A hold on a database file that a process has open: `<database>.live`, which stays after the hold is released. A
/// process that has the file open (a catalog, a cache, for as long as it is open) holds it shared, and so does nothing
/// to stop another such process. A process that would move the file aside or delete it (a catalog's rebuild, a cache
/// started over) takes it exclusively, which fails while any other process or connection holds it. The contents are
/// the transactions' business: this is about the file itself, which only a rebuild or a start over replaces. It holds
/// between processes of one operating system, as [`Lock`] does.
#[derive(Debug)]
pub struct Live {
    _held: File,
}

impl Live {
    /// The shared hold on the database at `path`, for as long as the caller keeps it. It waits up to `LOCK_TRIES` tries
    /// while an exclusive holder (a rebuild) lets go. `None` when one still holds the file after that.
    pub fn shared(path: &Path) -> io::Result<Option<Live>> {
        let file = open_live(path)?;
        for attempt in 0..LOCK_TRIES {
            match file.try_lock_shared() {
                Ok(()) => return Ok(Some(Live { _held: file })),
                Err(TryLockError::Error(e)) => return Err(e),
                Err(TryLockError::WouldBlock) if attempt + 1 < LOCK_TRIES => std::thread::sleep(LOCK_WAIT),
                Err(TryLockError::WouldBlock) => {}
            }
        }
        Ok(None)
    }

    /// The exclusive hold on the database at `path`, for a change to the file itself. It doesn't wait: `None` when any
    /// other process or connection holds the file, which then can't be changed.
    pub fn exclusive(path: &Path) -> io::Result<Option<Live>> {
        let file = open_live(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Live { _held: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => Err(e),
        }
    }
}

/// The `.live` file beside the database at `path`, made if it isn't there.
fn open_live(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).write(true).truncate(false).open(beside(path, ".live"))
}
