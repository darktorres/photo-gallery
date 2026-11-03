//! The library core: catalog, decoding, analysis (tiers 0–1) and thumbnails. No UI dependency (PLAN §7).

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod browse;
pub mod catalog;
pub mod display;
pub mod layout;
pub mod maker;
pub mod pdq_index;
pub mod query;
pub mod thumbs;
pub mod yandex;
