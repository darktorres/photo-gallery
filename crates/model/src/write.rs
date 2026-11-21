//! What a queued write changes in a file: the kinds the write queue (`file_writes`) holds. The queue stores them as
//! their keys (`rating`, `rotation`: [`write_rating!`](crate::write_rating!), [`write_rotation!`](crate::write_rotation!)), and a
//! notice (`notice::Notice::WriteDropped`) by the same keys. A write's value is [`Write`].

use crate::rating::Rating;
use crate::turns::Turns;

/// The write queue's key of a rating (`file_writes.kind`): the one spelling, for Rust ([`WriteKind::key`]) and the
/// SQL (`concat!`).
#[macro_export]
macro_rules! write_rating {
    () => {
        "rating"
    };
}

/// The write queue's key of quarter turns (see [`write_rating!`](crate::write_rating!)). `rotation` is what the queue
/// stores and what a finished copy's temp file is named for (`.name.rotation.pg-write`); the variant,
/// [`WriteKind::Rotation`], is named for the same thing.
#[macro_export]
macro_rules! write_rotation {
    () => {
        "rotation"
    };
}

/// What a queued write changes in the file. Its JSON (a notice's) is its key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(into = "&'static str", try_from = "String")]
pub enum WriteKind {
    /// The file's rating (`XMP:Rating`).
    Rating,
    /// Quarter turns clockwise, in the file's pixels or its orientation tag (the queue's key is `rotation`).
    Rotation,
}

impl WriteKind {
    /// The kind as the queue stores it.
    pub fn key(self) -> &'static str {
        match self {
            WriteKind::Rating => write_rating!(),
            WriteKind::Rotation => write_rotation!(),
        }
    }

    pub fn from_key(key: &str) -> Option<WriteKind> {
        match key {
            write_rating!() => Some(WriteKind::Rating),
            write_rotation!() => Some(WriteKind::Rotation),
            _ => None,
        }
    }
}

impl From<WriteKind> for &'static str {
    fn from(kind: WriteKind) -> &'static str {
        kind.key()
    }
}

/// Why a key is refused as a write's kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteKindError {
    /// The key names no kind (see [`WriteKind::from_key`]).
    NotAKind(String),
}

impl std::fmt::Display for WriteKindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAKind(key) => write!(f, "not a write's kind: {key}"),
        }
    }
}

impl std::error::Error for WriteKindError {}

impl TryFrom<String> for WriteKind {
    type Error = WriteKindError;

    fn try_from(key: String) -> Result<WriteKind, WriteKindError> {
        WriteKind::from_key(&key).ok_or(WriteKindError::NotAKind(key))
    }
}

/// A queued write's value, typed: the stars the file is to hold (0 clears its rating), or the quarter turns still to
/// write. The queue keeps it as its `(kind, value)` columns ([`Write::kind`], [`Write::stored`]), and a row is read
/// back through [`Write::from_stored`], so a value out of its kind's range is refused where it is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Write {
    Rating(Rating),
    Rotation(Turns),
}

impl Write {
    pub fn kind(self) -> WriteKind {
        match self {
            Write::Rating(_) => WriteKind::Rating,
            Write::Rotation(_) => WriteKind::Rotation,
        }
    }

    /// The `value` column: the stars 0 to 5, or the turns 0 to 3.
    pub fn stored(self) -> i64 {
        match self {
            Write::Rating(rating) => i64::from(rating.get()),
            Write::Rotation(turns) => i64::from(turns.get()),
        }
    }

    /// The write a row's `kind` and `value` hold; `None` for a value out of the kind's range (the schema's `CHECK`
    /// refuses one).
    pub fn from_stored(kind: WriteKind, value: i64) -> Option<Write> {
        let small = u8::try_from(value).ok()?;
        match kind {
            WriteKind::Rating => Rating::new(small).map(Write::Rating),
            WriteKind::Rotation if small <= 3 => Some(Write::Rotation(Turns::new(small))),
            WriteKind::Rotation => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_reads_back_from_its_columns_and_a_value_out_of_range_does_not() {
        for write in [
            Write::Rating(Rating::new(0).unwrap()),
            Write::Rating(Rating::new(5).unwrap()),
            Write::Rotation(Turns::new(3)),
        ] {
            assert_eq!(Write::from_stored(write.kind(), write.stored()), Some(write));
        }
        assert_eq!(Write::from_stored(WriteKind::Rating, 6), None);
        assert_eq!(Write::from_stored(WriteKind::Rotation, 4), None);
        assert_eq!(Write::from_stored(WriteKind::Rotation, -1), None);
    }

    #[test]
    fn the_rotation_kind_is_still_stored_as_rotation() {
        assert_eq!(WriteKind::Rotation.key(), "rotation");
        assert_eq!(serde_json::to_string(&WriteKind::Rotation).unwrap(), "\"rotation\"");
    }

    #[test]
    fn a_kind_reads_back_from_its_key_and_its_json() {
        for kind in [WriteKind::Rating, WriteKind::Rotation] {
            assert_eq!(WriteKind::from_key(kind.key()), Some(kind));
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.key()));
            assert_eq!(serde_json::from_str::<WriteKind>(&json).unwrap(), kind);
        }
    }
}
