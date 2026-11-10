//! TIFF, through the tiff crate as `image`'s `TiffDecoder` drives it, and the colour types and limits the TIFF crop
//! (`encode`) shares with it.

use std::io::Cursor;

use image::error::{DecodingError, LimitError, LimitErrorKind, UnsupportedError, UnsupportedErrorKind};
use image::{ColorType, ExtendedColorType, ImageError, ImageFormat, ImageResult};

use super::convert::{check_len, cmyk_to_rgb, cmyk_to_rgb16, convert, finish, luma8, rgb16, zeroed};
use super::limits::reserve;
use crate::pixels::Rgb;
use pg_model::num;

/// How a TIFF's samples are stored, against the colour type `image` gives the image.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stored {
    /// As the colour type.
    Same,
    /// 1-bit gray, given as 8-bit (`L8`).
    Gray1,
    /// 8-bit CMYK, given as `Rgb8`.
    Cmyk8,
    /// 16-bit CMYK, given as `Rgb16`.
    Cmyk16,
}

/// What [`plan`] reads from a TIFF's tags, for [`read`].
struct TiffPlan {
    w: u32,
    h: u32,
    /// The colour type `image` gives the image.
    color: ColorType,
    stored: Stored,
    /// The decoder's limit for its decoding buffer.
    decoding_size: usize,
}

/// The TIFF decoder over a file in memory.
type TiffDecoder<'a> = tiff::decoder::Decoder<Cursor<&'a [u8]>>;

/// `image`'s `TiffDecoder`: its colour types, its split of the limits between the decoded image and the
/// decoder's own buffers, and its conversions (CMYK to RGB, 1-bit gray to 8, planes interleaved), here done as
/// the samples are converted into `out` rather than into a second full-size buffer first. In the decompressors and
/// buffers of the TIFF before (vendored tiff's `Scratch`), which go back to `scratch` whatever happens; the sample
/// formats are read into `formats`, and samples that can't go straight into `out` into `raw`.
pub(super) fn decode(
    scratch: &mut tiff::decoder::Scratch,
    formats: &mut Vec<u16>,
    raw: &mut Vec<u8>,
    bytes: &[u8],
    out: &mut Rgb,
) -> ImageResult<()> {
    let mut decoder = match tiff::decoder::Decoder::new_with_scratch(Cursor::new(bytes), std::mem::take(scratch)) {
        Ok(decoder) => decoder,
        Err((e, kept)) => {
            *scratch = kept;
            return Err(tiff_error(e));
        }
    };
    // The decoder is given its limits between the two steps (`with_limits` takes it and gives it back).
    let (decoder, decoded) = match plan(&mut decoder, formats) {
        Ok((plan, limits)) => {
            let mut decoder = decoder.with_limits(limits);
            let decoded = read(&mut decoder, plan, raw, out);
            (decoder, decoded)
        }
        Err(e) => (decoder, Err(e)),
    };
    *scratch = decoder.into_scratch();
    decoded
}

/// [`decode`]'s first step: the image's size and colour types from its tags, and the decoder's limits.
fn plan(decoder: &mut TiffDecoder<'_>, formats: &mut Vec<u16>) -> ImageResult<(TiffPlan, tiff::decoder::Limits)> {
    let (w, h) = decoder.dimensions().map_err(tiff_error)?;
    let tiff_color = decoder.colortype().map_err(tiff_error)?;
    // `find_tag_unsigned_vec`, into a kept vector; without the tag, unsigned integers.
    if decoder.find_tag_unsigned_vec_into(tiff::tags::Tag::SampleFormat, formats).map_err(tiff_error)? {
        formats.iter().try_for_each(|&f| check_sample_format(f, tiff_color))?;
    }
    let (color, stored) = decoded_color(tiff_color)?;
    let limits = tiff_limits(color, stored, w, h)?;
    let decoding_size = limits.decoding_buffer_size;
    Ok((TiffPlan { w, h, color, stored, decoding_size }, limits))
}

