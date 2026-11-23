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
//! - The outcomes the front ends word: a pass's progress (`progress`), why a folder or a file is refused (`refusal`), the
//!   Yandex search's and save's (`yandex`), and the notices (`notice`).
//! - The plain data the catalog and the jobs hand out: the inbox's entries and counts (`inbox`), the folder and file
//!   failures of a ledger (`ledger`), the filter bar's counts (`facets`), a removal's outcome (`removal`), a job's
//!   events and causes (`job`), a write's error (`write`), and the thumbnail cache's size limit (`cache`).

// no_std: the model reads no clock, thread, file, socket, process or environment, by construction. The `specta` feature
// (the web client's TypeScript types) needs std: its derive expands to `::std` paths.
#![cfg_attr(not(feature = "specta"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;
#[cfg(any(feature = "specta", test))]
extern crate std;

pub mod analysis;
pub mod cache;
pub mod date;
pub mod details;
pub mod detection;
pub mod error;
pub mod facets;
pub mod features;
pub mod format;
pub mod grid;
pub mod ids;
pub mod image;
pub mod inbox;
pub mod job;
pub mod ledger;
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
pub mod removal;
pub mod rules;
pub mod size;
pub mod state;
pub mod tree;
pub mod turns;
pub mod versions;
pub mod view;
pub mod write;
pub mod yandex;
