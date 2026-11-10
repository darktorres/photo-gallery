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
    // Known once the headers are read; a JPEG's sides fit 16 bits.
    let size = headers.dimensions().and_then(|(w, h)| Some((u16::try_from(w).ok()?, u16::try_from(h).ok()?)));
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
        DecodeErrors::Unsupported(desc) => ImageError::Unsupported(UnsupportedError::from_format_and_kind(
            ImageFormat::Jpeg.into(),
            UnsupportedErrorKind::GenericFeature(format!("{desc:?}")),
        )),
        DecodeErrors::LargeDimensions(_) => ImageError::Limits(LimitError::from_kind(LimitErrorKind::DimensionError)),
        err => ImageError::Decoding(DecodingError::new(ImageFormat::Jpeg.into(), err)),
    }
}
