//! Encoding: decoded pixels as a JPEG ([`JpegEncoder`]: the thumbnails, and a photo re-encoded for a service), and a
//! crop of a format the lossless rewriters don't cut (TIFF and BMP), decoded, cropped and encoded again in the same
//! format ([`cropped`]). The cut of an original (`pg_edit::crop::cut`) re-encodes a JPEG exactly, at the quality its
//! tables say ([`jpeg_cropped`], [`jpeg_quality`]), and a lossy WebP lossy again ([`webp_lossy_cropped`]).

use crate::error::{Error, Result};
use image::ImageFormat;
use pg_model::{format::Format, num};
use std::io::Cursor;
use turbojpeg::{Colorspace, Compressor, Image, PixelFormat, Subsamp};

use crate::codecs::tiff::{decoded_color, tiff_limits};
use crate::pixels::{Rgb, release_over};

/// How a JPEG keeps the colour: at half resolution each way (4:2:0, a thumbnail's), or at full resolution (4:4:4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chroma {
    /// Half resolution each way (4:2:0).
    Half,
    /// Full resolution (4:4:4).
    Full,
}

/// libjpeg-turbo's compressor at one quality and chroma, and its output buffer, kept from one image to the next (a
/// thumbnail worker encodes every thumbnail with one).
pub struct JpegEncoder {
    compressor: Compressor,
    /// The compressor's output, at its worst-case size.
    buf: Vec<u8>,
}

impl JpegEncoder {
    /// An encoder at `quality` (libjpeg-turbo's 1–100) with `chroma`.
    pub fn new(quality: u8, chroma: Chroma) -> Result<JpegEncoder> {
        let mut compressor = Compressor::new().map_err(Error::encoder)?;
        compressor.set_quality(i32::from(quality)).map_err(Error::encoder)?;
        compressor
            .set_subsamp(match chroma {
                Chroma::Half => Subsamp::Sub2x2,
                Chroma::Full => Subsamp::None,
            })
            .map_err(Error::encoder)?;
        Ok(JpegEncoder { compressor, buf: Vec::new() })
    }

    /// `img` as a JPEG, into `out` (replacing what it holds), a buffer the caller reuses. The encoder's own buffer
    /// keeps its capacity for the next image ([`JpegEncoder::trim`]).
    pub fn encode_into(&mut self, img: &Rgb, out: &mut Vec<u8>) -> Result<()> {
        // Every byte libjpeg-turbo returns is written by it: a grown buffer only zeroes what it grew by.
        let len = self.compressor.buf_len(img.width, img.height).map_err(Error::encoder)?;
        self.buf.resize(len, 0);
        let n = self.compressor.compress_to_slice(image_of(img)?, &mut self.buf).map_err(Error::encoder)?;
        out.clear();
        out.extend_from_slice(
            self.buf.get(..n).ok_or_else(|| Error::invalid("libjpeg-turbo gave a size past its buffer"))?,
        );
        Ok(())
    }

    /// `img` as a JPEG, once: a buffer of its own, nothing kept.
    pub fn encode(&mut self, img: &Rgb) -> Result<Vec<u8>> {
        self.compressor.compress_to_vec(image_of(img)?).map_err(Error::encoder)
    }

    /// Free the output buffer when one large image grew it over `bytes` ([`release_over`]).
    pub fn trim(&mut self, bytes: usize) {
        release_over(&mut self.buf, bytes);
    }
}

/// `img` as libjpeg-turbo takes it: packed RGB rows.
fn image_of(img: &Rgb) -> Result<Image<&[u8]>> {
    let pitch = img.width.checked_mul(3).ok_or_else(|| Error::invalid("the image's row is longer than memory"))?;
    Ok(Image { pixels: img.pixels.as_slice(), width: img.width, pitch, height: img.height, format: PixelFormat::RGB })
}

