//! The cap on a decoded image's size, and its reservation the way `ImageReader::decode` makes it.

use image::{ColorType, ImageResult, Limits};

/// The most bytes one decoded image may take: `ImageReader`'s default `max_alloc` (512 MiB). The one cap for every
/// decoded buffer, the decoders' here and the lossless rewrites' (`pg_edit`), so a damaged header that declares a
/// huge image is an error before anything that size is allocated, never an allocation failure (which aborts).
pub fn max_decode_bytes() -> u64 {
    Limits::default().max_alloc.unwrap_or(u64::MAX)
}

/// `ImageReader::decode`'s reservation of the decoded image's size (`total_bytes`) from the default limits: an
/// image over 512 MiB fails here. The limits left are what the decoder is given.
pub(super) fn reserve(color: ColorType, w: u32, h: u32) -> ImageResult<Limits> {
    let mut limits = Limits::default();
    let total = (u64::from(w) * u64::from(h)).saturating_mul(u64::from(color.bytes_per_pixel()));
    limits.reserve(total)?;
    Ok(limits)
}
