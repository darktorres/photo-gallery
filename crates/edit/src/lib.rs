//! The lossless edits of a file, in its own bytes: a rotation (the EXIF orientation tag, or the lossless turn of the
//! pixels where the format allows it) and a crop (on the JPEG's MCU grid, or the stored grid). A file is re-encoded only
//! where its format has no lossless way (a crop of TIFF or BMP, through `pg_codec::encode`), and then the metadata is
//! not carried over.

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod crop;
mod rewrite;
pub mod rotate;

/// Test helpers: a file decoded as the viewer shows it (`pg_render::display`), so the edits are checked against the real
/// display decode; and as it is stored, which is what a lossless crop cuts (the codec's own decode).
#[cfg(test)]
pub(crate) mod testing {
    use pg_codec::decode::{Decoder, MinSize};
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
            .decode_stored_into(bytes, format, exif, MinSize::Full, Turns::NONE, &mut image)
            .unwrap();
        (image.width, image.height, image.pixels)
    }
}