/// `bytes` (a BMP or a TIFF) decoded, cropped to the `w` × `h` rectangle at (`x`, `y`) and encoded again in the same
/// format. The metadata is not carried over.
///
/// An 8- or 16-bit CMYK or RGB TIFF (one plane, unsigned samples) is cropped in its own samples and written in its
/// own kind, classic or BigTIFF (`tiff_samples_cropped`): a CMYK one stays CMYK. Any other TIFF goes through
/// `image`, which writes a classic TIFF of the colour type it decodes to (1-bit grey becomes 8-bit; palette and
/// YCbCr TIFFs are refused).
pub fn cropped(bytes: &[u8], format: Format, x: u32, y: u32, w: u32, h: u32) -> Result<Vec<u8>> {
    let format = match format {
        Format::Bmp => ImageFormat::Bmp,
        Format::Tiff => ImageFormat::Tiff,
        other => return Err(Error::invalid(format!("{other:?} isn't re-encoded: its crop is lossless"))),
    };
    if format == ImageFormat::Tiff
        && let Some(out) = tiff_samples_cropped(bytes, [x, y, w, h])?
    {
        return Ok(out);
    }
    let img =
        image::load_from_memory_with_format(bytes, format).map_err(|e| Error::decode("can't decode the image", e))?;
    let mut out = Cursor::new(Vec::new());
    img.crop_imm(x, y, w, h).write_to(&mut out, format).map_err(|e| Error::encode("can't encode the image", e))?;
    Ok(out.into_inner())
}

/// The quality a cut lossy WebP is encoded at, on libwebp's 0–100 scale. libwebp keeps no quantization tables to read
/// the source's quality from, so every cut lossy WebP is encoded at this one.
pub const WEBP_LOSSY_QUALITY: u8 = 90;

/// `bytes` (a still lossy WebP, RGB or with alpha) decoded, cropped to `rect` (left, top, width, height, stored pixels)
/// and encoded lossy again at `quality` (libwebp's 0–100), its alpha kept. The metadata is not carried over.
pub fn webp_lossy_cropped(bytes: &[u8], rect: [u32; 4], quality: u8) -> Result<Vec<u8>> {
    let image = webp::Decoder::new(bytes).decode().ok_or_else(|| Error::invalid("decoding"))?;
    let alpha = image.is_alpha();
    let pixels = rows(&image[..], image.width(), if alpha { 4 } else { 3 }, rect)?;
    let [_, _, w, h] = rect;
    let encoder = if alpha { webp::Encoder::from_rgba(&pixels, w, h) } else { webp::Encoder::from_rgb(&pixels, w, h) };
    Ok(encoder.encode(f32::from(quality)).to_vec())
}

/// `bytes` (a still JPEG in YCbCr or grey) decoded, cropped to `rect` (left, top, width, height, stored pixels) and
/// encoded again at `quality` (libjpeg-turbo's 1–100), with the chroma subsampling of the source. The crop is exact:
/// it is on no block grid, as a DCT crop's is. The metadata is not carried over. A JPEG in another colour space (CMYK,
/// YCCK, or RGB) is refused, since its re-encode would change its colours.
pub fn jpeg_cropped(bytes: &[u8], rect: [u32; 4], quality: u8) -> Result<Vec<u8>> {
    let header = turbojpeg::read_header(bytes).map_err(|e| Error::decode("decoding", e))?;
    if !matches!(header.colorspace, Colorspace::YCbCr | Colorspace::Gray) {
        return Err(Error::invalid(format!(
            "a JPEG in {:?} colour isn't cut: re-encoding it would change its colours",
            header.colorspace
        )));
    }
    let image = turbojpeg::decompress(bytes, PixelFormat::RGB).map_err(|e| Error::decode("decoding", e))?;
    let pixels = rows(&image.pixels, u32::try_from(image.width).map_err(Error::decoder)?, 3, rect)?;
    let [_, _, w, h] = rect;
    let mut compressor = Compressor::new().map_err(Error::encoder)?;
    compressor.set_quality(i32::from(quality)).map_err(Error::encoder)?;
    compressor.set_subsamp(header.subsamp).map_err(Error::encoder)?;
    let w = num::index(w);
    let image = Image { pixels: &pixels[..], width: w, pitch: w * 3, height: num::index(h), format: PixelFormat::RGB };
    compressor.compress_to_vec(image).map_err(Error::encoder)
}

/// The luminance quantization table of the IJG standard (ITU-T T.81, Annex K, Table K.1), in natural (row-major) order.
const IJG_LUMA: [i32; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51,
    87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, 49, 64, 78, 87, 103, 121, 120, 101,
    72, 92, 95, 98, 112, 100, 103, 99,
];

