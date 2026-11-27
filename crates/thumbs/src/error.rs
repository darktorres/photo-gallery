//! The thumbs pass's error type. It sits in its own module, so the pass's modules (`pending`, and the crate root that
//! calls it) both depend on it, and none depends on the root.

use pg_jobs::failure::Failed as JobFailed;
use pg_render::maker::MakeError;

/// What a thumbnail or placeholder pass couldn't do: the catalog failed, the cache couldn't store or couldn't be read, a
/// file couldn't be read (its read's error), a thumbnail couldn't be made (`Make`), the maker couldn't be started
/// (`Render`), or the pipeline failed. The cache's two are told apart where it is called (section 10, D7): its stores
/// are one round error, its reads another, each with its own notice.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The catalog failed (see `pg_catalog::Error`).
    #[error(transparent)]
    Catalog(#[from] pg_catalog::Error),
    /// The thumbnail cache couldn't store what the pass made, or keep its marks (`put_batch`, `checked`, `shrink`).
    #[error(transparent)]
    CacheStore(pg_thumbcache::Error),
    /// The thumbnail cache couldn't be read (the pending work's marks and entries, its totals, a cached thumbnail).
    #[error(transparent)]
    CacheRead(pg_thumbcache::Error),
    /// The maker couldn't make a picture of a file (see `pg_render::MakeError`).
    #[error(transparent)]
    Make(#[from] MakeError),
    /// The maker couldn't be started (see `pg_render::Error`).
    #[error(transparent)]
    Render(#[from] pg_render::Error),
    /// A file couldn't be read: its words, and the I/O error under them.
    #[error("{what}")]
    Read {
        /// The file, in words.
        what: String,
        /// The I/O error under the read.
        #[source]
        source: std::io::Error,
    },
    /// The pass's pipeline failed (see `pg_jobs::failure::Failed`).
    #[error(transparent)]
    Pipeline(#[from] JobFailed),
}

/// The result of a thumbnail or placeholder pass's steps.
pub type Result<T> = std::result::Result<T, Error>;

/// A pass that ended with an error (section 10, D6): the error, and what the pass did before it (its ledger outcome, its
/// unreadable count, its failures), which its caller reports in every case. Before the pass's setup, the stats are empty.
pub struct Stopped<S> {
    /// Why the pass ended.
    pub error: Error,
    /// What it did before.
    pub stats: S,
}

impl<S> std::fmt::Debug for Stopped<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stopped").field("error", &self.error).finish_non_exhaustive()
    }
}

impl<S> std::fmt::Display for Stopped<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.alternate() { write!(f, "{:#}", self.error) } else { write!(f, "{}", self.error) }
    }
}

impl<S> std::error::Error for Stopped<S> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.error)
    }
}

impl<S> From<Stopped<S>> for Error {
    fn from(stopped: Stopped<S>) -> Error {
        stopped.error
    }
}
