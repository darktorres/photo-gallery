//! The library core: catalog, decoding, analysis (tiers 0–1) and thumbnails. No UI dependency (PLAN §7).

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod analyze;
pub mod animation;
pub mod borders;
pub mod browse;
pub mod catalog;
mod codecs;
pub mod crop;
pub mod decode;
pub mod format;
pub mod layout;
pub mod media;
pub mod metadata;
pub mod orientation;
pub mod paths;
pub mod pdq;
pub mod pdq_index;
pub mod query;
pub mod resize;
mod rewrite;
pub mod rotate;
pub mod thumbs;
pub mod video;
pub mod yandex;
