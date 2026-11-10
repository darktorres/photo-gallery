//! WebP, through image-webp as `image`'s `WebPDecoder` drives it.

use std::io::Cursor;

use image::error::DecodingError;
use image::{ColorType, ImageError, ImageFormat, ImageResult};

use super::convert::{finish, target};
use super::limits::reserve;
use crate::pixels::Rgb;

/// `image`'s `WebPDecoder`, in the buffers of the WebP before (vendored image-webp's `Scratch`), which go back to
/// `scratch` whatever happens.
pub(super) fn decode(
    scratch: &mut image_webp::Scratch,
    raw: &mut Vec<u8>,
    bytes: &[u8],
    out: &mut Rgb,
) -> ImageResult<()> {
    let mut decoder = match image_webp::WebPDecoder::new_with_scratch(Cursor::new(bytes), std::mem::take(scratch)) {
        Ok(decoder) => decoder,
        Err((err, kept)) => {
            *scratch = kept;
            return Err(webp_error(err));
        }
    };
    let result = read(&mut decoder, raw, out);
    *scratch = decoder.into_scratch();
    result
}

/// [`decode`]'s image, once the headers are read: RGBA if it has alpha, RGB otherwise.
fn read(decoder: &mut image_webp::WebPDecoder<Cursor<&[u8]>>, raw: &mut Vec<u8>, out: &mut Rgb) -> ImageResult<()> {
    let (w, h) = decoder.dimensions();
    let color = if decoder.has_alpha() { ColorType::Rgba8 } else { ColorType::Rgb8 };
    reserve(color, w, h)?;
    decoder.read_image(target(color, w, h, out, raw)).map_err(webp_error)?;
    finish(color, w, h, out, raw)
}

/// `ImageError::from_webp_decode`.
fn webp_error(err: image_webp::DecodingError) -> ImageError {
    match err {
        image_webp::DecodingError::IoError(err) => ImageError::IoError(err),
        _ => ImageError::Decoding(DecodingError::new(ImageFormat::WebP.into(), err)),
    }
}