/// [`decode`]'s second step, once the decoder has its limits: the samples, into `out`.
fn read(decoder: &mut TiffDecoder<'_>, plan: TiffPlan, raw: &mut Vec<u8>, out: &mut Rgb) -> ImageResult<()> {
    use tiff::decoder::DecodingSampleType as Sample;

    let TiffPlan { w, h, color, stored, decoding_size } = plan;
    // `Decoder::read_image_to_buffer`: all planes if they fit the decoding limit, else the first (and `image`
    // then fails); the buffer in whole samples, within the limit.
    let layout = decoder.image_buffer_layout().map_err(tiff_error)?;
    let sample_size = match layout.sample_type {
        Some(Sample::U8 | Sample::I8) => 1,
        Some(Sample::U16 | Sample::I16 | Sample::F16) => 2,
        Some(Sample::U32 | Sample::I32 | Sample::F32) => 4,
        Some(_) => 8,
        // Sample formats and depths that `check_sample_format` and the colour types of `decoded_color` let through
        // all have a sample type.
        None => return Err(tiff_error(tiff::TiffError::LimitsExceeded)),
    };
    let plane_stride = layout.plane_stride.map_or(0, |n| n.get());
    let extent =
        if layout.complete_len <= decoding_size || layout.planes == 1 { layout.complete_len } else { plane_stride };
    let samples = extent.div_ceil(sample_size);
    if samples > decoding_size / sample_size {
        return Err(tiff_error(tiff::TiffError::LimitsExceeded));
    }
    let len = samples * sample_size;
    // Stored as the image is: read straight into `out`.
    let direct = layout.planes == 1
        && stored == Stored::Same
        && matches!(color, ColorType::Rgb8 | ColorType::Rgba8)
        && len == num::index(w) * num::index(h) * usize::from(color.bytes_per_pixel());
    let buf = if direct { zeroed(&mut out.pixels, len) } else { zeroed(raw, len) };
    decoder.read_image_bytes(buf).map_err(tiff_error)?;
    if len < layout.complete_len {
        return Err(ImageError::Limits(LimitError::from_kind(LimitErrorKind::InsufficientMemory)));
    }
    if direct {
        return finish(color, w, h, out, raw);
    }

    let pixels = num::index(w) * num::index(h);
    if layout.planes > 1 {
        // `TiffDecoder::interleave_planes`.
        if stored != Stored::Same {
            return Err(ImageError::Unsupported(UnsupportedError::from_format_and_kind(
                ImageFormat::Tiff.into(),
                UnsupportedErrorKind::GenericFeature("Planar TIFF with CMYK color type is not supported".to_string()),
            )));
        }
        // (The decoder refuses a zero width or height (tiff decoder `image.rs:325`), so an image has at least one
        // plane here.)
        if raw.len().checked_div(plane_stride).unwrap_or(0) < usize::from(color.channel_count()) {
            return Err(ImageError::Decoding(DecodingError::new(
                ImageFormat::Tiff.into(),
                "Not enough planes read from TIFF image".to_string(),
            )));
        }
        convert(raw, color, Some(plane_stride), pixels, out)?;
        (out.width, out.height) = (num::index(w), num::index(h));
        return Ok(());
    }
    match stored {
        Stored::Same => convert(raw, color, None, pixels, out)?,
        Stored::Cmyk8 => {
            // `image` writes each pixel with `write_all` (its tiff.rs:396): short data leaves the rest of the
            // pixels as they were, with no error.
            check_len(raw.len(), pixels * 4)?;
            let px = zeroed(&mut out.pixels, pixels * 3);
            for (cmyk, rgb) in raw.as_chunks::<4>().0.iter().zip(px.as_chunks_mut::<3>().0) {
                *rgb = cmyk_to_rgb(cmyk);
            }
        }
        Stored::Cmyk16 => {
            // As Cmyk8: `image`'s tiff.rs:402 `write_all` leaves the rest of the pixels as they were.
            check_len(raw.len(), pixels * 8)?;
            let px = zeroed(&mut out.pixels, pixels * 3);
            let samples = raw.as_chunks::<2>().0.as_chunks::<4>().0;
            for (cmyk, rgb) in samples.iter().zip(px.as_chunks_mut::<3>().0) {
                *rgb = cmyk_to_rgb16(&cmyk.map(u16::from_ne_bytes)).map(rgb16);
            }
        }
        Stored::Gray1 => {
            // `expand_bits(1, …)` per row (0 or 255), then L8 to RGB.
            let row_bytes = num::index(w).div_ceil(8);
            // `image`'s tiff.rs:410 `chunks_exact` drops a short last row: its rows past the data are left unwritten.
            check_len(raw.len(), row_bytes * num::index(h))?;
            let px = zeroed(&mut out.pixels, pixels * 3);
            for (row, out_row) in raw.chunks_exact(row_bytes.max(1)).zip(px.chunks_exact_mut(num::index(w) * 3)) {
                // Each byte holds eight pixels, the first in its high bit.
                for (&byte, rgbs) in row.iter().zip(out_row.as_chunks_mut::<3>().0.chunks_mut(8)) {
                    for (x, rgb) in rgbs.iter_mut().enumerate() {
                        let bit = byte >> (7 - x) & 1;
                        *rgb = [luma8([bit * 255]); 3];
                    }
                }
            }
        }
    }
    (out.width, out.height) = (num::index(w), num::index(h));
    Ok(())
}

