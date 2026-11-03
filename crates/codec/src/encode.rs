//! Re-encoding: a crop of a format the lossless rewriters don't cut (TIFF and BMP), decoded, cropped and encoded
//! again in the same format.

use anyhow::{Context, Result, bail};
use image::ImageFormat;
use pg_model::format::Format;
use std::io::Cursor;

/// `bytes` (a BMP or a TIFF) decoded, cropped to the `w` × `h` rectangle at (`x`, `y`) and encoded again in the same
/// format. The metadata is not carried over.
pub fn cropped(bytes: &[u8], format: Format, x: u32, y: u32, w: u32, h: u32) -> Result<Vec<u8>> {
    let format = match format {
        Format::Bmp => ImageFormat::Bmp,
        Format::Tiff => ImageFormat::Tiff,
        other => bail!("{other:?} isn't re-encoded: its crop is lossless"),
    };
    let img = image::load_from_memory_with_format(bytes, format).context("decoding")?;
    let mut out = Cursor::new(Vec::new());
    img.crop_imm(x, y, w, h).write_to(&mut out, format).context("encoding")?;
    Ok(out.into_inner())
}
