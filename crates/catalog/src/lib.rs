//! The catalog (PLAN §6): SQLite in WAL mode, the source of truth for files, analyses, pairs and ratings.
//!
//! File identity is (volume serial, NTFS file ID), so moves and renames need no re-analysis (PLAN §4). An analysis
//! row records what it was computed from: the size, mtime, catalog rotation and kind, and the pipeline version.
//! When any differ, the file is pending again (`files.pending`, kept by triggers). That is all the resumability
//! there is: every batch is one transaction, and the work list is an index.
//!
//! The thumbnails' work is tracked the same way: whatever a thumbnail depends on changing gives the file the next
//! number of a change sequence (`files.seq`, from `meta.seq`), so the thumbnail pass looks only at files changed
//! since it last looked (`thumbs::pending_in`).
// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

// The SQL macros are declared first: `macro_rules!` is textually scoped, so every module declared after
// this one can use them.
#[macro_use]
mod sql;

pub mod browse;
pub mod dups;
#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
pub mod inbox;
pub mod ledger;
pub mod pair;
pub mod writes;

mod core;
mod lifecycle;
mod types;
mod writers;

mod error;
pub use error::{Error, Result};
mod schema;

pub use self::core::{Catalog, Tx};
pub use types::{CropSource, Folder, Rebuilt, Recorded, Root, ScannedFile, Status, Versions, WorkItem, WriteTarget};
pub use writers::{AnalysisWriter, BordersWriter, ScanWriter};

#[cfg(test)]
mod test_support;
