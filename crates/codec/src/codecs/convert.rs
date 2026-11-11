//! Decoded samples into the caller's 8-bit RGB, as `image` converts them (`into_rgb8`, or `into_rgba8` then
//! flattened onto white), and the buffers the decoders write into.

use image::error::{DecodingError, UnsupportedError, UnsupportedErrorKind};
use image::{ColorType, ImageDecoder, ImageError, ImageResult, Limits};

use crate::pixels::Rgb;
use pg_model::num;

/// Any of `image`'s decoders, the way `ImageReader::decode` and `decoder_to_image` use it (BMP's: `image` is
/// its format crate).
pub(super) fn read_with(mut decoder: impl ImageDecoder, raw: &mut Vec<u8>, out: &mut Rgb) -> ImageResult<()> {
    let ((w, h), color) = (decoder.dimensions(), decoder.color_type());
    let mut limits = Limits::default();
    limits.reserve(decoder.total_bytes())?;
    decoder.set_limits(limits)?;
    decoder.read_image(target(color, w, h, out, raw))?;
    finish(color, w, h, out, raw)
}

/// The buffer a decoder writes a `color` image into: `out`'s own when it is 8-bit RGB or RGBA, else `raw`. Zeroed,
/// as `decoder_to_vec`'s new buffer is, so that what a decoder leaves unwritten is zero whatever the buffer held.
pub(super) fn target<'a>(color: ColorType, w: u32, h: u32, out: &'a mut Rgb, raw: &'a mut Vec<u8>) -> &'a mut [u8] {
    let len = num::index(w) * num::index(h) * usize::from(color.bytes_per_pixel());
    match color {
        ColorType::Rgb8 | ColorType::Rgba8 => zeroed(&mut out.pixels, len),
        _ => zeroed(raw, len),
    }
}

/// `buf` resized to `len` bytes, all zero.
pub(super) fn zeroed(buf: &mut Vec<u8>, len: usize) -> &mut [u8] {
    buf.clear();
    buf.resize(len, 0);
    buf
}

/// The decoded image into 8-bit RGB in `out`: as it is, flattened in place, or converted from `raw`.
pub(super) fn finish(color: ColorType, w: u32, h: u32, out: &mut Rgb, raw: &[u8]) -> ImageResult<()> {
    let pixels = num::index(w) * num::index(h);
    match color {
        ColorType::Rgb8 => {}
        ColorType::Rgba8 => {
            // Each pixel is flattened in its own four bytes, then the alpha bytes are dropped, in place.
            let px = &mut out.pixels;
            check_len(px.len(), pixels * 4)?;
            for p in px.as_chunks_mut::<4>().0 {
                let [r, g, b, a] = *p;
                *p = [over_white(r, a), over_white(g, a), over_white(b, a), a];
            }
            let mut at = 0usize;
            px.retain(|_| {
                at += 1;
                !at.is_multiple_of(4)
            });
            px.truncate(pixels * 3);
        }
        _ => convert(raw, color, None, pixels, out)?,
    }
    (out.width, out.height) = (num::index(w), num::index(h));
    Ok(())
}

/// `pixels` pixels of `color` (native-endian samples, interleaved, or in planes `plane_stride` bytes apart) into
/// 8-bit RGB in `out.pixels`, as `image` converts them (`into_rgb8`, or `into_rgba8` then flattened onto white when
/// the colour type has alpha).
pub(super) fn convert(
    src: &[u8],
    color: ColorType,
    plane_stride: Option<usize>,
    pixels: usize,
    out: &mut Rgb,
) -> ImageResult<()> {
    let px = &mut out.pixels;
    match color {
        ColorType::L8 => to_rgb::<1, 1>(src, plane_stride, pixels, px, luma8),
        ColorType::La8 => to_rgb::<2, 1>(src, plane_stride, pixels, px, luma8),
        ColorType::Rgb8 => to_rgb::<3, 1>(src, plane_stride, pixels, px, |[v]| v),
        ColorType::Rgba8 => to_rgb::<4, 1>(src, plane_stride, pixels, px, |[v]| v),
        ColorType::L16 => to_rgb::<1, 2>(src, plane_stride, pixels, px, |s| luma16(u16::from_ne_bytes(s))),
        ColorType::La16 => to_rgb::<2, 2>(src, plane_stride, pixels, px, |s| luma16(u16::from_ne_bytes(s))),
        ColorType::Rgb16 => to_rgb::<3, 2>(src, plane_stride, pixels, px, |s| rgb16(u16::from_ne_bytes(s))),
        ColorType::Rgba16 => to_rgb::<4, 2>(src, plane_stride, pixels, px, |s| rgb16(u16::from_ne_bytes(s))),
        ColorType::Rgb32F => to_rgb::<3, 4>(src, plane_stride, pixels, px, |s| rgb_f32(f32::from_ne_bytes(s))),
        ColorType::Rgba32F => to_rgb::<4, 4>(src, plane_stride, pixels, px, |s| rgb_f32(f32::from_ne_bytes(s))),
        _ => Err(ImageError::Unsupported(UnsupportedError::from_format_and_kind(
            image::error::ImageFormatHint::Unknown,
            UnsupportedErrorKind::Color(color.into()),
        ))),
    }
}

