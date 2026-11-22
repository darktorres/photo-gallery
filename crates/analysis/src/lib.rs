//! What the import computes from a file: its kind, size, orientation, frames and dates from the header and the
//! container (no decode), and the duplicate hashes, the border detector and the PDQ hasher the later passes use. No
//! catalog and no file writes here, and no UI.

pub mod analyze;
pub mod borders;
pub mod pdq;

/// What the import's analysis of a file couldn't do: the codec refused the bytes, the video couldn't be read, a file
/// was of no format the import takes, or a size didn't fit its 4 bytes.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The pixel codec refused the bytes (see `pg_codec::Error`).
    #[error(transparent)]
    Codec(#[from] pg_codec::Error),
    /// A video couldn't be read (see `pg_video::Error`).
    #[error(transparent)]
    Video(#[from] pg_video::Error),
    /// A file of no format the import takes, or a picture the detector can't read, with the words that say which.
    #[error("{0}")]
    Invalid(&'static str),
    /// A file that couldn't be read: its path, and the I/O error under it.
    #[error("reading {}", path.display())]
    Read {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A side of a picture over `u32::MAX`, which the hash and the detection take as 4 bytes.
    #[error("{what}")]
    Size {
        what: &'static str,
        #[source]
        source: std::num::TryFromIntError,
    },
}

/// The analysis's result.
pub type Result<T> = std::result::Result<T, Error>;

/// Converts a side of a picture, as `what` names it, into the 4 bytes the hash and the detection take.
pub(crate) fn side(n: usize, what: &'static str) -> Result<u32> {
    u32::try_from(n).map_err(|source| Error::Size { what, source })
}
