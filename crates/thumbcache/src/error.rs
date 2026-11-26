//! What the thumbnail cache couldn't do: a SQLite step (the file, its locks, its schema), a file-system step, a cache in a
//! state it can't be used in (not open, closed, its writer stopped), or a stored value outside what the schema holds.

/// The error under a [`Error::Step`].
pub(crate) type Inner = Box<dyn std::error::Error + Send + Sync + 'static>;

/// What the thumbnail cache couldn't do.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A step of opening, reading or writing the cache failed, with the words of the step and the error under them.
    #[error("{what}")]
    Step {
        /// The step, in words.
        what: String,
        /// The error under the step.
        #[source]
        source: Inner,
    },
    /// A SQLite call that has no step of its own (its text is SQLite's). The SQLite error is the source, not forwarded, so
    /// a damage check finds it (`pg_dbfile::Damage::in_chain`).
    #[error("{0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A value read from a row that its column can't hold (a `FromSqlError`).
    #[error("{0}")]
    Value(#[from] rusqlite::types::FromSqlError),
    /// A file-system call that has no step of its own.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The cache can't be used in its current state (not open, closed, its writer stopped), or a stored value is outside
    /// what the schema holds: the words say which.
    #[error("{0}")]
    Invalid(String),
}

/// The result of a thumbnail cache call.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// The step `what` failed on `source`.
    pub(crate) fn step<E>(what: impl Into<String>, source: E) -> Error
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Error::Step { what: what.into(), source: Box::new(source) }
    }

    /// Whether this is one row's value that its column can't hold (a stored type or range the read can't take), not a
    /// failure of the cache: its caller treats the row as a miss (section 10, D8).
    pub fn is_value(&self) -> bool {
        match self {
            Error::Value(_) => true,
            Error::Sqlite(e) => matches!(
                e,
                rusqlite::Error::InvalidColumnType(..)
                    | rusqlite::Error::FromSqlConversionFailure(..)
                    | rusqlite::Error::IntegralValueOutOfRange(..)
            ),
            Error::Step { .. } | Error::Io(_) | Error::Invalid(_) => false,
        }
    }

    /// A state the cache can't be used in, or a stored value out of range, as `message`.
    pub(crate) fn invalid(message: impl Into<String>) -> Error {
        Error::Invalid(message.into())
    }
}
