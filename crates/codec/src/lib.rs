//! The pixels in and out: every format's decoder (upright or as stored, at the smallest DCT scale that keeps the
//! size a caller needs), the resizer, the animation frames, the re-encode of a crop, and the lossless rewrites of a
//! file's own bytes (`rewrite`: a turn or a crop in the stored format). The codecs are the one place the image decoders
//! and encoders are linked (the bans in deny.toml). No file, catalog or metadata reads: the caller gives the bytes and
//! the EXIF orientation.

#![forbid(unsafe_code)]

pub mod animation;
mod codecs;
pub mod decode;
pub mod encode;
mod error;
#[cfg(any(test, feature = "fixture"))]
pub mod fixtures;
mod kernel;
pub mod pixels;
pub mod resize;
pub mod rewrite;

pub use codecs::limits::max_decode_bytes;

pub use error::{Error, Result, Source};