/// The colour type `image` decodes a TIFF of colour type `color` to, and how its samples are stored: `image`'s
/// mapping, with its refusals of palette, YCbCr and the unknown types. Shared by [`plan`] and the TIFF crop
/// (`encode`).
pub(crate) fn decoded_color(color: tiff::ColorType) -> ImageResult<(ColorType, Stored)> {
    use tiff::ColorType as Tiff;

    Ok(match color {
        Tiff::Gray(1) => (ColorType::L8, Stored::Gray1),
        Tiff::Gray(8) => (ColorType::L8, Stored::Same),
        Tiff::Gray(16) => (ColorType::L16, Stored::Same),
        Tiff::GrayA(8) => (ColorType::La8, Stored::Same),
        Tiff::GrayA(16) => (ColorType::La16, Stored::Same),
        Tiff::RGB(8) => (ColorType::Rgb8, Stored::Same),
        Tiff::RGB(16) => (ColorType::Rgb16, Stored::Same),
        Tiff::RGBA(8) => (ColorType::Rgba8, Stored::Same),
        Tiff::RGBA(16) => (ColorType::Rgba16, Stored::Same),
        Tiff::CMYK(8) => (ColorType::Rgb8, Stored::Cmyk8),
        Tiff::CMYK(16) => (ColorType::Rgb16, Stored::Cmyk16),
        Tiff::RGB(32) => (ColorType::Rgb32F, Stored::Same),
        Tiff::RGBA(32) => (ColorType::Rgba32F, Stored::Same),
        Tiff::Palette(n) | Tiff::Gray(n) => return Err(tiff_unknown_color(n)),
        Tiff::GrayA(n) => return Err(tiff_unknown_color(n.saturating_mul(2))),
        Tiff::RGB(n) | Tiff::YCbCr(n) => return Err(tiff_unknown_color(n.saturating_mul(3))),
        Tiff::RGBA(n) | Tiff::CMYK(n) => return Err(tiff_unknown_color(n.saturating_mul(4))),
        Tiff::Multiband { bit_depth, num_samples } => {
            let samples = u8::try_from(num_samples.min(255)).unwrap_or(u8::MAX);
            return Err(tiff_unknown_color(bit_depth.saturating_mul(samples)));
        }
        _ => return Err(tiff_unknown_color(0)),
    })
}