/// [`convert`] for `CH` channels of `N`-byte samples, each made 8-bit by `to_u8`. One or two channels are gray
/// (and alpha), three or four RGB (and alpha); the last of two or four is alpha.
fn to_rgb<const CH: usize, const N: usize>(
    src: &[u8],
    plane_stride: Option<usize>,
    pixels: usize,
    px: &mut Vec<u8>,
    to_u8: impl Fn([u8; N]) -> u8,
) -> ImageResult<()> {
    match plane_stride {
        None => check_len(src.len(), pixels * CH * N)?,
        // The caller checked there are `CH` planes; checked again here, so each plane below is whole.
        Some(stride) => {
            check_len(stride, pixels * N)?;
            check_len(src.len(), stride * CH)?;
        }
    }
    let out = zeroed(px, pixels * 3).as_chunks_mut::<3>().0;
    match plane_stride {
        None => {
            for (rgb, samples) in out.iter_mut().zip(src.as_chunks::<N>().0.as_chunks::<CH>().0) {
                *rgb = flatten(samples.map(&to_u8));
            }
        }
        Some(stride) => {
            // The planes side by side, each a run of samples. (A zero stride has no pixels to read.)
            let mut chunks = src.chunks_exact(stride.max(1));
            let mut planes: [_; CH] =
                std::array::from_fn(|_| chunks.next().unwrap_or_default().as_chunks::<N>().0.iter());
            for rgb in out.iter_mut() {
                *rgb = flatten(planes.each_mut().map(|plane| plane.next().map_or(0, |&s| to_u8(s))));
            }
        }
    }
    Ok(())
}

/// A pixel's `CH` 8-bit channels into RGB: one or two are gray (and alpha), three or four RGB (and alpha); alpha is
/// flattened onto white.
fn flatten<const CH: usize>(v: [u8; CH]) -> [u8; 3] {
    match v.as_slice() {
        [l] => [*l; 3],
        [l, a] => [over_white(*l, *a); 3],
        [r, g, b] => [*r, *g, *b],
        [r, g, b, a] => [*r, *g, *b].map(|c| over_white(c, *a)),
        // `convert` gives one to four channels.
        _ => [0; 3],
    }
}

/// The decoders give exactly the size the image takes: anything else is a decoder's bug. `image` would panic where
/// the lengths differ at its `copy_from_slice` (tiff.rs:417–444), or leave pixels unwritten (tiff.rs:396, 402, 410).
/// The sites stay errors: a panic would end a worker.
pub(super) fn check_len(len: usize, expected: usize) -> ImageResult<()> {
    if len >= expected {
        return Ok(());
    }
    Err(ImageError::Decoding(DecodingError::new(
        image::error::ImageFormatHint::Unknown,
        format!("decoded {len} bytes, the image takes {expected}"),
    )))
}

/// A channel flattened onto white by its alpha, as the evaluation did (`eval/common.py::load_rgb`).
pub(super) fn over_white(v: u8, a: u8) -> u8 {
    let (v, a) = (u32::from(v), u32::from(a));
    u8::try_from((v * a + 255 * (255 - a) + 127) / 255).unwrap_or(u8::MAX)
}

/// An 8-bit gray sample into RGB: `image` converts gray to RGB through `f32` (`cast_pixels_by_fallback`:
/// `expand_to_f32`, then `clamp_from_f32`).
pub(super) fn luma8([v]: [u8; 1]) -> u8 {
    // 1 / u8::MAX, as a literal: `f32::from` is not `const`.
    const R: f32 = 1.0 / 255.0;
    clamp_u8(f32::from(v) * R)
}

