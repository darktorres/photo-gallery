//! Where a search or a save gets its Yandex calls, and why they fail. A leaf: its signatures use only `pg_yandex` types
//! and its own error, so the parent (`yandex`) and its children `images` and `replay` (which implements the trait for a
//! recorded session) depend on it, and it depends on none of them.

use pg_model::yandex::RequestError;
use pg_yandex::{OtherSize, Similar, Upload, YandexError};

/// Why a call of a search or a save failed: the client's error, or one of the app's own cases.
#[derive(Clone, Debug, thiserror::Error)]
pub enum SourceError {
    #[error(transparent)]
    Yandex(#[from] YandexError),
    /// A result file that is not an image this app decodes (the reason is what failed).
    #[error("not an image this app can decode: {0}")]
    Undecodable(&'static str),
    /// A recorded session (`replay`) that can't answer: why.
    #[error("recorded Yandex session: {0}")]
    Recorded(String),
}

impl From<SourceError> for RequestError {
    /// The error as data, for the front ends to word.
    fn from(e: SourceError) -> Self {
        match e {
            SourceError::Yandex(e) => e.into(),
            SourceError::Undecodable(reason) => RequestError::Undecodable(reason.to_string()),
            SourceError::Recorded(error) => RequestError::Recorded(error),
        }
    }
}

/// Where a search or a save gets its Yandex calls: the real client, or a recorded session that stands in for it.
/// Each search and each save makes its own, on its thread.
pub trait Source {
    fn upload(&mut self, image: &[u8]) -> Result<Upload, SourceError>;
    fn other_sizes(&mut self, up: &Upload) -> Result<Vec<OtherSize>, SourceError>;
    fn similar(&mut self, up: &Upload, max_pages: u32) -> Result<Similar, SourceError>;
}
