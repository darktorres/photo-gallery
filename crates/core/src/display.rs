//! What a file shows as: its pixels turned upright (the EXIF orientation, then the catalog's quarter turns), decoded
//! at the smallest size a caller needs. The one place that reads a file's EXIF orientation for decoding, so the codec
//! ([`crate::decode`]) takes the orientation as a value and reads no metadata itself.

use anyhow::{Context, Result};

use crate::decode::{Decoded, Decoder, MinSize, Stored};
use crate::format::Format;
use crate::metadata;

/// Decode `bytes` upright at the smallest DCT scale that keeps `min` (full size if the image is smaller), into new
/// buffers: a one-off decode (the viewer). `turns` is the catalog rotation (`files.rotation`, quarter turns clockwise),
/// applied after the EXIF orientation.
pub fn decode(decoder: &mut Decoder, bytes: &[u8], min: MinSize, turns: u8) -> Result<Decoded> {
    let format = Format::sniff(bytes).context("unknown image format")?;
    let exif = metadata::orientation(bytes, format);
    let mut image = crate::decode::Rgb::default();
    let info = decoder.decode_into(bytes, format, exif, min, turns, &mut image, &mut Vec::new())?;
    Ok(Decoded { image, info })
}

/// [`decode`] without the orientation, into new buffers: the pixels as stored, which is what a lossless crop cuts.
pub fn decode_stored(decoder: &mut Decoder, bytes: &[u8], min: MinSize, turns: u8) -> Result<Stored> {
    let format = Format::sniff(bytes).context("unknown image format")?;
    let exif = metadata::orientation(bytes, format);
    let mut image = crate::decode::Rgb::default();
    let info = decoder.decode_stored_into(bytes, format, exif, min, turns, &mut image)?;
    Ok(Stored { image, info })
}
