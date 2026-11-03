//! Passes over the originals (scan, import, borders, crop, thumbnails, the writes back into the files, duplicates),
//! and the job (`runner`) that runs them in order for the app and for the command line (`pg_cli`). See PLAN §4, §8 M1.
//!
//! The passes report what they did as values (their stats, the failures they hit), and print nothing: whoever runs
//! them decides what to show, and in what words.

pub mod borders;
pub mod crop;
pub mod dups;
pub mod import;
pub mod runner;
pub mod scan;
pub mod thumbs;
pub mod writeback;

/// The catalog folder when none is given (on D:, never C:).
pub const DEFAULT_CATALOG_DIR: &str = r"D:\PhotoGallery\catalog";

/// A file or folder a pass couldn't process, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub path: String,
    pub error: String,
}

/// The failures a pass keeps for its stats. Past this many, only the count grows: a folder that fails on every file
/// doesn't fill the stats.
pub const FAILURES_KEPT: usize = 20;

/// Keeps `failure` if there is room, as a pass's stats do.
pub fn keep_failure(kept: &mut Vec<Failure>, path: impl Into<String>, error: impl std::fmt::Display) {
    if kept.len() < FAILURES_KEPT {
        kept.push(Failure { path: path.into(), error: error.to_string() });
    }
}

/// The versions of the analyses this build makes, for `Catalog::prepare`: the library passes them when it prepares a
/// catalog for writing.
pub fn versions() -> pg_catalog::Versions {
    pg_catalog::Versions {
        import: pg_analysis::analyze::IMPORT_VERSION,
        borders: pg_analysis::borders::BORDERS_VERSION,
    }
}
