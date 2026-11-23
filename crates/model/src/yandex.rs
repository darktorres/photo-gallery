//! The outcomes of a Yandex reverse image search and of the save of its results, as data: `pg-app` builds them,
//! and the front ends word them (`pg_wording::yandex_status`). The errors keep what the user reads (a code, a
//! count, the lower layers' text); the paths are as the user sees them.

use alloc::string::String;

/// Why a request to Yandex (or the recorded session standing in for it) failed. `pg_yandex`'s error, as data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// Yandex asked for an anti-robot check.
    Captcha,
    /// An HTTP status other than success.
    Status(u16),
    /// A connection, TLS or timeout failure: the transport's text.
    Transport(String),
    /// An answer without the data the app reads: which part was missing (the front ends word each one).
    Unexpected(UnexpectedReason),
    /// A recorded session that can't answer: why.
    Recorded(String),
    /// A result's file that is not an image this app decodes: what failed.
    Undecodable(String),
}

/// What an answer from Yandex lacked, or what the app's own state lacked, as a typed reason: the front ends word each one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnexpectedReason {
    /// The upload's answer is not JSON.
    UploadNotJson,
    /// The upload's answer has no `cbir_id`.
    UploadNoCbirId,
    /// The upload's answer has no URL for the original picture.
    UploadNoOriginal,
    /// The results page has no search state in it.
    ResultsPageNoState,
    /// The search state is not JSON.
    ResultsStateNotJson,
    /// The similar-images answer is not JSON.
    SimilarNotJson,
    /// The similar-images answer has no result list.
    SimilarNoResults,
    /// The result's file is not an image.
    NotAnImage,
    /// The picture is larger than the download limit.
    TooLarge,
    /// The search has no result of that id.
    NoResult,
    /// There is no search to take a result from.
    NoSearch,
    /// Nothing is on screen to save.
    NoSearchOnScreen,
    /// The request was never made (the app's stand-in before a search).
    NotCalled,
}

/// Where a search or a save is, while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The search has begun.
    Starting,
    /// The photo is being uploaded.
    Uploading,
    /// The other sizes are being found.
    OtherSizes,
    /// The similar images are being found.
    Similar,
    /// A save is about to write `files` files.
    Saving {
        /// How many files the save will write.
        files: usize,
    },
    /// The `n`th of `of` files (counted from 1) is being downloaded.
    SavingFile {
        /// Which file this is, counted from 1.
        n: usize,
        /// How many files the save has.
        of: usize,
    },
}

/// A search that ended with a note: the similar images stopped early or failed. The other sizes are still found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimilarNote {
    /// The list stopped after its first page: `shown` items were read before `error`.
    Stopped {
        /// How many items were read before the error.
        shown: usize,
        /// The error that stopped the list.
        error: RequestError,
    },
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
    Download {
        /// The file's address.
        url: String,
        /// Why the download failed.
        error: RequestError,
    },
    /// A downloaded file could not be written to `path` (as the user sees it). `error` is the error's text, not the error:
    /// an `io::Error` is neither `Clone` nor `Eq`, and this type derives both.
    Write {
        /// The file's address.
        url: String,
        /// Where the file was to be written, as the user sees it.
        path: String,
        /// The write's error, as text.
        error: String,
    },
}

/// What a save did: into which folder (as the user sees it), how many files were written and how many failed, and the
/// first failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    /// The folder the files went to, as the user sees it.
    pub folder: String,
    /// How many files were written.
    pub written: usize,
    /// How many files failed.
    pub failed: usize,
    /// The first failure, if any failed.
    pub first_error: Option<SaveFailure>,
}

/// A step's outcome, as the front end reads it on each poll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum YandexStatus {
    /// A step in progress.
    Working(Phase),
    /// The search ended. The note is there when the similar images stopped early or failed.
    Searched(Option<SimilarNote>),
    /// The search ended without results.
    SearchFailed(SearchError),
    /// The save ended.
    Saved(Saved),
    /// The search or the save ended in a panic (the panic is in the log). Not a stop the user asked for.
    Panicked,
}
