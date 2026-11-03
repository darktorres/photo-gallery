//! The formats the app reads: what a file's bytes say it is ([`Format::sniff`]) and its stable name in the
//! catalog and the query. Model-shaped: no decoding here, so the codec, the metadata and the catalog can all use it.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
        [Format::Jpeg, Format::Png, Format::Gif, Format::Webp, Format::Bmp, Format::Tiff]
            .into_iter()
            .find(|f| f.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Jpeg => "jpeg",
            Format::Png => "png",
            Format::Gif => "gif",
            Format::Webp => "webp",
            Format::Bmp => "bmp",
            Format::Tiff => "tiff",
        }
    }
}