/// [`luma8`] for 16-bit samples.
fn luma16(v: u16) -> u8 {
    // 1 / u16::MAX, as a literal: `f32::from` is not `const`.
    const R: f32 = 1.0 / 65535.0;
    clamp_u8(f32::from(v) * R)
}

/// `clamp_from_f32` for `u8`: [`f32_to_u8`] saturates.
fn clamp_u8(v: f32) -> u8 {
    f32_to_u8((v * f32::from(u8::MAX)).round())
}

/// A 16-bit RGB sample into 8 bits (`FromPrimitive<u16> for u8`): `round(v × 255 / 65535)`.
pub(super) fn rgb16(v: u16) -> u8 {
    u8::try_from((u32::from(v) + 128) / 257).unwrap_or(u8::MAX)
}

/// A float RGB sample into 8 bits (`FromPrimitive<f32> for u8`, `normalize_float`): clamped to [0, 1], NaN to 1.
#[expect(clippy::neg_cmp_op_on_partial_ord, reason = "NaN must take the first branch, which v < 1.0 alone would not")]
fn rgb_f32(v: f32) -> u8 {
    let clamped = if !(v < 1.0) { 1.0 } else { v.max(0.0) };
    f32_to_u8((clamped * f32::from(u8::MAX)).round())
}

/// `image`'s `cmyk_to_rgb` (TIFF).
pub(super) fn cmyk_to_rgb(cmyk: &[u8; 4]) -> [u8; 3] {
    let [c, m, y, k] = cmyk.map(f32::from);
    let kf = 1. - k / 255.;
    [f32_to_u8((255. - c) * kf), f32_to_u8((255. - m) * kf), f32_to_u8((255. - y) * kf)]
}

/// `image`'s `cmyk_to_rgb16` (TIFF).
pub(super) fn cmyk_to_rgb16(cmyk: &[u16; 4]) -> [u16; 3] {
    let [c, m, y, k] = cmyk.map(f32::from);
    let kf = 1. - k / 65535.;
    [f32_to_u16((65535. - c) * kf), f32_to_u16((65535. - m) * kf), f32_to_u16((65535. - y) * kf)]
}

/// `v` rounded toward zero, as `as` gives it: saturating at 0 and 255, NaN to 0.
fn f32_to_u8(v: f32) -> u8 {
    num::f64_to_u8(f64::from(v))
}

