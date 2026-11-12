//! The domain data of the app (pure, WASM-safe: no file, catalog, decoder or platform calls). The library crates above
//! it use these types; the web client will too.
//!
//! - A file and its analysis: the record (`analysis`), the borders found (`detection`), the EXIF orientation and the
//!   catalog's turns (`orientation`, `turns`), PDQ (`pdq`), formats and media kinds (`format`, `media`), civil dates
//!   (`date`), ratings (`rating`), and the numeric conversions (`num`).
//! - Names and paths: exact names (`name`), the path rules (`rules`), the catalog's row ids (`ids`) and the passes' work
//!   lists (`path`).
//! - What the catalog's reads and the front ends exchange: the grid's entries (`grid`), the folder tree (`tree`), a
//!   file's details (`details`), the images of a file and their keys (`image`), what a reader checks a file against
//!   (`state`), the query (`query`), the listing and the saved views (`view`), and the queued writes (`write`).
//! - The outcomes the front ends word: a pass's progress (`progress`), why a folder is refused (`refusal`), the Yandex
//!   search's and save's (`yandex`), and the notices (`notice`).

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
pub mod size;
pub mod state;
pub mod tree;
pub mod turns;
pub mod view;
pub mod write;
pub mod yandex;
