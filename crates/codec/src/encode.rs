//! Re-encoding: a crop of a format the lossless rewriters don't cut (TIFF and BMP), decoded, cropped and encoded
//! again in the same format.

use anyhow::{Context, Result, bail, ensure};
use image::ImageFormat;
use pg_model::{format::Format, num};
use std::io::Cursor;

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
        other => bail!("{other:?} isn't re-encoded: its crop is lossless"),
    };
    if format == ImageFormat::Tiff
        && let Some(out) = tiff_samples_cropped(bytes, [x, y, w, h])?
    {
        return Ok(out);
    }
    let img = image::load_from_memory_with_format(bytes, format).context("decoding")?;
    let mut out = Cursor::new(Vec::new());
    img.crop_imm(x, y, w, h).write_to(&mut out, format).context("encoding")?;
    Ok(out.into_inner())
}

/// A TIFF of 8- or 16-bit CMYK or RGB samples (one plane, unsigned), cropped to `rect` (left, top, width, height)
/// in its stored samples and written by the tiff crate's encoder in the file's own kind (classic, or BigTIFF): no
/// colour conversion (`image` would turn CMYK into RGB). `None` for any other TIFF.
fn tiff_samples_cropped(bytes: &[u8], rect: [u32; 4]) -> Result<Option<Vec<u8>>> {
    use tiff::ColorType as Tiff;
    use tiff::decoder::{Decoder, DecodingResult};
    use tiff::encoder::colortype::{CMYK8, CMYK16, RGB8, RGB16};

    let mut decoder = Decoder::new(Cursor::new(bytes)).context("decoding")?;
    let color = decoder.colortype().context("decoding")?;
    let channels = match color {
        Tiff::CMYK(8 | 16) => 4,
        Tiff::RGB(8 | 16) => 3,
        _ => return Ok(None),
    };
    let layout = decoder.image_buffer_layout().context("decoding")?;
    if layout.planes != 1 || !matches!(layout.sample_format, tiff::tags::SampleFormat::Uint) {
        return Ok(None);
    }
    let (width, _) = decoder.dimensions().context("decoding")?;
    // BigTIFF: 43 after the byte order, where a classic TIFF has 42.
    let big = matches!(bytes.get(..4), Some(b"II\x2B\0" | b"MM\0\x2B"));
    let [_, _, w, h] = rect;
    let mut out = Cursor::new(Vec::new());
    match (color, decoder.read_image().context("decoding")?) {
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
        out.extend_from_slice(row.get(x..x + w).context("the crop is outside the image")?);
    }
    ensure!(out.len() == w * h, "the crop is outside the image");
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
        tiff::encoder::TiffEncoder::new_big(out)?.write_image::<C>(w, h, data).context("encoding")?;
    } else {
        tiff::encoder::TiffEncoder::new(out)?.write_image::<C>(w, h, data).context("encoding")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiff::encoder::TiffEncoder;
    use tiff::encoder::colortype::{CMYK8, CMYK16};

    /// The stored samples of a TIFF, as the tiff crate reads them, with its colour type and size.
    fn samples(bytes: &[u8]) -> (tiff::ColorType, (u32, u32), tiff::decoder::DecodingResult) {
        let mut d = tiff::decoder::Decoder::new(Cursor::new(bytes)).unwrap();
        (d.colortype().unwrap(), d.dimensions().unwrap(), d.read_image().unwrap())
    }

    fn is_big(bytes: &[u8]) -> bool {
        matches!(bytes.get(..4), Some(b"II\x2B\0" | b"MM\0\x2B"))
    }

    /// A CMYK TIFF is cropped in its CMYK samples, at its depth and in its kind: no conversion to RGB, and a
    /// BigTIFF stays one.
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
            let mut file = Cursor::new(Vec::new());
            if big {
                TiffEncoder::new_big(&mut file).unwrap().write_image::<CMYK8>(w, h, &cmyk8).unwrap();
            } else {
                TiffEncoder::new(&mut file).unwrap().write_image::<CMYK8>(w, h, &cmyk8).unwrap();
            }
            let file = file.into_inner();
            let out = cropped(&file, Format::Tiff, rect[0], rect[1], rect[2], rect[3]).unwrap();
            assert_eq!(is_big(&out), big);
            let (color, size, got) = samples(&out);
            assert_eq!((color, size), (tiff::ColorType::CMYK(8), (3, 2)));
            let tiff::decoder::DecodingResult::U8(got) = got else { panic!("8-bit samples") };
            let all: Vec<u16> = cmyk8.iter().map(|&v| u16::from(v)).collect();
            assert_eq!(got.iter().map(|&v| u16::from(v)).collect::<Vec<_>>(), want(&all), "big {big}");

            let mut file = Cursor::new(Vec::new());
            if big {
                TiffEncoder::new_big(&mut file).unwrap().write_image::<CMYK16>(w, h, &cmyk16).unwrap();
            } else {
                TiffEncoder::new(&mut file).unwrap().write_image::<CMYK16>(w, h, &cmyk16).unwrap();
            }
            let out = cropped(&file.into_inner(), Format::Tiff, rect[0], rect[1], rect[2], rect[3]).unwrap();
            assert_eq!(is_big(&out), big);
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

    /// An RGB BigTIFF stays a BigTIFF, with its pixels.
    #[test]
    fn an_rgb_bigtiff_crops_as_a_bigtiff() {
        let (w, h) = (6u32, 4u32);
        let rgb: Vec<u8> = (0..w * h * 3).map(|i| u8::try_from(i * 7 % 256).unwrap()).collect();
        for big in [false, true] {
            let file = crate::fixtures::tiff_rgb8(w, h, &rgb, big).unwrap();
            let out = cropped(&file, Format::Tiff, 1, 1, 4, 2).unwrap();
            assert_eq!(is_big(&out), big);
            let (color, size, got) = samples(&out);
            assert_eq!((color, size), (tiff::ColorType::RGB(8), (4, 2)));
            let want: Vec<u8> = (1..3usize).flat_map(|y| rgb[(y * 6 + 1) * 3..(y * 6 + 5) * 3].to_vec()).collect();
            let tiff::decoder::DecodingResult::U8(got) = got else { panic!("8-bit samples") };
            assert_eq!(got, want);
        }
    }
}
