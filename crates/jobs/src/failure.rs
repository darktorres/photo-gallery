//! The failures a pass reports: a file or folder it couldn't process, and why (`pg_model::notice::FileFailure`, the one
//! shape of a failed file). A pass keeps the first few for its stats and counts the rest.

use pg_model::name::Name;
use pg_model::notice::FileFailure;
use pg_model::rules::{Path, PathRules};
use pg_platform::file::ReadFailed;

use crate::panics::Panicked;

/// Whether a failure is an I/O error of the read itself, not a panic a job caught: the file is unreadable for now (a
/// locked file, an offline drive), so the next pass tries it again and it is not recorded. Any cause in the error's
/// chain may say so: a [`ReadFailed`], or an I/O error under it.
pub fn is_io_read(e: &anyhow::Error) -> bool {
    !crate::panics::is_panic(e) && e.chain().any(|cause| cause.is::<ReadFailed>() || cause.is::<std::io::Error>())
}

/// A failure to decode bytes that were already read, as its text alone: a decode's I/O error under it (a truncated
/// file's cut short) is the data's, not the read's, so [`is_io_read`] must not see it. A read's own failure keeps its
/// chain.
pub fn undecoded(e: anyhow::Error) -> anyhow::Error {
    anyhow::anyhow!("{e:#}")
}

/// The failures a pass keeps for its stats. Past this many, only the count grows: a folder that fails on every file
/// doesn't fill the stats.
pub const FAILURES_KEPT: usize = 20;

/// Keeps `failure` if there is room, as a pass's stats do.
pub fn keep_file_failure(kept: &mut Vec<FileFailure>, failure: FileFailure) {
    if kept.len() < FAILURES_KEPT {
        kept.push(failure);
    }
}

/// Keeps a failure of a plain error (not a caught panic) if there is room, as a pass's stats do.
pub fn keep_failure(kept: &mut Vec<FileFailure>, path: Path, error: impl std::fmt::Display) {
    if kept.len() < FAILURES_KEPT {
        keep_file_failure(kept, FileFailure { path, error: error.to_string(), panicked: false });
    }
}

/// The failure of the file at `path` on `error`: a caught panic's message, or the error's text.
pub fn file_failure(path: Path, error: &anyhow::Error) -> FileFailure {
    let (error, panicked) = match Cause::of(error) {
        Cause::Error(text) => (text, false),
        Cause::Panicked(message) => (message, true),
    };
    FileFailure { path, error, panicked }
}

/// The path of a file from the bytes it is stored as (`WorkList::path`, the rules' join), for a failure to name it. The
/// stored bytes parse as the rules' path; bytes that don't (none is stored, as no name holds a NUL) are kept whole as
/// one name under an empty anchor, so the failure still names the file.
pub fn stored_path(rules: PathRules, bytes: &[u8]) -> Path {
    rules.parse_bytes(bytes).unwrap_or_else(|| Path { anchor: Name::from_raw(&[]), parts: vec![Name::from_raw(bytes)] })
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
