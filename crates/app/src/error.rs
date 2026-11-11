//! The app's errors. A caller that branches gets a variant; the rest carry the text of what failed, causes included
//! (the image, file and cache layers report anyhow errors, whose text is taken here as it is).

use std::fmt::Display;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Catalog(#[from] pg_catalog::Error),
    #[error("unknown file {}", .0.get())]
    UnknownFile(pg_model::ids::FileId),
    /// A write of the app's into the file was still in progress when the wait for it ended.
    #[error("file {} is being rewritten", .0.get())]
    Rewriting(pg_model::ids::FileId),
    /// The file has no placeholder yet: it isn't imported.
    #[error("not imported yet")]
    NotImported,
    /// The original can't be read (offline, gone): why.
    #[error("original unavailable: {0}")]
    Original(String),
    /// The original was read and doesn't decode (damaged, cut short, a format the decoder can't take): why.
    #[error("the original doesn't decode: {0}")]
    Undecodable(String),
    /// A folder not added to the library, because the scanner refuses it (`LibraryService::add_folder`): the folder, and
    /// why. The front end words it (the notice `FolderRefused`); this text names the reason by its code.
    #[error("{} can't be added: {reason}", pg_platform::native::PATH_RULES.display(folder))]
    FolderRefused { folder: pg_model::rules::Path, reason: pg_model::refusal::RefusedReason },
    /// A folder not added because its path can't be opened, or isn't one. The front end words it (the client's
    /// `Message::PathUnreadable` and `Message::NotAPath`).
    #[error(transparent)]
    FolderPath(pg_platform::names::NormalizeError),
    /// Anything else a lower layer failed with.
    #[error("{0}")]
    Failed(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Why a catalog read gave no result (`crate::reader`): the catalog's error, or one of the reader's own two cases.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error(transparent)]
    Catalog(#[from] pg_catalog::Error),
    /// The read panicked (the panic hook logged it); the reader goes on serving the others.
    #[error("the catalog read failed (an internal error)")]
    Panicked,
    /// The reader's connection couldn't be opened: why.
    #[error("the catalog can't be opened: {0}")]
    Unopened(String),
}

impl From<ReadError> for Error {
    fn from(e: ReadError) -> Error {
        match e {
            ReadError::Catalog(e) => Error::Catalog(e),
            e => failed(e),
        }
    }
}

/// The error of a lower layer, as its text (`{:#}`: the causes included).
pub fn failed(e: impl Display) -> Error {
    Error::Failed(format!("{e:#}"))
}

/// The original can't be read (offline, gone): [`Error::Original`], with the lower layer's text (`{:#}`: the causes
/// included). Its `Display` adds "original unavailable: ", so the text given here doesn't say it.
pub fn original(e: impl Display) -> Error {
    Error::Original(format!("{e:#}"))
}

/// What a maker's failure is for the app: the original unavailable when it couldn't be read, else it doesn't decode.
pub fn made_error(e: pg_render::maker::MakeError) -> Error {
    match e {
        pg_render::maker::MakeError::Unreadable(e) => original(e),
        pg_render::maker::MakeError::Undecodable(e) => Error::Undecodable(format!("{e:#}")),
    }
}
