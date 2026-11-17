//! What the import computes from one file, as data: the record the catalog stores and the grid reads. Producing it
//! is the job of pg-analysis's `analyze` module. This module is the shape of the result, with no decoding and no I/O.

use std::io::{self, Read};

use crate::format::Format;
use crate::orientation::Orientation;
use crate::pdq;

/// A file's content hash: the BLAKE3 of its bytes (tier 0a). The import stores it; the write-back makes it again for a
/// file whose metadata it rewrote, so the analysis made from the same pixels stays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FileHash(pub [u8; 32]);

impl FileHash {
    /// The hash of `bytes`, loaded.
    pub fn of(bytes: &[u8]) -> FileHash {
        FileHash(*blake3::hash(bytes).as_bytes())
    }

    /// The hash of what `reader` gives, streamed (a video, not read whole): the same as [`FileHash::of`] its bytes.
    pub fn of_reader(reader: impl Read) -> io::Result<FileHash> {
        let mut hasher = blake3::Hasher::new();
        hasher.update_reader(reader)?;
        Ok(FileHash(*hasher.finalize().as_bytes()))
    }
}

/// One file's analysis: what every file has, and the facts of its kind (an image's or a video's).
#[derive(Clone)]
pub struct Analysis {
    /// The version of the producer that made it (pg-analysis's `analyze::IMPORT_VERSION`); an older one's are redone.
    pub version: i64,
    /// EXIF orientation (1–8) the file asks for (a video's display matrix).
    pub orientation: Orientation,
    /// Upright full-size dimensions.
    pub width: u32,
    pub height: u32,
    /// Frames in the file (more than one: animated); an image's analysis is of the first.
    pub frames: u32,
    /// When the file says it was taken, if it does.
    pub taken: Option<Taken>,
    pub media: Media,
}

/// When a file was taken: a wall-clock time (no time zone: what the camera's clock showed), or a UTC instant (a
/// video's creation time). The catalog filters and sorts by the wall clock: a UTC instant is shown as the wall clock
/// of this computer's zone, and kept as the instant too, so the wall clock can be computed again for another zone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Taken {
    Wall(i64),
    Utc(i64),
}

impl Taken {
    /// The seconds the catalog stores, and whether they are a UTC instant (the catalog converts that to this
    /// computer's wall clock, and keeps the instant as `analysis.taken_utc`).
    pub fn parts(self) -> (i64, bool) {
        match self {
            Taken::Wall(t) => (t, false),
            Taken::Utc(t) => (t, true),
        }
    }
}

/// The facts of a file's kind.
#[derive(Clone)]
pub enum Media {
    Image(ImageFacts),
    Video(VideoFacts),
}

/// A still image's facts: its format and the rating the file itself holds. Its duplicate hashes are the Hashes pass's
/// ([`ImageHashes`]), not the import's.
#[derive(Clone)]
pub struct ImageFacts {
    pub format: Format,
    /// The rating the file itself holds (`Meta::rating`), for the catalog's three-way sync.
    pub file_rating: Option<crate::rating::Rating>,
}

/// A still image's duplicate hashes: the file's (tier 0a) and the pixels' (tiers 0b and 1). The Hashes pass makes them
/// (`pg_import::hashes`); the import does not.
#[derive(Clone)]
pub struct ImageHashes {
    /// BLAKE3 of the file bytes (tier 0a).
    pub file: FileHash,
    pub pixels: PixelHashes,
}

/// A still image's pixel hashes.
#[derive(Clone)]
pub struct PixelHashes {
    /// XXH3-128 of the upright decoded pixels (tier 0b), see `pg_analysis::analyze::pixel_hash`.
    pub pixel_hash: [u8; 16],
    pub pdq: pdq::Pdq,
}

/// A video's facts: its codec, its length, and where the frame its thumbnails and placeholder come from is.
#[derive(Clone)]
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

/// The version of a placeholder's definition (`pg_render::thumbhash_of`): a placeholder made at another version is owed
/// again. Version 1 is the first definition: the ThumbHashes the import made, moved to `placeholders` by the migration
/// that makes that table (`pg_catalog`'s `placeholders_from_thumbhashes`), and the Placeholders pass's own.
pub const PLACEHOLDER_VERSION: i64 = 1;

/// The longest ThumbHash: 25 bytes with alpha, 24 opaque. It is the vendored encoder's `MAX_LEN` as a literal (the
/// model has no dependencies); the `analyze` tests in pg-analysis check the two are equal.
pub const THUMBHASH_MAX: usize = 25;

/// A ThumbHash (a placeholder's `hash`, `placeholders`), held inline: the grid's placeholder.
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
        Some(ThumbHash { bytes, len: u8::try_from(hash.len()).ok()? })
    }

    /// A hash the encoder wrote into `bytes`, as its first `len` bytes; all of them for a `len` past the buffer.
    pub fn encoded(bytes: [u8; THUMBHASH_MAX], len: usize) -> ThumbHash {
        ThumbHash { bytes, len: u8::try_from(len.min(THUMBHASH_MAX)).unwrap_or(u8::MAX) }
    }

    /// The hash's bytes; all of the buffer for a length past it, which no encoder writes.
    pub fn as_slice(&self) -> &[u8] {
        self.bytes.get(..usize::from(self.len)).unwrap_or(&self.bytes)
    }
}
