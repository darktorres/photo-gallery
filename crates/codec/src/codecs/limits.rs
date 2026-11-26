//! The cap on a decoded image's size, and its reservation the way `ImageReader::decode` makes it.

use image::{ColorType, ImageResult, Limits};

use crate::{Error, Result};

/// The most bytes one decoded image may take: `ImageReader`'s default `max_alloc` (512 MiB). The one cap for every
/// decoded buffer, the decoders' here and the lossless rewrites' (`rewrite`), so a damaged header that declares a
/// huge image is an error before anything that size is allocated, never an allocation failure (which aborts).
pub fn max_decode_bytes() -> u64 {
    Limits::default().max_alloc.unwrap_or(u64::MAX)
}

/// The size of a decoded buffer, the product of `factors` (its width, height and bytes per pixel), held to
/// [`max_decode_bytes`]. A decoder that sizes its own buffer checks it here first (`what` names the image in the error).
pub(crate) fn decoded_size(what: &str, factors: &[usize]) -> Result<usize> {
    let len = factors.iter().try_fold(1usize, |n, &f| n.checked_mul(f));
    match len {
        Some(len) if u64::try_from(len).is_ok_and(|len| len <= max_decode_bytes()) => Ok(len),
        _ => Err(Error::invalid(format!("{what} is too large to decode"))),
    }
}

/// `ImageReader::decode`'s reservation of the decoded image's size (`total_bytes`) from the default limits: an
/// image over 512 MiB fails here. The limits left are what the decoder is given.
pub(super) fn reserve(color: ColorType, w: u32, h: u32) -> ImageResult<Limits> {
    let mut limits = Limits::default();
    let total = (u64::from(w) * u64::from(h)).saturating_mul(u64::from(color.bytes_per_pixel()));
    limits.reserve(total)?;
    Ok(limits)
}