/// The natural (row-major) index of each entry of a JPEG quantization table, which the file stores in zigzag order.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21,
    28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54,
    47, 55, 62, 63,
];

/// The luminance table libjpeg writes at `quality` (1–100), in natural order: the IJG standard scaled by the IJG
/// quality formula and clamped to 1–255 (`jpeg_set_quality`, with baseline forced).
fn ijg_luma(quality: u8) -> [i32; 64] {
    let q = i32::from(quality.clamp(1, 100));
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    IJG_LUMA.map(|v| ((v * scale + 50) / 100).clamp(1, 255))
}

/// The quality (IJG's 1–100) the JPEG `bytes` was encoded at, read from its luminance quantization table: the quality
/// whose table is the file's, and for a table that no quality gives (another encoder's), the one closest by the sum of
/// the absolute differences. Equal tables quantize alike, so the lowest of them is given. `None` when the file has no
/// luminance table.
pub fn jpeg_quality(bytes: &[u8]) -> Option<u8> {
    let table = jpeg_luma_table(bytes)?;
    (1..=100u8).min_by_key(|&q| ijg_luma(q).iter().zip(&table).map(|(a, b)| (a - b).abs()).sum::<i32>())
}

/// The luminance table of the JPEG `bytes` (its first DQT table with id 0, before the scan), in natural order. `None` when
/// there is none, or the markers before the scan don't parse.
fn jpeg_luma_table(bytes: &[u8]) -> Option<[i32; 64]> {
    if bytes.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut at = 2;
    loop {
        // Fill bytes (0xFF) may come before a marker's code.
        while bytes.get(at)? == &0xFF && bytes.get(at + 1)? == &0xFF {
            at += 1;
        }
        if bytes.get(at)? != &0xFF {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        at += 2;
        match marker {
            // Standalone markers, with no length: a restart, or a temporary.
            0x01 | 0xD0..=0xD7 => continue,
            // The end of the image, or the scan: the tables are all before it.
            0xD9 | 0xDA => return None,
            _ => {}
        }
        let len = usize::from(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]));
        let segment = bytes.get(at + 2..at.checked_add(len)?)?;
        if marker == 0xDB
            && let Some(table) = dqt_luma(segment)
        {
            return Some(table);
        }
        at += len;
    }
}

/// The luminance table among a DQT segment's tables (8- or 16-bit, stored in zigzag order), in natural order.
fn dqt_luma(mut segment: &[u8]) -> Option<[i32; 64]> {
    while let Some((&spec, rest)) = segment.split_first() {
        let size = match spec >> 4 {
            0 => 1,
            1 => 2,
            _ => return None,
        };
        let body = rest.get(..64 * size)?;
        if spec & 0x0F == 0 {
            let mut table = [0; 64];
            for (&natural, entry) in ZIGZAG.iter().zip(body.chunks_exact(size)) {
                let value = match *entry {
                    [b] => i32::from(b),
                    [hi, lo] => i32::from(u16::from_be_bytes([hi, lo])),
                    _ => continue,
                };
                if let Some(slot) = table.get_mut(natural) {
                    *slot = value;
                }
            }
            return Some(table);
        }
        segment = rest.get(64 * size..)?;
    }
    None
}

