//! The domain data of the app: formats, media kinds, civil dates and the path rules (pure, WASM-safe: no file,
//! catalog, decoder or platform calls). The library crates above it use these types; the web client will too.

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod date;
pub mod format;
pub mod media;
pub mod path;
