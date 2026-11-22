//! What the codecs couldn't do: a decode, an encode or a resize, and the inputs that can't be used.

/// What a decode, an encode or a resize couldn't do. Each variant's text is the step that failed, and the decoder's or
/// encoder's own error is its source (`pg_model::error::chain` writes both).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A decoder's step failed on the bytes (or on the header's sizes).
    #[error("{step}")]
    Decode {
        step: String,
        #[source]
        source: Source,
    },
    /// An encoder's step failed.
    #[error("{step}")]
    Encode {
        step: String,
        #[source]
        source: Source,
    },
    /// The resizer's step failed.
    #[error("{step}")]
    Resize {
        step: String,
        #[source]
        source: Source,
    },
    /// An input that can't be used (a size that overflows, a crop outside the picture, a format with no decoder),
    /// with no error under it: the text says which.
    #[error("{0}")]
    Invalid(String),
}

/// The codecs' result.
pub type Result<T> = std::result::Result<T, Error>;

/// The error under a [`Error`]: a decoder's, an encoder's or the resizer's own.
pub type Source = Box<dyn std::error::Error + Send + Sync + 'static>;

impl Error {
    /// A decoder's own error, its text as the step (a bare `?` on it: the text is the decoder's, as it was).
    pub(crate) fn decoder<E>(error: E) -> Error
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Error::Decode { step: error.to_string(), source: Box::new(error) }
    }

    /// An encoder's own error, its text as the step.
    pub(crate) fn encoder<E>(error: E) -> Error
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Error::Encode { step: error.to_string(), source: Box::new(error) }
    }

    /// A decoder's failure at `step`.
    pub(crate) fn decode(step: impl Into<String>, source: impl Into<Source>) -> Error {
        Error::Decode { step: step.into(), source: source.into() }
    }

    /// An encoder's failure at `step`.
    pub(crate) fn encode(step: impl Into<String>, source: impl Into<Source>) -> Error {
        Error::Encode { step: step.into(), source: source.into() }
    }

    /// The resizer's failure at `step`.
    pub(crate) fn resize(step: impl Into<String>, source: impl Into<Source>) -> Error {
        Error::Resize { step: step.into(), source: source.into() }
    }

    /// An input that can't be used, as `message`.
    pub(crate) fn invalid(message: impl Into<String>) -> Error {
        Error::Invalid(message.into())
    }
}