/// `v` rounded toward zero, as `as` gives it: saturating at 0 and 65535, NaN to 0.
fn f32_to_u16(v: f32) -> u16 {
    num::f64_to_u16(f64::from(v))
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, ImageBuffer};

    use super::*;
    use crate::codecs::test_support::{flattened, ne_bytes, noise};

    /// Each colour type's samples, every value of every channel (16-bit included), against `image`'s conversion.
    #[test]
    fn conversions_are_images() {
        let check = |raw: Vec<u8>, color: ColorType, img: DynamicImage| {
            let (w, h, want) = flattened(img);
            let mut out = Rgb::default();
            convert(&raw, color, None, w * h, &mut out).unwrap();
            assert!(out.pixels == want, "{color:?}");
        };
        // Every 8-bit pair (value, alpha); every 16-bit value, with alphas across their range.
        let pairs8: Vec<[u8; 2]> = (0..=255).flat_map(|a| (0..=255).map(move |v| [v, a])).collect();
        let v16: Vec<u16> = (0..=u16::MAX).collect();
        let pairs16: Vec<[u16; 2]> = v16.iter().map(|&v| [v, v.wrapping_mul(40503)]).collect();
        let n8 = u32::try_from(pairs8.len()).unwrap();
        let n16 = u32::try_from(v16.len()).unwrap();

        let l8: Vec<u8> = pairs8.iter().map(|p| p[0]).collect();
        check(l8.clone(), ColorType::L8, DynamicImage::ImageLuma8(ImageBuffer::from_raw(n8, 1, l8).unwrap()));
        let la8: Vec<u8> = pairs8.iter().flatten().copied().collect();
        check(la8.clone(), ColorType::La8, DynamicImage::ImageLumaA8(ImageBuffer::from_raw(n8, 1, la8).unwrap()));
        let rgba8: Vec<u8> = pairs8.iter().flat_map(|&[v, a]| [v, v ^ 0x5A, 255 - v, a]).collect();
        check(rgba8.clone(), ColorType::Rgba8, DynamicImage::ImageRgba8(ImageBuffer::from_raw(n8, 1, rgba8).unwrap()));
        let rgb8: Vec<u8> = pairs8.iter().flat_map(|&[v, a]| [v, a, v ^ a]).collect();
        check(rgb8.clone(), ColorType::Rgb8, DynamicImage::ImageRgb8(ImageBuffer::from_raw(n8, 1, rgb8).unwrap()));

        let l16 = DynamicImage::ImageLuma16(ImageBuffer::from_raw(n16, 1, v16.clone()).unwrap());
        check(ne_bytes(&v16, u16::to_ne_bytes), ColorType::L16, l16);
        let la16: Vec<u16> = pairs16.iter().flatten().copied().collect();
        let la16_bytes = ne_bytes(&la16, u16::to_ne_bytes);
        check(la16_bytes, ColorType::La16, DynamicImage::ImageLumaA16(ImageBuffer::from_raw(n16, 1, la16).unwrap()));
        let rgb16: Vec<u16> = v16.iter().flat_map(|&v| [v, !v, v.rotate_left(5)]).collect();
        let rgb16_bytes = ne_bytes(&rgb16, u16::to_ne_bytes);
        check(rgb16_bytes, ColorType::Rgb16, DynamicImage::ImageRgb16(ImageBuffer::from_raw(n16, 1, rgb16).unwrap()));
        let rgba16: Vec<u16> = pairs16.iter().flat_map(|&[v, a]| [v, !v, v.rotate_left(5), a]).collect();
        let rgba16_bytes = ne_bytes(&rgba16, u16::to_ne_bytes);
        let rgba16 = DynamicImage::ImageRgba16(ImageBuffer::from_raw(n16, 1, rgba16).unwrap());
        check(rgba16_bytes, ColorType::Rgba16, rgba16);

        // Floats: the edges (NaN, infinities, signed zeros, out of range, each rounding boundary and its
        // neighbours), then a million spread over [-0.5, 1.5].
        let mut floats = vec![f32::NAN, -f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -0.0, -1.0, 2.0, 1.0];
        floats.extend([f32::MIN_POSITIVE, -f32::MIN_POSITIVE, 1.0f32.next_up(), 1.0f32.next_down()]);
        for k in 0..=u8::MAX {
            let edge = (f32::from(k) + 0.5) / 255.0;
            floats.extend([edge.next_down(), edge, edge.next_up()]);
        }
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        floats.extend((0..1_000_000).map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            // A 24-bit value, as its high 16 and low 8 bits: each is exact in an f32, and so is their sum.
            let sample = state >> 40;
            let hi = f32::from(u16::try_from(sample >> 8).unwrap());
            let lo = f32::from(u8::try_from(sample & 0xFF).unwrap());
            let unit = (hi * 256.0 + lo) / 16_777_216.0;
            unit * 2.0 - 0.5
        }));
        // Whole pixels of both 3 and 4 channels.
        floats.truncate(floats.len() / 12 * 12);
        let n = u32::try_from(floats.len() / 12).unwrap();
        let float_bytes = ne_bytes(&floats, f32::to_ne_bytes);
        let rgb = DynamicImage::ImageRgb32F(ImageBuffer::from_raw(n * 4, 1, floats.clone()).unwrap());
        check(float_bytes.clone(), ColorType::Rgb32F, rgb);
        let rgba = DynamicImage::ImageRgba32F(ImageBuffer::from_raw(n * 3, 1, floats).unwrap());
        check(float_bytes, ColorType::Rgba32F, rgba);
    }

    /// Planes give what the same samples interleaved give.
    #[test]
    fn planes_convert_as_interleaved() {
        let pixels = 300;
        let chunky = noise(pixels * 4 * 2, 5);
        let mut planar = vec![0; chunky.len()];
        for i in 0..pixels {
            for c in 0..4 {
                planar[(c * pixels + i) * 2..][..2].copy_from_slice(&chunky[(i * 4 + c) * 2..][..2]);
            }
        }
        let (mut a, mut b) = (Rgb::default(), Rgb::default());
        convert(&chunky, ColorType::Rgba16, None, pixels, &mut a).unwrap();
        convert(&planar, ColorType::Rgba16, Some(pixels * 2), pixels, &mut b).unwrap();
        assert!(a.pixels == b.pixels);
    }
}