/// A TIFF of 8- or 16-bit CMYK or RGB samples (one plane, unsigned), cropped to `rect` (left, top, width, height)
/// in its stored samples and written by the tiff crate's encoder in the file's own kind (classic, or BigTIFF): no
/// colour conversion (`image` would turn CMYK into RGB). `None` for any other TIFF.
fn tiff_samples_cropped(bytes: &[u8], rect: [u32; 4]) -> Result<Option<Vec<u8>>> {
    use tiff::ColorType as Tiff;
    use tiff::decoder::{Decoder, DecodingResult};
    use tiff::encoder::colortype::{CMYK8, CMYK16, RGB8, RGB16};

    let decoding = |e: tiff::TiffError| Error::decode("decoding", e);
    let mut decoder = Decoder::new(Cursor::new(bytes)).map_err(decoding)?;
    let color = decoder.colortype().map_err(decoding)?;
    let channels = match color {
        Tiff::CMYK(8 | 16) => 4,
        Tiff::RGB(8 | 16) => 3,
        _ => return Ok(None),
    };
    let layout = decoder.image_buffer_layout().map_err(decoding)?;
    if layout.planes != 1 || !matches!(layout.sample_format, tiff::tags::SampleFormat::Uint) {
        return Ok(None);
    }
    let (width, height) = decoder.dimensions().map_err(decoding)?;
    // The same limits as a decode of this file (`codecs::tiff_limits`): the decoded image reserved from the cap, and
    // the decoder's buffers from the rest.
    let image_decoding = |e: image::ImageError| Error::decode("decoding", e);
    let (image, stored) = decoded_color(color).map_err(image_decoding)?;
    let mut decoder = decoder.with_limits(tiff_limits(image, stored, width, height).map_err(image_decoding)?);
    let big = pg_model::format::is_bigtiff(bytes);
    let [_, _, w, h] = rect;
    let mut out = Cursor::new(Vec::new());
    match (color, decoder.read_image().map_err(decoding)?) {
        (Tiff::CMYK(8), DecodingResult::U8(s)) => {
            write::<CMYK8>(&mut out, big, w, h, &rows(&s, width, channels, rect)?)?;
        }
        (Tiff::CMYK(16), DecodingResult::U16(s)) => {
            write::<CMYK16>(&mut out, big, w, h, &rows(&s, width, channels, rect)?)?;
        }
        (Tiff::RGB(8), DecodingResult::U8(s)) => {
            write::<RGB8>(&mut out, big, w, h, &rows(&s, width, channels, rect)?)?;
        }
        (Tiff::RGB(16), DecodingResult::U16(s)) => {
            write::<RGB16>(&mut out, big, w, h, &rows(&s, width, channels, rect)?)?;
        }
        _ => return Ok(None),
    }
    Ok(Some(out.into_inner()))
}

/// The `rect` (left, top, width, height, in pixels) of an image `width` pixels wide whose pixels are `channels`
/// samples each, row after row.
fn rows<T: Copy>(samples: &[T], width: u32, channels: usize, [x, y, w, h]: [u32; 4]) -> Result<Vec<T>> {
    let stride = num::index(width) * channels;
    let (x, w) = (num::index(x) * channels, num::index(w) * channels);
    let (y, h) = (num::index(y), num::index(h));
    let mut out = Vec::with_capacity(w * h);
    // (`max`: an image no pixel wide has no rows.)
    for row in samples.chunks_exact(stride.max(1)).skip(y).take(h) {
        out.extend_from_slice(row.get(x..x + w).ok_or_else(|| Error::invalid("the crop is outside the image"))?);
    }
    if out.len() != w * h {
        return Err(Error::invalid("the crop is outside the image"));
    }
    Ok(out)
}

