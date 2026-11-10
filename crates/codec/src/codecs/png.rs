//! PNG (and APNG's first frame), through the png crate as `image`'s `PngDecoder` drives it.

use std::io::Cursor;

use image::error::{
    DecodingError, LimitError, LimitErrorKind, ParameterError, ParameterErrorKind, UnsupportedError,
    UnsupportedErrorKind,
};
use image::{ColorType, ExtendedColorType, ImageError, ImageFormat, ImageResult};

use super::convert::{finish, target};
use super::limits::{max_decode_bytes, reserve};
use crate::pixels::Rgb;

/// `image`'s `PngDecoder::with_limits` and `read_image`, in the buffers of the PNG before (vendored png's
/// `Scratch`), which go back to `scratch` whatever happens. Samples that aren't 8-bit RGB or RGBA are read into
/// `raw`.
pub(super) fn decode(scratch: &mut png::Scratch, raw: &mut Vec<u8>, bytes: &[u8], out: &mut Rgb) -> ImageResult<()> {
    // The PNG decoder is given `ImageReader`'s limits as they are before the image's size is reserved.
    let max_bytes = usize::try_from(max_decode_bytes()).unwrap_or(usize::MAX);
    let limits = png::Limits { bytes: max_bytes };
    let mut decoder = png::Decoder::new_with_scratch(Cursor::new(bytes), limits, std::mem::take(scratch));
    // Text and ICC profiles are skipped, where `image` reads them for its metadata getters, which a decode
    // doesn't call: the metadata reader finds the XMP itself, and colours aren't managed. The png crate
    // ignores what is wrong in them, so skipping them changes no pixel and no error, except for a file whose
    // text alone exceeds the 512 MiB limit; it spares a copy of each.
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    if let Err(err) = decoder.read_header_info() {
        *scratch = decoder.into_scratch();
        return Err(png_error(err));
    }
    // 16-bit samples kept, samples of fewer bits expanded to 8, palettes to RGB(A), tRNS to alpha.
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = match decoder.read_info_or_scratch() {
        Ok(reader) => reader,
        Err((err, kept)) => {
            *scratch = kept;
            return Err(png_error(err));
        }
    };
    let result = read(&mut reader, raw, out);
    *scratch = reader.into_scratch();
    result
}

/// [`decode`]'s image, once the headers are read.
fn read(reader: &mut png::Reader<Cursor<&[u8]>>, raw: &mut Vec<u8>, out: &mut Rgb) -> ImageResult<()> {
    let color = png_color(reader.output_color_type())?;
    let (w, h) = reader.info().size();
    reserve(color, w, h)?;
    let buf = target(color, w, h, out, raw);
    reader.next_frame(buf).map_err(png_error)?;
    // PNG's samples are big-endian; `read_image` gives them in native order.
    if color.bytes_per_pixel() / color.channel_count() == 2 {
        for c in buf.as_chunks_mut::<2>().0 {
            *c = u16::from_be_bytes(*c).to_ne_bytes();
        }
    }
    finish(color, w, h, out, raw)
}

/// `image`'s colour type for a PNG's output (`PngDecoder::with_limits`).
fn png_color((color, bits): (png::ColorType, png::BitDepth)) -> ImageResult<ColorType> {
    use png::BitDepth as B;
    use png::ColorType as C;
    let unsupported = match (color, bits) {
        (C::Grayscale, B::Eight) => return Ok(ColorType::L8),
        (C::Grayscale, B::Sixteen) => return Ok(ColorType::L16),
        (C::GrayscaleAlpha, B::Eight) => return Ok(ColorType::La8),
        (C::GrayscaleAlpha, B::Sixteen) => return Ok(ColorType::La16),
        (C::Rgb, B::Eight) => return Ok(ColorType::Rgb8),
        (C::Rgb, B::Sixteen) => return Ok(ColorType::Rgb16),
        (C::Rgba, B::Eight) => return Ok(ColorType::Rgba8),
        (C::Rgba, B::Sixteen) => return Ok(ColorType::Rgba16),
        (C::Grayscale, B::One) => ExtendedColorType::L1,
        (C::GrayscaleAlpha, B::One) => ExtendedColorType::La1,
        (C::Rgb, B::One) => ExtendedColorType::Rgb1,
        (C::Rgba, B::One) => ExtendedColorType::Rgba1,
        (C::Grayscale, B::Two) => ExtendedColorType::L2,
        (C::GrayscaleAlpha, B::Two) => ExtendedColorType::La2,
        (C::Rgb, B::Two) => ExtendedColorType::Rgb2,
        (C::Rgba, B::Two) => ExtendedColorType::Rgba2,
        (C::Grayscale, B::Four) => ExtendedColorType::L4,
        (C::GrayscaleAlpha, B::Four) => ExtendedColorType::La4,
        (C::Rgb, B::Four) => ExtendedColorType::Rgb4,
        (C::Rgba, B::Four) => ExtendedColorType::Rgba4,
        (C::Indexed, bits) => ExtendedColorType::Unknown(match bits {
            B::One => 1,
            B::Two => 2,
            B::Four => 4,
            B::Eight => 8,
            B::Sixteen => 16,
        }),
    };
    Err(ImageError::Unsupported(UnsupportedError::from_format_and_kind(
        ImageFormat::Png.into(),
        UnsupportedErrorKind::Color(unsupported),
    )))
}

/// `ImageError::from_png`.
fn png_error(err: png::DecodingError) -> ImageError {
    match err {
        png::DecodingError::IoError(err) => ImageError::IoError(err),
        err @ png::DecodingError::Format(_) => ImageError::Decoding(DecodingError::new(ImageFormat::Png.into(), err)),
        err @ png::DecodingError::Parameter(_) => {
            ImageError::Parameter(ParameterError::from_kind(ParameterErrorKind::Generic(err.to_string())))
        }
        png::DecodingError::LimitsExceeded => {
            ImageError::Limits(LimitError::from_kind(LimitErrorKind::InsufficientMemory))
        }
    }
}
