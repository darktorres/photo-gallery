//! What the search gives and why it fails: the ids of an upload, the results, a download, and the errors.

use pg_model::format::Format;

/// Why a Yandex request failed.
#[derive(Clone, Debug, thiserror::Error)]
pub enum YandexError {
    /// Yandex asked for an anti-robot check. Nothing is retried; the user has to resolve it in a browser.
    #[error("Yandex asked for a captcha")]
    Captcha,
    /// An HTTP status other than success.
    #[error("Yandex answered HTTP {0}")]
    Status(u16),
    /// A connection, TLS or timeout failure.
    #[error("Yandex request failed: {0}")]
    Transport(String),
    /// A response that did not have the data we read: which part was missing.
    #[error("unexpected Yandex response: {0:?}")]
    Unexpected(pg_model::yandex::UnexpectedReason),
}

impl From<YandexError> for pg_model::yandex::RequestError {
    /// The error as data, for the front ends to word.
    fn from(e: YandexError) -> Self {
        use pg_model::yandex::RequestError;
        match e {
            YandexError::Captcha => RequestError::Captcha,
            YandexError::Status(status) => RequestError::Status(status),
            YandexError::Transport(error) => RequestError::Transport(error),
            YandexError::Unexpected(reason) => RequestError::Unexpected(reason),
        }
    }
}

/// The ids Yandex gives an upload. `orig_url` is the uploaded photo on Yandex's CDN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    /// The id Yandex gives the upload, for its similar search.
    pub cbir_id: String,
    /// The uploaded photo's address on Yandex's CDN.
    pub orig_url: String,
}

/// One image at another resolution. `url` is the full-size file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherSize {
    /// The size group Yandex files the image under (its label for the resolution).
    pub group: String,
    /// Its width, in pixels.
    pub width: u32,
    /// Its height, in pixels.
    pub height: u32,
    /// The address of the full-size file.
    pub url: String,
}

/// One visually similar image. `image_url` is the full-size file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarItem {
    /// Its place in the similar list.
    pub pos: u32,
    /// Its width, in pixels.
    pub width: u32,
    /// Its height, in pixels.
    pub height: u32,
    /// The address of the full-size file.
    pub image_url: String,
}

/// One result image downloaded with `Fetcher::download`.
#[derive(Debug)]
pub struct Download {
    /// The bytes of the image, as downloaded.
    pub bytes: Vec<u8>,
    /// The format of the bytes, from the bytes themselves (`Format::sniff`): any the model knows, or none. Whether the
    /// app can decode it is the app's to say.
    pub format: Option<Format>,
}

/// One page of the similar endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarPage {
    /// The items on this page.
    pub items: Vec<SimilarItem>,
    /// Whether the endpoint has a page after this one.
    pub has_next: bool,
}

/// The similar images read, in order, and why the list ends early if it does.
#[derive(Debug)]
pub struct Similar {
    /// The items read, in order.
    pub items: Vec<SimilarItem>,
    /// The error that ended the list after its first page: a captcha, a failed request, or a page that isn't what
    /// was read. `None` when the list ended by itself (an empty page, no next page) or at the page limit.
    pub stopped: Option<YandexError>,
}
