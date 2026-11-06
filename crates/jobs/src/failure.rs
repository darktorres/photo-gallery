//! The failures a pass reports: a file or folder it couldn't process, and why. A pass keeps the first few for its
//! stats and counts the rest.

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
