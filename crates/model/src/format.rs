//! The formats the app reads: what a file's bytes say it is ([`Format::sniff`]) and its stable name in the
//! catalog and the query. Model-shaped: no decoding here, so the codec, the metadata and the catalog can all use it.

#[derive(Clone, Copy, PartialEq, Eq, Debug, strum::EnumString, strum::IntoStaticStr, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
/// An image's format, as the sniffer and the catalog name it.
pub enum Format {
    /// JPEG.
    Jpeg,
    /// PNG.
    Png,
    /// GIF.
    Gif,
    /// WebP.
    Webp,
    /// BMP.
    Bmp,
    /// TIFF.
    Tiff,
}

impl Format {
    /// The format of the bytes, from their first bytes, if it is one this build knows.
    pub fn sniff(bytes: &[u8]) -> Option<Format> {
        Some(match bytes {
            [0xFF, 0xD8, 0xFF, ..] => Format::Jpeg,
            [0x89, b'P', b'N', b'G', ..] => Format::Png,
            [b'G', b'I', b'F', b'8', ..] => Format::Gif,
            [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Format::Webp,
            [b'I', b'I', 0x2A, 0x00, ..] | [b'M', b'M', 0x00, 0x2A, ..] => Format::Tiff,
            _ if is_bmp(bytes) => Format::Bmp,
            _ if is_bigtiff(bytes) => Format::Tiff,
            _ => return None,
        })
    }

    /// The inverse of [`Format::key`].
    pub fn from_key(key: &str) -> Option<Format> {
        key.parse().ok()
    }

    /// The extension a file of the format is saved under, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::Png => "png",
            Format::Gif => "gif",
            Format::Webp => "webp",
            Format::Bmp => "bmp",
            Format::Tiff => "tif",
        }
    }

    /// The format's name (`jpeg`, `png`, …).
    pub fn key(self) -> &'static str {
        self.into()
    }

    /// The code the catalog stores the format under (`analysis.format`). Fixed per format, never reused: a new format
    /// takes the next code.
    pub const fn code(self) -> i64 {
        match self {
            Format::Jpeg => 0,
            Format::Png => 1,
            Format::Gif => 2,
            Format::Webp => 3,
            Format::Bmp => 4,
            Format::Tiff => 5,
        }
    }

    /// The format a stored code stands for, if it is one this build knows.
    pub fn from_code(code: i64) -> Option<Format> {
        use strum::IntoEnumIterator;
        Format::iter().find(|f| f.code() == code)
    }
}

/// Whether a BMP's header is one: `BM`, the file size, two reserved words that are zero, the pixel offset, then the size
/// of the header after it, which names the variant (OS/2's 12, or Windows' 40 and the later ones). Two letters alone open
/// many other files, so they are not enough.
fn is_bmp(bytes: &[u8]) -> bool {
    match bytes {
        [b'B', b'M', _, _, _, _, 0, 0, 0, 0, _, _, _, _, a, b, c, d, ..] => {
            matches!(u32::from_le_bytes([*a, *b, *c, *d]), 12 | 16 | 40 | 52 | 56 | 64 | 108 | 124)
        }
        _ => false,
    }
}

/// Whether a TIFF's header is a BigTIFF's: 43 after the byte order, where a classic TIFF has 42, then 8 (the offset
/// size) and 0.
pub fn is_bigtiff(bytes: &[u8]) -> bool {
    matches!(
        bytes,
        [b'I', b'I', 0x2B, 0x00, 0x08, 0x00, 0x00, 0x00, ..] | [b'M', b'M', 0x00, 0x2B, 0x00, 0x08, 0x00, 0x00, ..]
    )
}

/// Whether the format can carry a rating (an XMP packet) that other programs read. BMP can't: its rating lives in the
/// catalog only.
pub fn can_hold_rating(format: Format) -> bool {
    !matches!(format, Format::Bmp)
}

/// Whether a file of `kind` and analyzed `format` (`None` when it isn't analyzed yet) may hold a rating of its own: an
/// image (a video keeps its rating in the catalog, as rewriting it would copy GBs), whose format can hold one
/// ([`can_hold_rating`]) or isn't known yet. The writer decides for an unknown format once it has read the file
/// (`pg-writeback`).
pub fn rating_in_file_possible(kind: crate::media::Kind, format: Option<Format>) -> bool {
    kind == crate::media::Kind::Image && format.is_none_or(can_hold_rating)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use strum::IntoEnumIterator;

    use super::*;

    #[test]
    fn sniffs_bmp_by_its_header_not_its_first_two_bytes() {
        // "BM", the file size, zero reserved words, the pixel offset (54), then a 40-byte header.
        let header = b"BM\x36\x00\x00\x00\x00\x00\x00\x00\x36\x00\x00\x00\x28\x00\x00\x00";
        assert_eq!(Format::sniff(header), Some(Format::Bmp));
        assert_eq!(Format::sniff(b"BMhello, this is a text file"), None);
        assert_eq!(Format::sniff(b"BM\x36\x00\x00\x00\x00\x00\x00\x00\x36\x00\x00\x00\x29\x00\x00\x00"), None);
    }

    #[test]
    fn sniffs_classic_tiff_and_bigtiff() {
        assert_eq!(Format::sniff(b"II*\0\x08\0\0\0"), Some(Format::Tiff));
        assert_eq!(Format::sniff(b"MM\0*\0\0\0\x08"), Some(Format::Tiff));
        assert_eq!(Format::sniff(b"II+\0\x08\0\0\0\x10\0\0\0\0\0\0\0"), Some(Format::Tiff));
        assert_eq!(Format::sniff(b"MM\0+\0\x08\0\0\0\0\0\0\0\x10"), Some(Format::Tiff));
        // A 43 whose offset size isn't 8, or a 43 cut short, isn't a BigTIFF header.
        assert_eq!(Format::sniff(b"II+\0\x04\0\0\0"), None);
        assert_eq!(Format::sniff(b"MM\0+\0\x08\0\x01"), None);
        assert_eq!(Format::sniff(b"II+\0\x08"), None);
    }

    #[test]
    fn the_names_are_the_catalog_names() {
        assert_eq!(Format::iter().map(Format::key).collect::<Vec<_>>(), ["jpeg", "png", "gif", "webp", "bmp", "tiff"],);
        for f in Format::iter() {
            assert_eq!(Format::from_key(f.key()), Some(f));
        }
        assert_eq!(Format::from_key("jpg"), None);
    }

    /// The stored codes are pinned: `analysis.format` holds them in every catalog.
    #[test]
    fn the_stored_codes_are_pinned() {
        assert_eq!(Format::iter().map(Format::code).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5]);
        for f in Format::iter() {
            assert_eq!(Format::from_code(f.code()), Some(f));
        }
        assert_eq!(Format::from_code(6), None);
    }
}