/// One `w` × `h` image of colour type `C`, as a classic TIFF or a BigTIFF (`big`).
fn write<C: tiff::encoder::colortype::ColorType>(
    out: &mut Cursor<Vec<u8>>,
    big: bool,
    w: u32,
    h: u32,
    data: &[C::Inner],
) -> Result<()>
where
    [C::Inner]: tiff::encoder::TiffValue,
{
    if big {
        let mut encoder = tiff::encoder::TiffEncoder::new_big(out).map_err(Error::encoder)?;
        encoder.write_image::<C>(w, h, data).map_err(|e| Error::encode("can't encode the image", e))?;
    } else {
        let mut encoder = tiff::encoder::TiffEncoder::new(out).map_err(Error::encoder)?;
        encoder.write_image::<C>(w, h, data).map_err(|e| Error::encode("can't encode the image", e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiff::encoder::TiffEncoder;
    use tiff::encoder::colortype::{CMYK8, CMYK16, RGB16};

    /// The stored samples of a TIFF, as the tiff crate reads them, with its colour type and size.
    fn samples(bytes: &[u8]) -> (tiff::ColorType, (u32, u32), tiff::decoder::DecodingResult) {
        let mut d = tiff::decoder::Decoder::new(Cursor::new(bytes)).unwrap();
        (d.colortype().unwrap(), d.dimensions().unwrap(), d.read_image().unwrap())
    }

    // A CMYK TIFF is cropped in its CMYK samples, at its depth and in its kind: no conversion to RGB, and a
    // BigTIFF stays one.
    #[test]
    fn a_cmyk_tiff_crops_as_cmyk_in_its_own_kind() {
        let (w, h) = (7u32, 5u32);
        let n = usize::try_from(w * h * 4).unwrap();
        let cmyk8: Vec<u8> = (0..n).map(|i| u8::try_from(i * 13 % 256).unwrap()).collect();
        let cmyk16: Vec<u16> = (0..n).map(|i| u16::try_from(i * 2741 % 65536).unwrap()).collect();
        // Left 2, top 1, 3 × 2: rows 1 and 2, pixels 2 to 4.
        let rect = [2, 1, 3, 2];
        let want = |all: &[u16]| -> Vec<u16> {
            (1..3usize).flat_map(|y| all[(y * 7 + 2) * 4..(y * 7 + 5) * 4].to_vec()).collect()
        };
        for big in [false, true] {
            let file = crate::fixtures::tiff_of::<CMYK8>(w, h, &cmyk8, big).unwrap();
            let out = cropped(&file, Format::Tiff, rect[0], rect[1], rect[2], rect[3]).unwrap();
            assert_eq!(pg_model::format::is_bigtiff(&out), big);
            let (color, size, got) = samples(&out);
            assert_eq!((color, size), (tiff::ColorType::CMYK(8), (3, 2)));
            let tiff::decoder::DecodingResult::U8(got) = got else { panic!("8-bit samples") };
            let all: Vec<u16> = cmyk8.iter().map(|&v| u16::from(v)).collect();
            assert_eq!(got.iter().map(|&v| u16::from(v)).collect::<Vec<_>>(), want(&all), "big {big}");

            let file = crate::fixtures::tiff_of::<CMYK16>(w, h, &cmyk16, big).unwrap();
            let out = cropped(&file, Format::Tiff, rect[0], rect[1], rect[2], rect[3]).unwrap();
            assert_eq!(pg_model::format::is_bigtiff(&out), big);
            let (color, size, got) = samples(&out);
            assert_eq!((color, size), (tiff::ColorType::CMYK(16), (3, 2)));
            let tiff::decoder::DecodingResult::U16(got) = got else { panic!("16-bit samples") };
            assert_eq!(got, want(&cmyk16), "big {big}");
        }
        // A crop outside the image is an error, not a short file.
        let mut file = Cursor::new(Vec::new());
        TiffEncoder::new(&mut file).unwrap().write_image::<CMYK8>(w, h, &cmyk8).unwrap();
        assert!(cropped(&file.into_inner(), Format::Tiff, 5, 0, 3, 1).is_err());
    }

    // An RGB BigTIFF stays a BigTIFF, with its pixels.
    #[test]
    fn an_rgb_bigtiff_crops_as_a_bigtiff() {
        let (w, h) = (6u32, 4u32);
        let rgb: Vec<u8> = (0..w * h * 3).map(|i| u8::try_from(i * 7 % 256).unwrap()).collect();
        for big in [false, true] {
            let file = crate::fixtures::tiff_rgb8(w, h, &rgb, big).unwrap();
            let out = cropped(&file, Format::Tiff, 1, 1, 4, 2).unwrap();
            assert_eq!(pg_model::format::is_bigtiff(&out), big);
            let (color, size, got) = samples(&out);
            assert_eq!((color, size), (tiff::ColorType::RGB(8), (4, 2)));
            let want: Vec<u8> = (1..3usize).flat_map(|y| rgb[(y * 6 + 1) * 3..(y * 6 + 5) * 3].to_vec()).collect();
            let tiff::decoder::DecodingResult::U8(got) = got else { panic!("8-bit samples") };
            assert_eq!(got, want);
        }
    }

    /// A gradient JPEG of 24 × 16 pixels at `quality`, with the chroma subsampling `subsamp`, from libjpeg-turbo.
    fn jpeg_at(quality: i32, subsamp: Subsamp) -> Vec<u8> {
        let (w, h) = (24usize, 16usize);
        let pixels: Vec<u8> = (0..w * h * 3).map(|i| u8::try_from(i * 7 % 256).unwrap()).collect();
        let mut c = Compressor::new().unwrap();
        c.set_quality(quality).unwrap();
        c.set_subsamp(subsamp).unwrap();
        c.compress_to_vec(Image { pixels: &pixels[..], width: w, pitch: w * 3, height: h, format: PixelFormat::RGB })
            .unwrap()
    }

    // The quality read from a libjpeg-turbo JPEG is the one it was encoded at, for every quality from 1 to 100.
    #[test]
    fn jpeg_quality_reads_the_quality_libjpeg_wrote() {
        for q in 1..=100 {
            assert_eq!(jpeg_quality(&jpeg_at(q, Subsamp::Sub2x2)), Some(u8::try_from(q).unwrap()), "quality {q}");
        }
    }

    // A JPEG cut exactly keeps its colour space and subsampling, and its size is the crop's, on no block grid.
    #[test]
    fn jpeg_cropped_is_exact_and_keeps_the_subsampling() {
        for subsamp in [Subsamp::Sub2x2, Subsamp::Sub2x1, Subsamp::None, Subsamp::Gray] {
            let file = jpeg_at(85, subsamp);
            // Left 5, top 3, 7 × 9 pixels: not on an 8 or 16 px grid.
            let out = jpeg_cropped(&file, [5, 3, 7, 9], 85).unwrap();
            let header = turbojpeg::read_header(&out).unwrap();
            assert_eq!((header.width, header.height), (7, 9), "{subsamp:?}");
            assert_eq!(header.subsamp, subsamp, "{subsamp:?}");
            assert_eq!(jpeg_quality(&out), Some(85), "{subsamp:?}");
        }
        assert!(jpeg_cropped(&jpeg_at(85, Subsamp::Sub2x2), [21, 0, 4, 1], 85).is_err(), "a crop outside the image");
    }

    // A file with no luminance table to read has no quality; a table that no quality gives still reads as the
    // nearest one.
    #[test]
    fn jpeg_quality_needs_a_luminance_table() {
        assert_eq!(jpeg_quality(&[0xFF, 0xD8, 0xFF, 0xD9]), None);
        assert_eq!(jpeg_quality(b"not a jpeg"), None);
        let file = jpeg_at(75, Subsamp::Sub2x2);
        assert_eq!(jpeg_quality(&file[..20]), None, "cut off before the tables");
        // The table at 75 with every entry doubled is the one at 50 (the IJG scale halves, then doubles back): no
        // quality gives it exactly for the rounding, so the nearest, about 50, is read.
        let mut custom = file.clone();
        let dqt = custom.windows(2).position(|w| w == [0xFF, 0xDB]).unwrap();
        // The first table's entries follow its 4-byte header and 1-byte id.
        for entry in &mut custom[dqt + 5..dqt + 5 + 64] {
            *entry = entry.saturating_mul(2);
        }
        let q = jpeg_quality(&custom).unwrap();
        assert!((45..=55).contains(&q), "{q}");
    }

    // A 16-bit RGB TIFF is cropped in its 16-bit samples, at its depth and in its kind, classic or BigTIFF.
    #[test]
    fn an_rgb16_tiff_crops_as_rgb16_in_its_own_kind() {
        let (w, h) = (6u32, 4u32);
        let rgb: Vec<u16> = (0..w * h * 3).map(|i| u16::try_from(i * 2741 % 65536).unwrap()).collect();
        // Left 1, top 1, 4 × 2: rows 1 and 2, pixels 1 to 4.
        for big in [false, true] {
            let file = crate::fixtures::tiff_of::<RGB16>(w, h, &rgb, big).unwrap();
            let out = cropped(&file, Format::Tiff, 1, 1, 4, 2).unwrap();
            assert_eq!(pg_model::format::is_bigtiff(&out), big);
            let (color, size, got) = samples(&out);
            assert_eq!((color, size), (tiff::ColorType::RGB(16), (4, 2)));
            let tiff::decoder::DecodingResult::U16(got) = got else { panic!("16-bit samples") };
            let want: Vec<u16> = (1..3usize).flat_map(|y| rgb[(y * 6 + 1) * 3..(y * 6 + 5) * 3].to_vec()).collect();
            assert_eq!(got, want, "big {big}");
        }
    }
}
