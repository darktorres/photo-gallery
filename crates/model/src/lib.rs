//! The domain data of the app: the analysis record, border detection, EXIF orientation, PDQ, formats, media kinds,
//! civil dates and the path rules (pure, WASM-safe: no file, catalog, decoder or platform calls). The library crates
//! above it use these types; the web client will too.

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod analysis;
pub mod date;
pub mod details;
pub mod detection;
pub mod format;
pub mod grid;
pub mod ids;
pub mod image;
pub mod media;
pub mod name;
pub mod notice;
pub mod num;
pub mod orientation;
pub mod path;
pub mod pdq;
pub mod progress;
pub mod query;
pub mod rating;
pub mod refusal;
pub mod rules;
pub mod state;
pub mod tree;
pub mod turns;
pub mod view;
pub mod write;
pub mod yandex;
