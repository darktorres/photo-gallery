//! The failures a pass reports: a file or folder it couldn't process, and why. A pass keeps the first few for its
//! stats and counts the rest.

use crate::panics::Panicked;

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

/// Why a job failed on a file, for the app to word: an error, or a panic the job caught on it
/// ([`crate::panics::guard`]). The job holds no words; the app says which it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// The error's text, its causes after it (`{:#}`).
    Error(String),
    /// The message of the panic.
    Panicked(String),
}

impl Cause {
    /// The cause of `error`: a panic if it is (or was caused by) one, else its text.
    pub fn of(error: &anyhow::Error) -> Cause {
        match error.downcast_ref::<Panicked>() {
            Some(Panicked(message)) => Cause::Panicked(message.clone()),
            None => Cause::Error(format!("{error:#}")),
        }
    }
}

/// The text alone, without saying which it is (a log line, the command line).
impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Cause::Error(text) | Cause::Panicked(text) => f.write_str(text),
        }
    }
}

/// A file a job failed on (its path as the user sees it), and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileError {
    pub path: String,
    pub cause: Cause,
}
