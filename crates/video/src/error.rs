//! What a video couldn't be read, probed or shown for.

use std::path::PathBuf;

use crate::unopened::Unopened;

/// What a video couldn't be read, probed or shown for. A file that can't be opened is [`Error::Open`], which the render
/// tells apart from a video that doesn't decode (the file may be readable later).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file couldn't be opened, or its size read: an I/O failure (an offline drive, a file in use, a file gone).
    #[error("opening {}", path.display())]
    Open {
        path: PathBuf,
        #[source]
        source: Unopened,
    },
    /// FFmpeg couldn't be loaded: the library is missing, or its message says what it looked for.
    #[error("{0}")]
    Library(String),
    /// A video the stream of which can't be used: no video stream, no decoder, no picture size, no frame.
    #[error("{0}")]
    Stream(String),
    /// A call of FFmpeg, or an allocation, that failed: its message is what failed.
    #[error("{0}")]
    Ffmpeg(String),
}

/// The result of a video call.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// A stream that can't be used, as `message`.
    pub(crate) fn stream(message: impl Into<String>) -> Error {
        Error::Stream(message.into())
    }

    /// A failed call of FFmpeg, as `message`.
    pub(crate) fn ffmpeg(message: impl Into<String>) -> Error {
        Error::Ffmpeg(message.into())
    }
}
