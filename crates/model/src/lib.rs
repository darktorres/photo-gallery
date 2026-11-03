//! The domain data of the app: the analysis record, border detection, EXIF orientation, PDQ, formats, media kinds,
//! civil dates and the path rules (pure, WASM-safe: no file, catalog, decoder or platform calls). The library crates
//! above it use these types; the web client will too.

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod analysis;
pub mod date;
pub mod detection;
pub mod format;
pub mod ids;
pub mod media;
pub mod orientation;
pub mod path;
pub mod pdq;
pub mod rating;
pub mod state;
pub mod turns;
pub mod write;
