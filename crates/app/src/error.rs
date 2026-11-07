//! The app's errors. A caller that branches gets a variant; the rest carry the text of what failed, causes included
//! (the image, file and cache layers report anyhow errors, whose text is taken here as it is).

use std::fmt::Display;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Catalog(#[from] pg_catalog::Error),
    #[error("unknown file {0}")]
    UnknownFile(i64),
    /// A write of the app's into the file was still in progress when the wait for it ended.
    #[error("file {0} is being rewritten")]
    Rewriting(i64),
    /// The file has no placeholder yet: it isn't imported.
    #[error("not imported yet")]
    NotImported,
    /// The original can't be read (offline, or not decodable): why.
    #[error("original unavailable: {0}")]
    Original(String),
    /// Anything else a lower layer failed with.
    #[error("{0}")]
    Failed(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The error of a lower layer, as its text (`{:#}`: the causes included).
pub fn failed(e: impl Display) -> Error {
    Error::Failed(format!("{e:#}"))
}
