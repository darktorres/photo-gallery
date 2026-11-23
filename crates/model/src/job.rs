//! What a job reports and why it failed: the words are the app's, so these carry the facts alone.

use alloc::string::String;

/// What the library's background job or its writer changed, for an owner that reads the new state on its own thread
/// instead of polling. An event says only that something changed: the state is read from the job's and the writer's
/// getters. (The library's passes publish these; the enum lives in the model so the writer and the job share it without
/// the writer depending on the job.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The pass's progress, or the pause.
    Progress,
    /// A folder scan changed the catalog (`Runner::generation`).
    Scanned,
    /// An import or border pass analyzed something (`Runner::generation`).
    Imported,
    /// A round ended (`Runner::rounds`).
    Round,
    /// The writer's queue or its latest failure (`Writer::status`).
    Writes,
    /// A file was turned (`Writer::turned`).
    Turned,
}

/// Why a job failed on a file, for the app to word: an error, or a panic the job caught on it. The job holds no words;
/// the app says which it was. (`pg-jobs` builds it from a failure: `impl From<&Failed> for Cause`.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// The error's text, its causes after it.
    Error(String),
    /// The message of the panic.
    Panicked(String),
}

/// The text alone, without saying which it is (a log line, the command line).
impl core::fmt::Display for Cause {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Cause::Error(text) | Cause::Panicked(text) => f.write_str(text),
        }
    }
}
