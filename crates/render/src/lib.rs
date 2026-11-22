//! The display images: the pixels the app shows or sends, made from the files. [`maker`] makes the grid's
//! thumbnails and the viewer's images in the thread's own buffers; [`display`] decodes a file once, upright, at the
//! smallest size a caller needs. Both read the EXIF orientation here and hand it to the codec, which reads none.
//! [`player`] plays an animation's frames into a [`player::Sink`] at their delays. [`transcode`] re-encodes a photo
//! as a JPEG for a service that doesn't take its format.

#![forbid(unsafe_code)]

pub mod display;
pub mod maker;
pub mod player;
pub mod transcode;

/// What a picture couldn't be made from: the pixel codec refused the bytes, a video's frame couldn't be read, or the file
/// is of no format the app takes. A decode that fails is the file's; the front ends see it as undecodable.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The pixel codec refused the bytes, or its encoder failed (see `pg_codec::Error`).
    #[error(transparent)]
    Codec(#[from] pg_codec::Error),
    /// A video's frame couldn't be read, or converted (see `pg_video::Error`).
    #[error(transparent)]
    Video(#[from] pg_video::Error),
    /// A file or a value this crate can't make a picture of, with the words that say why.
    #[error("{0}")]
    Invalid(&'static str),
}

/// The result of a picture.
pub type Result<T> = std::result::Result<T, Error>;
