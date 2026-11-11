//! The catalog's errors. Callers branch on a few cases: [`Error::Interrupted`], a statement stopped by the cancel flag
//! (`Catalog::cancel_when`) or by SQLite's own interrupt; [`Error::ScanBusy`], a walk refused because another walks an
//! overlapping folder (`Catalog::begin_scan`); and [`Error::is_busy`], the catalog locked by another connection past
//! the busy timeout. Every other failure is one of the sources the catalog reads or writes through (SQLite, a stored
//! value, a file), or [`Error::Other`]: an `anyhow` error that carries its context.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A statement stopped before it finished: the reader gave up on it (its cancel flag was set).
    #[error("interrupted")]
    Interrupted,
    /// A value read from a row that its column can't hold (a `FromSqlError`).
    #[error(transparent)]
    Value(#[from] rusqlite::types::FromSqlError),
    /// The catalog stores its paths under other rules than this build's: its names and paths don't read as this
    /// build's. It is a valid catalog of another platform, so it is not moved aside: open it on the platform that made
    /// it, or move it aside by hand and scan again.
    #[error(
        "catalog {path}: {detail}, this build uses the {wanted} path rules: open it on the platform that made it, or move \
         it aside and scan again"
    )]
    PathRules { path: String, detail: String, wanted: &'static str },
    /// A walk of `root` was refused: a walk of a folder that overlaps it (one is inside the other) holds an unexpired
    /// lease (`Catalog::begin_scan`), `holder` says whose. Nothing was written. It can be tried again once that walk ends,
    /// or its lease expires (`SCAN_LEASE_SECS` without a heartbeat).
    #[error("a scan of {root} is running ({holder})")]
    ScanBusy { root: String, holder: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// The catalog's result. `E` defaults to [`Error`], so the functions are written `Result<T>`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<rusqlite::Error> for Error {
    /// SQLite's interrupt is [`Error::Interrupted`]; any other statement error is kept with its context.
    fn from(e: rusqlite::Error) -> Error {
        if e.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted) {
            Error::Interrupted
        } else {
            Error::Other(e.into())
        }
    }
}

impl Error {
    /// Whether this is SQLite's `SQLITE_BUSY` (another connection holds the lock, past this one's busy timeout),
    /// anywhere in its chain: a statement error is kept inside [`Error::Other`], under whatever context was added. It
    /// is worth trying again on the same connection. `SQLITE_LOCKED` (a conflict inside this connection) is not busy:
    /// trying it again can loop.
    pub fn is_busy(&self) -> bool {
        match self {
            Error::Other(e) => e.chain().any(|cause| {
                cause.downcast_ref::<rusqlite::Error>().and_then(rusqlite::Error::sqlite_error_code)
                    == Some(rusqlite::ErrorCode::DatabaseBusy)
                    || cause.downcast_ref::<Error>().is_some_and(Error::is_busy)
            }),
            Error::Interrupted | Error::Value(_) | Error::PathRules { .. } | Error::ScanBusy { .. } | Error::Io(_) => {
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn busy_is_found_under_any_context_and_locked_is_not_busy() {
        let failure = |code| {
            rusqlite::Error::SqliteFailure(rusqlite::ffi::Error { code, extended_code: 0 }, Some("locked".into()))
        };
        let busy: Error = failure(rusqlite::ErrorCode::DatabaseBusy).into();
        assert!(busy.is_busy());
        let wrapped = Error::Other(
            Err::<(), _>(Error::from(failure(rusqlite::ErrorCode::DatabaseBusy)))
                .context("posting")
                .context("the inbox")
                .unwrap_err(),
        );
        assert!(wrapped.is_busy(), "{wrapped:#}");
        assert!(!Error::from(failure(rusqlite::ErrorCode::DatabaseLocked)).is_busy());
        assert!(!Error::Interrupted.is_busy());
    }
}
