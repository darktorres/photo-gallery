//! The catalog's errors. Callers branch on one case, [`Error::Interrupted`]: a statement stopped by the cancel flag
//! (`Catalog::cancel_when`) or by SQLite's own interrupt. Every other failure is one of the sources the catalog reads
//! or writes through (SQLite, a stored value, a file), or [`Error::Other`]: an `anyhow` error that carries its context.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A statement stopped before it finished: the reader gave up on it (its cancel flag was set).
    #[error("interrupted")]
    Interrupted,
    /// A value read from a row that its column can't hold (a `FromSqlError`).
    #[error(transparent)]
    Value(#[from] rusqlite::types::FromSqlError),
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
