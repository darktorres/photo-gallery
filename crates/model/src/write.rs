//! What a queued write changes in a file: the kinds the write queue (`file_writes`) holds. The queue stores them as
//! their strum keys (`rating`, `rotation`).

/// What a queued write changes in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::EnumString, strum::IntoStaticStr)]
pub enum WriteKind {
    /// The file's rating (`XMP:Rating`).
    #[strum(serialize = "rating")]
    Rating,
    /// Quarter turns clockwise, in the file's pixels or its orientation tag (the queue's key is `rotation`).
    #[strum(serialize = "rotation")]
    Turns,
}

impl WriteKind {
    /// The kind as the queue stores it.
    pub fn key(self) -> &'static str {
        self.into()
    }

    pub fn from_key(key: &str) -> Option<WriteKind> {
        key.parse().ok()
    }
}
