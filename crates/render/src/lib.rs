//! The display images (PLAN §6): the pixels the app shows or sends, made from the files. [`maker`] makes the grid's
//! thumbnails and the viewer's images in the thread's own buffers; [`display`] decodes a file once, upright, at the
//! smallest size a caller needs. Both read the EXIF orientation here and hand it to the codec, which reads none.
//! [`player`] plays an animation's frames into a [`player::Sink`] at their delays. [`transcode`] re-encodes a photo
//! as a JPEG for a service that doesn't take its format.

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

/// The version of the thumbnails this build makes: bumped by hand when a change to the decoders or the maker makes a
/// file that failed before decode now (a codec upgrade is not detected). A catalog's or a cache's mark that differs
/// (`pg_catalog::Versions::thumbs`, `pg_thumbcache::ThumbCache::open`) tries again the files whose thumbnail panicked,
/// and the next pass looks at every file again.
pub const THUMBS_VERSION: i64 = 1;

pub mod display;
pub mod maker;
pub mod player;
pub mod transcode;
