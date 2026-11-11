//! The outcomes of a Yandex reverse image search and of the save of its results, as data (R-1): `pg-app` builds them,
//! and the front ends word them (`pg_client::text::yandex_status`). The errors keep what the user reads (a code, a
//! count, the lower layers' text); the paths are as the user sees them.

/// Why a request to Yandex (or the recorded session standing in for it) failed. `pg_yandex`'s error, as data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// Yandex asked for an anti-robot check.
    Captcha,
    /// An HTTP status other than success.
    Status(u16),
    /// A connection, TLS or timeout failure: the transport's text.
    Transport(String),
    /// An answer without the data the app reads: what was missing.
    Unexpected(String),
    /// A recorded session that can't answer: why.
    Recorded(String),
    /// A result's file that is not an image this app decodes: what failed.
    Undecodable(String),
}

/// Where a search or a save is, while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Starting,
    Uploading,
    OtherSizes,
    Similar,
    /// A save is about to write `files` files.
    Saving { files: usize },
    /// The `n`th of `of` files (counted from 1) is being downloaded.
    SavingFile { n: usize, of: usize },
}

/// A search that ended with a note: the similar images stopped early or failed. The other sizes are still found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimilarNote {
    /// The list stopped after its first page: `shown` items were read before `error`.
    Stopped { shown: usize, error: RequestError },
    /// The similar images could not be read.
    Failed(RequestError),
}

/// Why a search ended without results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchError {
    /// Refused before anything is sent: only still images are searched.
    NotStill,
    /// The photo could not be read from disk: the error's text.
    Unreadable(String),
    /// The photo is in a format Yandex may refuse (TIFF), and it could not be decoded to be sent as a JPEG.
    Undecodable(String),
    /// A request to Yandex failed before any results came (the client, the upload or the other sizes).
    Request(RequestError),
}

/// Why one save failed. The first one is kept in [`Saved`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveFailure {
    /// The client could not be made, so nothing was downloaded.
    Client(RequestError),
    /// A file could not be downloaded.
    Download { url: String, error: RequestError },
    /// A downloaded file could not be written to `path` (as the user sees it): the error's text.
    Write { url: String, path: String, error: String },
}

/// What a save did: into which folder (as the user sees it), how many files were written and how many failed, and the
/// first failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub folder: String,
    pub written: usize,
    pub failed: usize,
    pub first_error: Option<SaveFailure>,
}

/// A step's outcome, as the front end reads it on each poll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// A step in progress.
    Working(Phase),
    /// The search ended. The note is there when the similar images stopped early or failed.
    Searched(Option<SimilarNote>),
    /// The search ended without results.
    SearchFailed(SearchError),
    /// The save ended.
    Saved(Saved),
    /// The search or the save stopped at an internal error (the panic is in the log).
    Stopped,
}
