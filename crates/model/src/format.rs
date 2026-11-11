//! The formats the app reads: what a file's bytes say it is ([`Format::sniff`]) and its stable name in the
//! catalog and the query. Model-shaped: no decoding here, so the codec, the metadata and the catalog can all use it.

#[derive(Clone, Copy, PartialEq, Eq, Debug, strum::EnumString, strum::IntoStaticStr, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum Format {
    Jpeg,
    Png,
    Gif,
    Webp,
    Bmp,
    Tiff,
}

impl Format {
    pub fn sniff(bytes: &[u8]) -> Option<Format> {
        Some(match bytes {
            [0xFF, 0xD8, 0xFF, ..] => Format::Jpeg,
            [0x89, b'P', b'N', b'G', ..] => Format::Png,
            [b'G', b'I', b'F', b'8', ..] => Format::Gif,
            [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Format::Webp,
            [b'B', b'M', ..] => Format::Bmp,
            [b'I', b'I', 0x2A, 0x00, ..] | [b'M', b'M', 0x00, 0x2A, ..] => Format::Tiff,
            // BigTIFF: 43, then 8 (the offset size) and 0.
            [b'I', b'I', 0x2B, 0x00, 0x08, 0x00, 0x00, 0x00, ..]
            | [b'M', b'M', 0x00, 0x2B, 0x00, 0x08, 0x00, 0x00, ..] => Format::Tiff,
            _ => return None,
        })
    }

    /// The inverse of [`Format::name`].
    pub fn from_name(name: &str) -> Option<Format> {
        name.parse().ok()
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

    pub fn name(self) -> &'static str {
        self.into()
    }
}

/// Whether the format can carry a rating (an XMP packet) that other programs read. BMP can't: its rating lives in the
/// catalog only.
pub fn can_hold_rating(format: Format) -> bool {
    !matches!(format, Format::Bmp)
}

/// Whether a file of `kind` and analyzed `format` (`None` when it isn't analyzed yet) may hold a rating of its own: an
/// image (a video keeps its rating in the catalog, as rewriting it would copy GBs), whose format can hold one
/// ([`can_hold_rating`]) or isn't known yet. The writer decides for an unknown format once it has read the file
/// (`pg_writeback`).
pub fn rating_in_file_possible(kind: crate::media::Kind, format: Option<Format>) -> bool {
    kind == crate::media::Kind::Image && format.is_none_or(can_hold_rating)
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    #[test]
    fn the_names_are_the_catalog_names() {
        assert_eq!(Format::iter().map(Format::name).collect::<Vec<_>>(), ["jpeg", "png", "gif", "webp", "bmp", "tiff"],);
        for f in Format::iter() {
            assert_eq!(Format::from_name(f.name()), Some(f));
        }
        assert_eq!(Format::from_name("jpg"), None);
    }
}
