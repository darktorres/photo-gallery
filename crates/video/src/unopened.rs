//! The error of a video file that couldn't be opened (a leaf: `ffi` returns it, the crate's callers look for it).

/// The video's file couldn't be opened, or its size read: an I/O failure (an offline drive, a file in use, a file gone),
/// not a video that doesn't decode. A frame's or a probe's error is this one, under the context of the open, when that
/// is why it failed (`anyhow::Error::downcast`).
#[derive(Debug)]
pub struct Unopened(pub std::io::Error);

impl std::fmt::Display for Unopened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// The I/O error's own text is this one's, so its source is the I/O error's source (a chain doesn't say it twice).
impl std::error::Error for Unopened {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}