/// The decoder's limits for a `w` × `h` TIFF decoded as `color` and stored as `stored`, the one construction used by
/// [`plan`] and the TIFF crop (`encode`), so both obey the same cap (`max_decode_bytes`). The decoded image is
/// reserved from it as `image` reserves it (`reserve`). `TiffDecoder::set_limits` then splits the rest: what the
/// stored samples take (CMYK's 4 or 8 bytes a pixel, more than the RGB image) for the decoding buffer, and the rest
/// of the allowance for the decoder's intermediate buffers.
pub(crate) fn tiff_limits(color: ColorType, stored: Stored, w: u32, h: u32) -> ImageResult<tiff::decoder::Limits> {
    let reserved = reserve(color, w, h)?;
    let max_alloc = reserved.max_alloc.unwrap_or(u64::MAX);
    let stored_bpp = match stored {
        Stored::Cmyk8 => 4,
        Stored::Cmyk16 => 8,
        _ => u64::from(color.bytes_per_pixel()),
    };
    let stored_bytes = (u64::from(w) * u64::from(h)).saturating_mul(stored_bpp);
    let intermediate = max_alloc.saturating_sub(stored_bytes);
    let mut limits = tiff::decoder::Limits::default();
    limits.decoding_buffer_size = usize::try_from(max_alloc - intermediate).unwrap_or(usize::MAX);
    limits.intermediate_buffer_size = usize::try_from(intermediate).unwrap_or(usize::MAX);
    limits.ifd_value_size = limits.intermediate_buffer_size;
    Ok(limits)
}

/// `image`'s `check_sample_format` (TIFF).
fn check_sample_format(sample_format: u16, color: tiff::ColorType) -> ImageResult<()> {
    use tiff::ColorType as Tiff;
    use tiff::tags::SampleFormat;
    let unsupported = |feature: String| {
        ImageError::Unsupported(UnsupportedError::from_format_and_kind(
            ImageFormat::Tiff.into(),
            UnsupportedErrorKind::GenericFeature(feature),
        ))
    };
    let bits = match color {
        Tiff::CMYK(k) | Tiff::Gray(k) | Tiff::RGB(k) | Tiff::RGBA(k) | Tiff::GrayA(k) => k,
        Tiff::Palette(k) | Tiff::YCbCr(k) => {
            return Err(unsupported(format!("Unhandled TIFF color type {color:?} for {k} bits")));
        }
        _ => return Err(unsupported(format!("Unhandled TIFF color type {color:?}"))),
    };
    match SampleFormat::from_u16(sample_format) {
        Some(SampleFormat::Uint) if bits <= 16 => Ok(()),
        Some(SampleFormat::IEEEFP) if bits == 32 => Ok(()),
        _ => Err(unsupported(format!("Unhandled TIFF sample format {sample_format:?} for {bits} bits"))),
    }
}

/// `image`'s `err_unknown_color_type` (TIFF).
fn tiff_unknown_color(value: u8) -> ImageError {
    ImageError::Unsupported(UnsupportedError::from_format_and_kind(
        ImageFormat::Tiff.into(),
        UnsupportedErrorKind::Color(ExtendedColorType::Unknown(value)),
    ))
}

/// `ImageError::from_tiff_decode`.
fn tiff_error(err: tiff::TiffError) -> ImageError {
    use tiff::TiffError;
    match err {
        TiffError::IoError(err) => ImageError::IoError(err),
        err @ (TiffError::FormatError(_) | TiffError::IntSizeError | TiffError::UsageError(_)) => {
            ImageError::Decoding(DecodingError::new(ImageFormat::Tiff.into(), err))
        }
        TiffError::UnsupportedError(desc) => ImageError::Unsupported(UnsupportedError::from_format_and_kind(
            ImageFormat::Tiff.into(),
            UnsupportedErrorKind::GenericFeature(desc.to_string()),
        )),
        TiffError::LimitsExceeded => ImageError::Limits(LimitError::from_kind(LimitErrorKind::InsufficientMemory)),
    }
}
