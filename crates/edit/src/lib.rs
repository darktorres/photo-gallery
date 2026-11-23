//! The lossless edits of a file, in its own bytes: a rotation (the EXIF orientation tag, or the lossless turn of the
//! pixels where the format allows it) and a crop (on the JPEG's MCU grid, or the stored grid). A file is re-encoded only
//! where its format has no lossless way (a crop of TIFF or BMP, through `pg_codec::encode`), and then the metadata is
//! not carried over. The pixel rewrites themselves are the codec's (`pg_codec::rewrite`).

#![forbid(unsafe_code)]

pub mod crop;
pub mod rotate;

/// What an edit couldn't do to a file's bytes. `rotate` takes [`pg_codec::Error::Imperfect`] and
/// [`pg_codec::Error::NothingLeft`] as "tag it" and "the catalog turns it", not as failures; every other one is a failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The edit is not possible on the file as it is, with the words that say why (a crop that leaves nothing, a format
    /// with no lossless way to cut it, a damaged file).
    #[error("{0}")]
    Invalid(String),
    /// The codec refused the bytes or failed the rewrite (see `pg_codec::Error`).
    #[error(transparent)]
    Codec(#[from] pg_codec::Error),
}

/// The result of an edit.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// An edit that is not possible on the file, as `words`.
    pub(crate) fn invalid(words: impl Into<String>) -> Error {
        Error::Invalid(words.into())
    }
}

/// Test helpers: a file decoded as the viewer shows it (`pg_render::display`), so the edits are checked against the real
/// display decode; and as it is stored, which is what a lossless crop cuts (the codec's own decode).
#[cfg(test)]
pub(crate) mod testing {
    use pg_codec::decode::{Decoder, MinSize, Request};
    use pg_codec::pixels::Rgb;
    use pg_model::format::Format;
    use pg_model::turns::Turns;

    /// `bytes` decoded upright at full size: (width, height, pixels).
    pub(crate) fn shown(bytes: &[u8]) -> (usize, usize, Vec<u8>) {
        let image =
            pg_render::display::decode(&mut Decoder::new().unwrap(), bytes, MinSize::Full, Turns::NONE).unwrap().image;
        (image.width, image.height, image.pixels)
    }

    /// `bytes` decoded as stored (not oriented) at full size: (width, height, pixels).
    pub(crate) fn stored(bytes: &[u8]) -> (usize, usize, Vec<u8>) {
        let format = Format::sniff(bytes).unwrap();
        let mut image = Rgb::default();
        let exif = pg_metadata::orientation(bytes, format);
        Decoder::new()
            .unwrap()
            .decode_stored_into(bytes, Request { format, exif, min: MinSize::Full, turns: Turns::NONE }, &mut image)
            .unwrap();
        (image.width, image.height, image.pixels)
    }
}
