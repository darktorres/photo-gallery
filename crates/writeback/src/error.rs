//! What the writer couldn't do to a file, or its queue. The offline rule and the copy's recovery read the chain of an error:
//! the I/O error the file system gave (wherever it is under the steps), and the marker of a copy that may have started
//! ([`Error::CopyStarted`]). The chain walks are here, so both are read the same way everywhere.

use std::error::Error as StdError;

/// The error under a [`Error::CopyStarted`] or a step: any error of the writer's dependencies.
pub(crate) type Source = Box<dyn StdError + Send + Sync + 'static>;

/// What the writer couldn't do.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The catalog failed (see `pg_catalog::Error`).
    #[error(transparent)]
    Catalog(#[from] pg_catalog::Error),
    /// ExifTool failed (see `pg_exiftool::Error`). Not transparent: the I/O error under it is in the chain.
    #[error("{0}")]
    Exiftool(#[from] pg_exiftool::Error),
    /// The edit couldn't turn the file (see `pg_edit::Error`).
    #[error("{0}")]
    Edit(#[from] pg_edit::Error),
    /// The thumbnail cache failed (see `pg_thumbcache::Error`), when a rewritten file's thumbnail followed it.
    #[error("{0}")]
    Cache(#[from] pg_thumbcache::Error),
    /// A file-system step failed: its words, and the I/O error under them.
    #[error("{what}")]
    Io {
        /// The step, in words.
        what: String,
        /// The I/O error under the step.
        #[source]
        source: std::io::Error,
    },
    /// A failure after the copy into the original may have started: the temp it was made in, as the user sees its path.
    #[error("copying {temp} into the original")]
    CopyStarted {
        /// The temp the copy was made from, as the user sees its path.
        temp: String,
        /// The error that ended the copy.
        #[source]
        source: Source,
    },
    /// A file the writer can't write as asked (an unknown format, a rating that didn't read back), with the words that say
    /// why.
    #[error("{0}")]
    Invalid(String),
}

/// The writer's result.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// A file-system step `what` failed on `source`.
    pub(crate) fn io(what: impl Into<String>, source: std::io::Error) -> Error {
        Error::Io { what: what.into(), source }
    }

    /// A failure after the copy into the original may have started, in the temp `temp`.
    pub(crate) fn copy_started<E>(temp: impl Into<String>, source: E) -> Error
    where
        E: StdError + Send + Sync + 'static,
    {
        Error::CopyStarted { temp: temp.into(), source: Box::new(source) }
    }

    /// A file the writer can't write as asked, as `words`.
    pub(crate) fn invalid(words: impl Into<String>) -> Error {
        Error::Invalid(words.into())
    }
}

/// A job of the writer fails with an ordinary failure (a panic in one comes as the guard's own, and is not this).
impl From<Error> for pg_jobs::failure::Failed {
    fn from(error: Error) -> pg_jobs::failure::Failed {
        pg_jobs::failure::Failed::new(pg_jobs::failure::Class::Other, error)
    }
}

/// The first I/O error in `error`'s chain, as its kind: the file system's answer, wherever the steps wrapped it.
pub(crate) fn io_kind(error: &(dyn StdError + 'static)) -> Option<std::io::ErrorKind> {
    let mut cause = Some(error);
    while let Some(next) = cause {
        if let Some(io) = next.downcast_ref::<std::io::Error>() {
            return Some(io.kind());
        }
        cause = next.source();
    }
    None
}

/// The temp of a copy that may have started, from the first [`Error::CopyStarted`] in `error`'s chain.
pub(crate) fn copy_started_temp(error: &(dyn StdError + 'static)) -> Option<String> {
    let mut cause = Some(error);
    while let Some(next) = cause {
        if let Some(Error::CopyStarted { temp, .. }) = next.downcast_ref::<Error>() {
            return Some(temp.clone());
        }
        cause = next.source();
    }
    None
}
