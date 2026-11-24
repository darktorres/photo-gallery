//! The job (`runner`) that runs the passes over the originals in order, for the app and for the command line (`pg_cli`),
//! where the data folders are (`layout`), and how a catalog folder is opened for them (`open_catalog`, `open_thumbs`,
//! `versions`). What both front ends do around the passes is here too, once: adding a folder (`folders`) and what
//! follows a scan (`scans`). The passes are their own crates, which the caller depends on directly: `pg-scan`, `pg-import`,
//! `pg-thumbs`, `pg-borders`, `pg-crop` and `pg-writeback`.
//!
//! The passes report what they did as values (their stats, the failures they hit), and print nothing: whoever runs
//! them decides what to show, and in what words.

#![forbid(unsafe_code)]

mod coordinator;
pub mod folders;
mod inner;
mod lane;
pub mod lanes;
pub mod layout;
mod pair_gate;
mod passes;
pub mod removals;
pub mod runner;
pub mod scans;
pub mod shared;
mod tally;

#[cfg(test)]
mod park_tests;

/// What the library couldn't do: a catalog or thumbnail-cache failure, the failure of a pass or of a lane (a lane that
/// panicked is a failure of class `Panic`), or a step of the library that failed, with the words of the step.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The catalog failed (see `pg_catalog::Error`).
    #[error(transparent)]
    Catalog(#[from] pg_catalog::Error),
    /// The thumbnail cache failed (see `pg_thumbcache::Error`).
    #[error(transparent)]
    Cache(#[from] pg_thumbcache::Error),
    /// The crop failed (see `pg_crop::Error`).
    #[error(transparent)]
    Crop(#[from] pg_crop::Error),
    /// A pass of a round failed, or a lane panicked (see `pg_jobs::failure::Failed`).
    #[error(transparent)]
    Pass(#[from] pg_jobs::failure::Failed),
    /// A step of the library failed: its words, and the error under them.
    #[error("{what}")]
    Step {
        /// The step, in words.
        what: String,
        /// The error under the step.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// The library's result.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// A step of the library that failed on `source`, with the words of the step.
    pub(crate) fn step(what: impl Into<String>, source: impl std::error::Error + Send + Sync + 'static) -> Error {
        Error::Step { what: what.into(), source: Box::new(source) }
    }
}

impl From<Error> for pg_jobs::failure::Failed {
    /// A failure of a pass: a pass's own failure keeps its class, anything else is an ordinary one.
    fn from(error: Error) -> pg_jobs::failure::Failed {
        match error {
            Error::Pass(failed) => failed,
            other => pg_jobs::failure::Failed::new(pg_jobs::failure::Class::Other, other),
        }
    }
}

/// The catalog of `layout` (one as SQLite opens it, `Layout::for_sqlite`), prepared for this build (`Catalog::prepare`
/// with [`versions`], under the native rules). A catalog this build can't use is moved aside and rebuilt;
/// [`pg_catalog::Catalog::rebuilt`] says so.
pub fn open_catalog(layout: &layout::Layout) -> pg_catalog::Result<pg_catalog::Catalog> {
    pg_catalog::Catalog::prepare(&layout.catalog_path(), versions(), pg_platform::native::PATH_RULES)
}

/// The thumbnail cache of `layout` (as SQLite opens it) for the catalog `catalog_id`, holding at most `cap` bytes
/// (`None`: the cache's own cap), for this build's thumbnails (`pg_model::versions::THUMBS_VERSION`). Its events go to `events`.
pub fn open_thumbs(
    layout: &layout::Layout,
    catalog_id: &str,
    cap: Option<pg_model::cache::ThumbCap>,
    events: std::sync::Arc<pg_jobs::events::Bus<pg_thumbcache::Event>>,
) -> Result<pg_thumbcache::ThumbCache> {
    pg_thumbcache::ThumbCache::open(&layout.thumbs_path(), catalog_id, cap, pg_model::versions::THUMBS_VERSION, events)
        .map_err(Error::from)
}

/// The versions of the analyses this build makes, for `Catalog::prepare`: the library passes them when it prepares a
/// catalog for writing. The registry's (`pg_model::versions`).
pub fn versions() -> pg_catalog::Versions {
    pg_catalog::Versions::current()
}
