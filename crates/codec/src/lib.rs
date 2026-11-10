//! The pixels in and out: every format's decoder (upright or as stored, at the smallest DCT scale that keeps the
//! size a caller needs), the resizer, the animation frames, and the re-encode of a crop. The codecs are the one place
//! the image decoders and encoders are linked (the bans in deny.toml). No file, catalog or metadata reads: the caller
//! gives the bytes and the EXIF orientation.

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod animation;
mod codecs;
pub mod decode;
pub mod encode;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub mod pixels;
pub mod resize;

pub use codecs::limits::max_decode_bytes;
