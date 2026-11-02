//! What the import computes from one file, as data: the record the catalog stores and the grid reads. Producing it
//! is [`crate::analyze`]'s job. This module is the shape of the result, with no decoding and no I/O.

use crate::format::Format;
use crate::pdq;

/// One file's analysis: what every file has, and the facts of its kind (an image's or a video's).
pub struct Analysis {
    /// The version of the producer that made it ([`crate::analyze::IMPORT_VERSION`]); an older one's are redone.
    pub version: i64,
    /// BLAKE3 of the file bytes (tier 0a).
    pub file_hash: [u8; 32],
    /// EXIF orientation (1–8) the file asks for (a video's display matrix).
    pub orientation: u8,
    /// Upright full-size dimensions.
    pub width: u32,
    pub height: u32,
    pub thumbhash: ThumbHash,
    /// Frames in the file (more than one: animated); an image's analysis is of the first.
    pub frames: u32,
    /// When the file says it was taken, if it does.
    pub taken: Option<Taken>,
    pub media: Media,
}

/// When a file was taken: a wall-clock time (no time zone: what the camera's clock showed), or a UTC instant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Taken {
    Wall(i64),
    Utc(i64),
}

impl Taken {
    /// The seconds the catalog stores, and whether they are a UTC instant (the catalog stores that as this
    /// computer's wall-clock time).
    pub fn parts(self) -> (i64, bool) {
        match self {
            Taken::Wall(t) => (t, false),
            Taken::Utc(t) => (t, true),
        }
    }
}

/// The facts of a file's kind.
// The image's hashes are held inline on purpose: an import allocates nothing per file, and a video's facts are the
// small case.
#[allow(clippy::large_enum_variant)]
pub enum Media {
    Image(ImageFacts),
    Video(VideoFacts),
}

/// A still image's facts: its format, the rating the file itself holds, and its hashes (tiers 0b and 1).
pub struct ImageFacts {
    pub format: Format,
    /// The rating the file itself holds (`Meta::rating`), for the catalog's three-way sync.
    pub file_rating: Option<u8>,
    pub pixels: PixelHashes,
}

/// A still image's pixel hashes.
pub struct PixelHashes {
    /// XXH3-128 of the upright decoded pixels (tier 0b), see [`crate::analyze::pixel_hash`].
    pub pixel_hash: [u8; 16],
    pub pdq: pdq::Pdq,
    /// The 8 dihedral PDQ hashes, for the query side of tier 1. The catalog stores them with the analysis.
    pub pdq_dihedral: [pdq::Hash; 8],
}

/// A video's facts: its codec, its length, and where the frame its ThumbHash and thumbnails come from is.
pub struct VideoFacts {
    pub codec: &'static str,
    pub duration_ms: i64,
    pub poster_ms: i64,
}

impl Analysis {
    /// The format's name for an image, or the codec for a video (the catalog's `format` column).
    pub fn format(&self) -> &'static str {
        match &self.media {
            Media::Image(image) => image.format.name(),
            Media::Video(video) => video.codec,
        }
    }

    pub fn image(&self) -> Option<&ImageFacts> {
        match &self.media {
            Media::Image(image) => Some(image),
            Media::Video(_) => None,
        }
    }

    pub fn video(&self) -> Option<&VideoFacts> {
        match &self.media {
            Media::Video(video) => Some(video),
            Media::Image(_) => None,
        }
    }
}

/// The longest ThumbHash (vendor/thumbhash `MAX_LEN`: 25 bytes with alpha, 24 opaque).
pub const THUMBHASH_MAX: usize = thumbhash::MAX_LEN;

/// A ThumbHash (`analysis.thumbhash`), held inline: the import's result, and the grid's placeholder.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ThumbHash {
    bytes: [u8; THUMBHASH_MAX],
    len: u8,
}

impl ThumbHash {
    /// `None` for one longer than any ThumbHash, which only a damaged catalog holds.
    pub fn new(hash: &[u8]) -> Option<ThumbHash> {
        let mut bytes = [0; THUMBHASH_MAX];
        bytes.get_mut(..hash.len())?.copy_from_slice(hash);
        Some(ThumbHash { bytes, len: hash.len() as u8 })
    }

    /// A hash the encoder wrote into `bytes`, as its first `len` bytes.
    pub(crate) fn encoded(bytes: [u8; THUMBHASH_MAX], len: usize) -> ThumbHash {
        ThumbHash { bytes, len: len as u8 }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}
