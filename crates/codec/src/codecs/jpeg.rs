//! The JPEGs libjpeg-turbo rejects (CMYK, some damaged files), through zune-jpeg as `image`'s `JpegDecoder` drives
//! it.

use image::error::{DecodingError, LimitError, LimitErrorKind, UnsupportedError, UnsupportedErrorKind};
use image::{ColorType, ImageError, ImageFormat, ImageResult};
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

use super::convert::{finish, target};
use super::limits::reserve;
use crate::pixels::Rgb;

/// The stored size of a JPEG libjpeg-turbo's header read rejects, from zune-jpeg's headers (what [`decode`] would then
/// decode from). No pixels: the headers only.
pub(super) fn dimensions(scratch: &mut zune_jpeg::Scratch, bytes: &[u8]) -> ImageResult<(u32, u32)> {
    let options =
        || DecoderOptions::default().set_strict_mode(false).set_max_width(usize::MAX).set_max_height(usize::MAX);
    let mut headers = JpegDecoder::new_with_scratch(ZCursor::new(bytes), options(), std::mem::take(scratch));
    headers.set_skip_metadata(true);
    let read = headers.decode_headers();
    let size = headers.dimensions();
    *scratch = headers.into_scratch();
    read.map_err(jpeg_error)?;
    let (w, h) = size.ok_or_else(|| {
        ImageError::Decoding(DecodingError::new(ImageFormat::Jpeg.into(), "the headers gave no size".to_string()))
    })?;
    Ok((
        u32::try_from(w).map_err(|_| ImageError::Limits(LimitError::from_kind(LimitErrorKind::DimensionError)))?,
        u32::try_from(h).map_err(|_| ImageError::Limits(LimitError::from_kind(LimitErrorKind::DimensionError)))?,
    ))
}

/// A side of the size the decoder reports, as the 16 bits a JPEG's side fits in: the one narrowing of a size, at the
/// decoder's boundary (a size is a `u32` everywhere else). `None` past 16 bits.
fn decoder_side(side: usize) -> Option<u16> {
    u16::try_from(side).ok()
}

/// `image`'s `JpegDecoder` (zune-jpeg), for the JPEGs libjpeg-turbo rejects. As there, one decoder reads the
/// headers and another, set up from them, decodes: the colour conversion zune-jpeg uses is chosen as it reads
/// the headers, from the output colour space it was given. Both decode in the buffers of the JPEG before
/// (vendored zune-jpeg's `Scratch`), which go back to `scratch` whatever happens.
///
/// The metadata segments (EXIF, XMP, ICC, IPTC…) are checked as `image`'s decoder checks them, with the same
/// errors, but not copied (`set_skip_metadata`): `image` keeps them for its metadata getters, which a decode
/// doesn't call.
pub(super) fn decode(
    scratch: &mut zune_jpeg::Scratch,
    raw: &mut Vec<u8>,
    bytes: &[u8],
    out: &mut Rgb,
) -> ImageResult<()> {
    let options =
        || DecoderOptions::default().set_strict_mode(false).set_max_width(usize::MAX).set_max_height(usize::MAX);
    let mut headers = JpegDecoder::new_with_scratch(ZCursor::new(bytes), options(), std::mem::take(scratch));
    headers.set_skip_metadata(true);
    let read = headers.decode_headers();
    if let Err(e) = read {
        *scratch = headers.into_scratch();
        return Err(jpeg_error(e));
    }
    // Known once the headers are read.
    let size = headers.dimensions().and_then(|(w, h)| Some((decoder_side(w)?, decoder_side(h)?)));
    let input = headers.input_colorspace();
    let kept = headers.into_scratch();
    let (Some((w, h)), Some(input)) = (size, input) else {
        *scratch = kept;
        return Err(ImageError::Decoding(DecodingError::new(
            ImageFormat::Jpeg.into(),
            "the headers gave no size or colour space".to_string(),
        )));
    };
    let (w, h) = (u32::from(w), u32::from(h));
    // Gray and RGB(A) are kept; the rest (CMYK, YCCK…) is converted to RGB by the decoder.
    let (space, color) = match input {
        ColorSpace::RGB => (ColorSpace::RGB, ColorType::Rgb8),
        ColorSpace::RGBA => (ColorSpace::RGBA, ColorType::Rgba8),
        ColorSpace::Luma => (ColorSpace::Luma, ColorType::L8),
        ColorSpace::LumaA => (ColorSpace::LumaA, ColorType::La8),
        _ => (ColorSpace::RGB, ColorType::Rgb8),
    };
    if let Err(e) = reserve(color, w, h) {
        *scratch = kept;
        return Err(e);
    }
    let mut decoder =
        JpegDecoder::new_with_scratch(ZCursor::new(bytes), options().jpeg_set_out_colorspace(space), kept);
    decoder.set_skip_metadata(true);
    let decoded = decoder.decode_into(target(color, w, h, out, raw)).map_err(jpeg_error);
    *scratch = decoder.into_scratch();
    decoded?;
    finish(color, w, h, out, raw)
}

/// `ImageError::from_jpeg` (zune-jpeg).
fn jpeg_error(err: zune_jpeg::errors::DecodeErrors) -> ImageError {
    use zune_jpeg::errors::DecodeErrors;
    match err {
        // zune-jpeg's reason has no `Display`, only `Debug`, and that form is not for a user: the text names the cause.
        DecodeErrors::Unsupported(_) => ImageError::Unsupported(UnsupportedError::from_format_and_kind(
            ImageFormat::Jpeg.into(),
            UnsupportedErrorKind::GenericFeature("a JPEG coding this decoder does not read".to_string()),
        )),
        DecodeErrors::LargeDimensions(_) => ImageError::Limits(LimitError::from_kind(LimitErrorKind::DimensionError)),
        err => ImageError::Decoding(DecodingError::new(ImageFormat::Jpeg.into(), err)),
    }
}
