//! Re-encoding: a crop of a format the lossless rewriters don't cut (TIFF and BMP), decoded, cropped and encoded
//! again in the same format.

use anyhow::{Context, Result};
use image::ImageFormat;
use std::io::Cursor;

/// `bytes` decoded, cropped to the `w` × `h` rectangle at (`x`, `y`) and encoded again in `format`. The metadata is
/// not carried over.
pub fn cropped(bytes: &[u8], format: ImageFormat, x: u32, y: u32, w: u32, h: u32) -> Result<Vec<u8>> {
    let img = image::load_from_memory_with_format(bytes, format).context("decoding")?;
    let mut out = Cursor::new(Vec::new());
    img.crop_imm(x, y, w, h).write_to(&mut out, format).context("encoding")?;
    Ok(out.into_inner())
}
